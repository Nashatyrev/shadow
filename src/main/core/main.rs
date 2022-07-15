use std::borrow::Borrow;
use std::ffi::{CStr, OsStr};
use std::os::unix::ffi::OsStrExt;

use anyhow::{self, Context};
use clap::Parser;
use nix::sys::{personality, resource, signal};

use crate::core::controller::Controller;
use crate::core::logger::shadow_logger;
use crate::core::sim_config::SimConfig;
use crate::core::support::configuration::{CliOptions, ConfigFileOptions, ConfigOptions};
use crate::core::worker;
use crate::cshadow as c;
use crate::utility::shm_cleanup;

/// Main entry point for the simulator.
pub fn run_shadow<'a>(args: Vec<&'a OsStr>) -> anyhow::Result<()> {
    if unsafe { c::main_checkGlibVersion() } != 0 {
        return Err(anyhow::anyhow!("Unsupported GLib version"));
    }

    // unblock all signals in shadow and child processes since cmake's ctest blocks
    // SIGTERM (and maybe others)
    signal::sigprocmask(
        signal::SigmaskHow::SIG_SETMASK,
        Some(&signal::SigSet::empty()),
        None,
    )?;

    // parse the options from the command line
    let options = match CliOptions::try_parse_from(args.clone()) {
        Ok(x) => x,
        Err(e) => {
            if e.use_stderr() {
                eprint!("{}", e);
                std::process::exit(1);
            } else {
                print!("{}", e);
                std::process::exit(0);
            }
        }
    };

    if options.show_build_info {
        unsafe { c::main_printBuildInfo() };
        std::process::exit(0);
    }

    if options.shm_cleanup {
        // clean up any orphaned shared memory
        shm_cleanup::shm_cleanup(shm_cleanup::SHM_DIR_PATH).context("Cleaning shared memory files")?;
        std::process::exit(0);
    }

    // read from stdin if the config filename is given as '-'
    let config_filename: String = match options.config.as_ref().unwrap().as_str() {
        "-" => "/dev/stdin",
        x => x,
    }
    .into();

    // load the configuration yaml
    let file = std::fs::File::open(&config_filename)
        .with_context(|| format!("Could not open config file {:?}", &config_filename))?;
    let config_file: ConfigFileOptions = serde_yaml::from_reader(file)
        .with_context(|| format!("Could not parse configuration file {:?}", &config_filename))?;

    // generate the final shadow configuration from the config file and cli options
    let shadow_config = ConfigOptions::new(config_file, options.clone());

    if options.show_config {
        eprintln!("{:#?}", shadow_config);
        return Ok(());
    }

    // run any global C configuration handlers
    unsafe { c::runConfigHandlers(&shadow_config as *const ConfigOptions) };

    // configure other global state
    if shadow_config.experimental.use_object_counters.unwrap() {
        worker::enable_object_counters();
    }

    // start up the logging subsystem to handle all future messages
    shadow_logger::init().unwrap();
    // register the C logger
    unsafe { log_bindings::logger_setDefault(c::rustlogger_new()) };

    // disable log buffering during startup so that we see every message immediately in the terminal
    shadow_logger::set_buffering_enabled(false);

    // set the log level
    let log_level = shadow_config.general.log_level.unwrap();
    let log_level: log::Level = log_level.into();
    log::set_max_level(log_level.to_level_filter());

    // check if some log levels have been compiled out
    if log_level > log::STATIC_MAX_LEVEL {
        log::warn!(
            "Log level set to {}, but messages higher than {} have been compiled out",
            log_level,
            log::STATIC_MAX_LEVEL,
        );
    }

    // before we run the simulation, clean up any orphaned shared memory
    if let Err(e) = shm_cleanup::shm_cleanup(shm_cleanup::SHM_DIR_PATH) {
        log::warn!("Unable to clean up shared memory files: {:?}", e);
    }

    // save the platform data required for CPU pinning
    if shadow_config.experimental.use_cpu_pinning.unwrap() {
        if unsafe { c::affinity_initPlatformInfo() } != 0 {
            return Err(anyhow::anyhow!("Unable to initialize platform info"));
        }
    }

    // raise fd soft limit to hard limit
    raise_rlimit(resource::Resource::RLIMIT_NOFILE).context("Could not raise fd limit")?;

    // raise number of processes/threads soft limit to hard limit
    raise_rlimit(resource::Resource::RLIMIT_NPROC).context("Could not raise proc limit")?;

    if shadow_config.experimental.use_sched_fifo.unwrap() {
        set_sched_fifo().context("Could not set real-time scheduler mode to SCHED_FIFO")?;
        log::debug!("Successfully set real-time scheduler mode to SCHED_FIFO");
    }

    // Disable address space layout randomization of processes forked from this
    // one to improve determinism in cases when an executable under simulation
    // branch on memory addresses.
    match disable_aslr() {
        Ok(()) => log::debug!("ASLR disabled for processes forked from this parent process"),
        Err(e) => log::warn!("Could not disable address space layout randomization. This may affect determinism: {:?}", e),
    };

    // check sidechannel mitigations
    if unsafe { c::main_sidechannelMitigationsEnabled() } {
        log::warn!(
            "Speculative Store Bypass sidechannel mitigation is enabled (perhaps by seccomp?). \
             This typically adds ~30% performance overhead."
        );
    }

    // log some information
    unsafe { c::main_logBuildInfo() };
    log_environment(args.clone());

    log::debug!("Startup checks passed, we are ready to start the simulation");

    // allow gdb to attach before starting the simulation
    if options.gdb {
        pause_for_gdb_attach().context("Could not pause shadow to allow gdb to attach")?;
    }

    let sim_config = SimConfig::new(&shadow_config, &options.debug_hosts.unwrap_or_default())
        .context("Failed to initialize the simulation")?;

    // allocate and initialize our main simulation driver
    let controller = Controller::new(sim_config, &shadow_config);

    // enable log buffering if not at trace level
    let buffer_log = log::max_level() < log::LevelFilter::Trace;
    shadow_logger::set_buffering_enabled(buffer_log);
    if buffer_log {
        log::info!("Log message buffering is enabled for efficiency");
    }

    // run the simulation
    controller.run().context("Failed to run the simulation")?;

    // disable log buffering
    shadow_logger::set_buffering_enabled(false);
    if buffer_log {
        // only show if we disabled buffering above
        log::info!("Log message buffering is disabled during cleanup");
    }

    Ok(())
}

fn pause_for_gdb_attach() -> anyhow::Result<()> {
    let pid = nix::unistd::getpid();
    log::info!(
        "Pausing with SIGTSTP to enable debugger attachment (pid {})",
        pid
    );
    eprintln!(
        "** Pausing with SIGTSTP to enable debugger attachment (pid {})",
        pid
    );

    signal::raise(signal::Signal::SIGTSTP)?;

    log::info!("Resuming now");
    Ok(())
}

fn set_sched_fifo() -> anyhow::Result<()> {
    let mut param: libc::sched_param = unsafe { std::mem::zeroed() };
    param.sched_priority = 1;

    if unsafe { libc::sched_setscheduler(0, libc::SCHED_FIFO, &param as *const _) } != 0 {
        return Err(anyhow::anyhow!(
            "Could not set kernel SCHED_FIFO: {}",
            nix::errno::Errno::from_i32(nix::errno::errno()),
        ));
    }

    Ok(())
}

fn raise_rlimit(resource: resource::Resource) -> anyhow::Result<()> {
    let (_soft_limit, hard_limit) = resource::getrlimit(resource)?;
    resource::setrlimit(resource, hard_limit, hard_limit)?;
    Ok(())
}

fn disable_aslr() -> anyhow::Result<()> {
    let pers = personality::get()?;
    personality::set(pers | personality::Persona::ADDR_NO_RANDOMIZE)?;
    Ok(())
}

fn log_environment<'a>(args: Vec<&'a OsStr>) {
    for arg in args {
        log::info!("arg: {}", arg.to_string_lossy());
    }

    for (key, value) in std::env::vars_os() {
        let level = match key.to_string_lossy().borrow() {
            "LD_PRELOAD" | "SHADOW_SPAWNED" | "LD_STATIC_TLS_EXTRA" | "G_DEBUG" | "G_SLICE" => {
                log::Level::Info
            }
            _ => log::Level::Trace,
        };
        log::log!(level, "env: {:?}={:?}", key, value);
    }
}

mod export {
    use super::*;

    #[no_mangle]
    pub extern "C" fn main_runShadow(
        argc: libc::c_int,
        argv: *const *const libc::c_char,
    ) -> libc::c_int {
        let args = (0..argc).map(|x| unsafe { CStr::from_ptr(*argv.add(x as usize)) });
        let args = args.map(|x| OsStr::from_bytes(x.to_bytes()));

        let result = run_shadow(args.collect());
        log::logger().flush();

        if let Err(e) = result {
            // log the full error, its context, and its backtrace if enabled
            if log::log_enabled!(log::Level::Error) {
                for line in format!("{:?}", e).split('\n') {
                    log::error!("{}", line);
                }
                log::logger().flush();

                // print the short error
                eprintln!("** Shadow did not complete successfully: {}", e);
                eprintln!("**   {}", e.root_cause());
                eprintln!("** See the log for details");
            } else {
                // logging may not be configured yet, so print to stderr
                eprintln!("{:?}", e);
            }

            return 1;
        }

        eprintln!("** Shadow completed successfully");
        0
    }
}
