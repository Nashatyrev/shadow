use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::RwLock;

use linux_api::errno::Errno;
use linux_api::syscall::SyscallNum;
use shadow_shim_helper_rs::syscall_types::SysCallArgs;
use shadow_shim_helper_rs::syscall_types::SysCallReg;

use crate::cshadow as c;
use crate::host::context::{ThreadContext, ThreadContextObjs};
use crate::host::descriptor::descriptor_table::{DescriptorHandle, DescriptorTable};
use crate::host::descriptor::Descriptor;
use crate::host::syscall::formatter::log_syscall_simple;
use crate::host::syscall_types::SyscallReturn;
use crate::host::syscall_types::{SyscallError, SyscallResult};

mod clone;
mod epoll;
mod eventfd;
mod fcntl;
mod file;
mod fileat;
mod futex;
mod ioctl;
mod mman;
mod poll;
mod prctl;
mod random;
mod resource;
mod sched;
mod select;
mod shadow;
mod signal;
mod socket;
mod sysinfo;
mod time;
mod timerfd;
mod uio;
mod unistd;
mod wait;

type LegacySyscallFn =
    unsafe extern "C-unwind" fn(*mut c::SysCallHandler, *const SysCallArgs) -> SyscallReturn;

pub struct SyscallHandler {
    // Will eventually contain syscall handler state once migrated from the c handler
}

impl SyscallHandler {
    #[allow(clippy::new_without_default)]
    pub fn new() -> SyscallHandler {
        SyscallHandler {}
    }

    #[allow(non_upper_case_globals)]
    pub fn syscall(&self, ctx: &mut ThreadContext, args: &SysCallArgs) -> SyscallResult {
        let mut ctx = SyscallContext { objs: ctx, args };

        const NR_shadow_yield: SyscallNum = SyscallNum::new(c::ShadowSyscallNum_SYS_shadow_yield);
        const NR_shadow_init_memory_manager: SyscallNum =
            SyscallNum::new(c::ShadowSyscallNum_SYS_shadow_init_memory_manager);
        const NR_shadow_hostname_to_addr_ipv4: SyscallNum =
            SyscallNum::new(c::ShadowSyscallNum_SYS_shadow_hostname_to_addr_ipv4);

        let syscall = SyscallNum::new(ctx.args.number.try_into().unwrap());
        let syscall_name = syscall.to_str().unwrap_or("unknown-syscall");

        let was_blocked =
            unsafe { c::_syscallhandler_wasBlocked(ctx.objs.thread.csyscallhandler()) };

        log::trace!(
            "SYSCALL_HANDLER_PRE: {} ({}){} — ({}, tid={})",
            syscall_name,
            ctx.args.number,
            if was_blocked {
                " (previously BLOCKed)"
            } else {
                ""
            },
            &*ctx.objs.process.name(),
            ctx.objs.thread.id(),
        );

        // Count the frequency of each syscall, but only on the initial call. This avoids double
        // counting in the case where the initial call blocked at first, but then later became
        // unblocked and is now being handled again here.
        let syscall_counter =
            unsafe { c::_syscallhandler_getCounter(ctx.objs.thread.csyscallhandler()) };
        if let Some(syscall_counter) = unsafe { syscall_counter.as_mut() } {
            if !was_blocked {
                syscall_counter.add_one(syscall_name);
            }
        }

        macro_rules! handle {
            ($f:ident) => {{
                SyscallHandlerFn::call(Self::$f, &mut ctx)
            }};
        }

        let rv = match syscall {
            // SHADOW-HANDLED SYSCALLS
            //
            SyscallNum::NR_accept => handle!(accept),
            SyscallNum::NR_accept4 => handle!(accept4),
            SyscallNum::NR_bind => handle!(bind),
            SyscallNum::NR_brk => handle!(brk),
            SyscallNum::NR_clock_getres => handle!(clock_getres),
            SyscallNum::NR_clock_nanosleep => handle!(clock_nanosleep),
            SyscallNum::NR_clone => handle!(clone),
            SyscallNum::NR_clone3 => handle!(clone3),
            SyscallNum::NR_close => handle!(close),
            SyscallNum::NR_connect => handle!(connect),
            SyscallNum::NR_creat => handle!(creat),
            SyscallNum::NR_dup => handle!(dup),
            SyscallNum::NR_dup2 => handle!(dup2),
            SyscallNum::NR_dup3 => handle!(dup3),
            SyscallNum::NR_epoll_create => handle!(epoll_create),
            SyscallNum::NR_epoll_create1 => handle!(epoll_create1),
            SyscallNum::NR_epoll_ctl => handle!(epoll_ctl),
            SyscallNum::NR_epoll_pwait => handle!(epoll_pwait),
            SyscallNum::NR_epoll_pwait2 => handle!(epoll_pwait2),
            SyscallNum::NR_epoll_wait => handle!(epoll_wait),
            SyscallNum::NR_eventfd => handle!(eventfd),
            SyscallNum::NR_eventfd2 => handle!(eventfd2),
            SyscallNum::NR_execve => handle!(execve),
            SyscallNum::NR_execveat => handle!(execveat),
            SyscallNum::NR_exit_group => handle!(exit_group),
            SyscallNum::NR_faccessat => handle!(faccessat),
            SyscallNum::NR_fadvise64 => handle!(fadvise64),
            SyscallNum::NR_fallocate => handle!(fallocate),
            SyscallNum::NR_fchmod => handle!(fchmod),
            SyscallNum::NR_fchmodat => handle!(fchmodat),
            SyscallNum::NR_fchown => handle!(fchown),
            SyscallNum::NR_fchownat => handle!(fchownat),
            SyscallNum::NR_fcntl => handle!(fcntl),
            SyscallNum::NR_fdatasync => handle!(fdatasync),
            SyscallNum::NR_fgetxattr => handle!(fgetxattr),
            SyscallNum::NR_flistxattr => handle!(flistxattr),
            SyscallNum::NR_flock => handle!(flock),
            SyscallNum::NR_fork => handle!(fork),
            SyscallNum::NR_fremovexattr => handle!(fremovexattr),
            SyscallNum::NR_fsetxattr => handle!(fsetxattr),
            SyscallNum::NR_fstat => handle!(fstat),
            SyscallNum::NR_fstatfs => handle!(fstatfs),
            SyscallNum::NR_fsync => handle!(fsync),
            SyscallNum::NR_ftruncate => handle!(ftruncate),
            SyscallNum::NR_futex => handle!(futex),
            SyscallNum::NR_futimesat => handle!(futimesat),
            SyscallNum::NR_get_robust_list => handle!(get_robust_list),
            SyscallNum::NR_getdents => handle!(getdents),
            SyscallNum::NR_getdents64 => handle!(getdents64),
            SyscallNum::NR_getitimer => handle!(getitimer),
            SyscallNum::NR_getpeername => handle!(getpeername),
            SyscallNum::NR_getpgid => handle!(getpgid),
            SyscallNum::NR_getpgrp => handle!(getpgrp),
            SyscallNum::NR_getpid => handle!(getpid),
            SyscallNum::NR_getppid => handle!(getppid),
            SyscallNum::NR_getrandom => handle!(getrandom),
            SyscallNum::NR_getsid => handle!(getsid),
            SyscallNum::NR_getsockname => handle!(getsockname),
            SyscallNum::NR_getsockopt => handle!(getsockopt),
            SyscallNum::NR_gettid => handle!(gettid),
            SyscallNum::NR_ioctl => handle!(ioctl),
            SyscallNum::NR_kill => handle!(kill),
            SyscallNum::NR_linkat => handle!(linkat),
            SyscallNum::NR_listen => handle!(listen),
            SyscallNum::NR_lseek => handle!(lseek),
            SyscallNum::NR_mkdirat => handle!(mkdirat),
            SyscallNum::NR_mknodat => handle!(mknodat),
            SyscallNum::NR_mmap => handle!(mmap),
            SyscallNum::NR_mprotect => handle!(mprotect),
            SyscallNum::NR_mremap => handle!(mremap),
            SyscallNum::NR_munmap => handle!(munmap),
            SyscallNum::NR_nanosleep => handle!(nanosleep),
            SyscallNum::NR_newfstatat => handle!(newfstatat),
            SyscallNum::NR_open => handle!(open),
            SyscallNum::NR_openat => handle!(openat),
            SyscallNum::NR_pipe => handle!(pipe),
            SyscallNum::NR_pipe2 => handle!(pipe2),
            SyscallNum::NR_poll => handle!(poll),
            SyscallNum::NR_ppoll => handle!(ppoll),
            SyscallNum::NR_prctl => handle!(prctl),
            SyscallNum::NR_pread64 => handle!(pread64),
            SyscallNum::NR_preadv => handle!(preadv),
            SyscallNum::NR_preadv2 => handle!(preadv2),
            SyscallNum::NR_prlimit64 => handle!(prlimit64),
            SyscallNum::NR_pselect6 => handle!(pselect6),
            SyscallNum::NR_pwrite64 => handle!(pwrite64),
            SyscallNum::NR_pwritev => handle!(pwritev),
            SyscallNum::NR_pwritev2 => handle!(pwritev2),
            SyscallNum::NR_read => handle!(read),
            SyscallNum::NR_readahead => handle!(readahead),
            SyscallNum::NR_readlinkat => handle!(readlinkat),
            SyscallNum::NR_readv => handle!(readv),
            SyscallNum::NR_recvfrom => handle!(recvfrom),
            SyscallNum::NR_recvmsg => handle!(recvmsg),
            SyscallNum::NR_renameat => handle!(renameat),
            SyscallNum::NR_renameat2 => handle!(renameat2),
            SyscallNum::NR_rseq => handle!(rseq),
            SyscallNum::NR_rt_sigaction => handle!(rt_sigaction),
            SyscallNum::NR_rt_sigprocmask => handle!(rt_sigprocmask),
            SyscallNum::NR_sched_getaffinity => handle!(sched_getaffinity),
            SyscallNum::NR_sched_setaffinity => handle!(sched_setaffinity),
            SyscallNum::NR_select => handle!(select),
            SyscallNum::NR_sendmsg => handle!(sendmsg),
            SyscallNum::NR_sendto => handle!(sendto),
            SyscallNum::NR_set_robust_list => handle!(set_robust_list),
            SyscallNum::NR_set_tid_address => handle!(set_tid_address),
            SyscallNum::NR_setitimer => handle!(setitimer),
            SyscallNum::NR_setpgid => handle!(setpgid),
            SyscallNum::NR_setsid => handle!(setsid),
            SyscallNum::NR_setsockopt => handle!(setsockopt),
            SyscallNum::NR_shutdown => handle!(shutdown),
            SyscallNum::NR_sigaltstack => handle!(sigaltstack),
            SyscallNum::NR_socket => handle!(socket),
            SyscallNum::NR_socketpair => handle!(socketpair),
            SyscallNum::NR_statx => handle!(statx),
            SyscallNum::NR_symlinkat => handle!(symlinkat),
            SyscallNum::NR_sync_file_range => handle!(sync_file_range),
            SyscallNum::NR_syncfs => handle!(syncfs),
            SyscallNum::NR_sysinfo => handle!(sysinfo),
            SyscallNum::NR_tgkill => handle!(tgkill),
            SyscallNum::NR_timerfd_create => handle!(timerfd_create),
            SyscallNum::NR_timerfd_gettime => handle!(timerfd_gettime),
            SyscallNum::NR_timerfd_settime => handle!(timerfd_settime),
            SyscallNum::NR_tkill => handle!(tkill),
            SyscallNum::NR_uname => handle!(uname),
            SyscallNum::NR_unlinkat => handle!(unlinkat),
            SyscallNum::NR_utimensat => handle!(utimensat),
            SyscallNum::NR_vfork => handle!(vfork),
            SyscallNum::NR_waitid => handle!(waitid),
            SyscallNum::NR_wait4 => handle!(wait4),
            SyscallNum::NR_write => handle!(write),
            SyscallNum::NR_writev => handle!(writev),
            //
            // CUSTOM SHADOW-SPECIFIC SYSCALLS
            //
            NR_shadow_hostname_to_addr_ipv4 => handle!(shadow_hostname_to_addr_ipv4),
            NR_shadow_init_memory_manager => handle!(shadow_init_memory_manager),
            NR_shadow_yield => handle!(shadow_yield),
            //
            // SHIM-ONLY SYSCALLS
            //
            SyscallNum::NR_clock_gettime
            | SyscallNum::NR_gettimeofday
            | SyscallNum::NR_sched_yield
            | SyscallNum::NR_time => {
                panic!(
                    "Syscall {} ({}) should have been handled in the shim",
                    syscall_name, ctx.args.number,
                )
            }
            //
            // NATIVE LINUX-HANDLED SYSCALLS
            //
            SyscallNum::NR_access
            | SyscallNum::NR_arch_prctl
            | SyscallNum::NR_chmod
            | SyscallNum::NR_chown
            | SyscallNum::NR_exit
            | SyscallNum::NR_getcwd
            | SyscallNum::NR_geteuid
            | SyscallNum::NR_getegid
            | SyscallNum::NR_getgid
            | SyscallNum::NR_getgroups
            | SyscallNum::NR_getresgid
            | SyscallNum::NR_getresuid
            | SyscallNum::NR_getrlimit
            | SyscallNum::NR_getuid
            | SyscallNum::NR_getxattr
            | SyscallNum::NR_lchown
            | SyscallNum::NR_lgetxattr
            | SyscallNum::NR_link
            | SyscallNum::NR_listxattr
            | SyscallNum::NR_llistxattr
            | SyscallNum::NR_lremovexattr
            | SyscallNum::NR_lsetxattr
            | SyscallNum::NR_lstat
            | SyscallNum::NR_madvise
            | SyscallNum::NR_mkdir
            | SyscallNum::NR_mknod
            | SyscallNum::NR_readlink
            | SyscallNum::NR_removexattr
            | SyscallNum::NR_rename
            | SyscallNum::NR_rmdir
            | SyscallNum::NR_rt_sigreturn
            | SyscallNum::NR_setfsgid
            | SyscallNum::NR_setfsuid
            | SyscallNum::NR_setgid
            | SyscallNum::NR_setregid
            | SyscallNum::NR_setresgid
            | SyscallNum::NR_setresuid
            | SyscallNum::NR_setreuid
            | SyscallNum::NR_setrlimit
            | SyscallNum::NR_setuid
            | SyscallNum::NR_setxattr
            | SyscallNum::NR_stat
            | SyscallNum::NR_statfs
            | SyscallNum::NR_symlink
            | SyscallNum::NR_truncate
            | SyscallNum::NR_unlink
            | SyscallNum::NR_utime
            | SyscallNum::NR_utimes => {
                log::trace!("Native syscall {} ({})", syscall_name, ctx.args.number);

                let rv = Err(SyscallError::Native);

                log_syscall_simple(
                    ctx.objs.process,
                    ctx.objs.process.strace_logging_options(),
                    ctx.objs.thread.id(),
                    syscall_name,
                    "...",
                    &rv,
                )
                .unwrap();

                rv
            }
            //
            // UNSUPPORTED SYSCALL
            //
            _ => {
                // only show a warning the first time we encounter this unsupported syscall
                static WARNED_SET: RwLock<Option<HashSet<SyscallNum>>> = RwLock::new(None);

                let has_already_warned = WARNED_SET
                    .read()
                    .unwrap()
                    .as_ref()
                    .map(|x| x.contains(&syscall))
                    .unwrap_or(false);

                if !has_already_warned {
                    // `insert()` returns `false` if the syscall num was already in the set
                    assert!(WARNED_SET
                        .write()
                        .unwrap()
                        .get_or_insert_with(HashSet::new)
                        .insert(syscall));
                }

                let level = if has_already_warned {
                    log::Level::Debug
                } else {
                    log::Level::Warn
                };

                // we can't use the `warn_once_then_debug` macro here since we want to log this for
                // each unique syscall encountered, not only the first unsupported syscall
                // encountered
                log::log!(
                    level,
                    "(LOG_ONCE) Detected unsupported syscall {} ({}) called from thread {} in process {} on host {}",
                    syscall_name,
                    ctx.args.number,
                    ctx.objs.thread.id(),
                    &*ctx.objs.process.plugin_name(),
                    ctx.objs.host.name(),
                );

                let rv = Err(Errno::ENOSYS.into());

                let (syscall_name, syscall_args) = match syscall.to_str() {
                    // log it in the form "poll(...)"
                    Some(syscall_name) => (syscall_name, Cow::Borrowed("...")),
                    // log it in the form "syscall(X, ...)"
                    None => ("syscall", Cow::Owned(format!("{}, ...", ctx.args.number))),
                };

                log_syscall_simple(
                    ctx.objs.process,
                    ctx.objs.process.strace_logging_options(),
                    ctx.objs.thread.id(),
                    syscall_name,
                    &syscall_args,
                    &rv,
                )
                .unwrap();

                rv
            }
        };

        if log::log_enabled!(log::Level::Trace) {
            let rv_formatted = match &rv {
                Ok(reg) => format!("{}", i64::from(*reg)),
                Err(SyscallError::Failed(failed)) => {
                    let errno = failed.errno;
                    format!("{} ({errno})", errno.to_negated_i64())
                }
                Err(SyscallError::Native) => "<native>".to_string(),
                Err(SyscallError::Blocked(_)) => "<blocked>".to_string(),
            };

            log::trace!(
                "SYSCALL_HANDLER_POST: {} ({}) result {}{} — ({}, tid={})",
                syscall_name,
                ctx.args.number,
                if was_blocked { "BLOCK -> " } else { "" },
                rv_formatted,
                &*ctx.objs.process.name(),
                ctx.objs.thread.id(),
            );
        }

        rv
    }

    /// Internal helper that returns the `Descriptor` for the fd if it exists, otherwise returns
    /// EBADF.
    fn get_descriptor(
        descriptor_table: &DescriptorTable,
        fd: impl TryInto<DescriptorHandle>,
    ) -> Result<&Descriptor, linux_api::errno::Errno> {
        // check that fd is within bounds
        let fd = fd.try_into().or(Err(linux_api::errno::Errno::EBADF))?;

        match descriptor_table.get(fd) {
            Some(desc) => Ok(desc),
            None => Err(linux_api::errno::Errno::EBADF),
        }
    }

    /// Internal helper that returns the `Descriptor` for the fd if it exists, otherwise returns
    /// EBADF.
    fn get_descriptor_mut(
        descriptor_table: &mut DescriptorTable,
        fd: impl TryInto<DescriptorHandle>,
    ) -> Result<&mut Descriptor, linux_api::errno::Errno> {
        // check that fd is within bounds
        let fd = fd.try_into().or(Err(linux_api::errno::Errno::EBADF))?;

        match descriptor_table.get_mut(fd) {
            Some(desc) => Ok(desc),
            None => Err(linux_api::errno::Errno::EBADF),
        }
    }

    /// Run a legacy C syscall handler.
    fn legacy_syscall(syscall: LegacySyscallFn, ctx: &mut SyscallContext) -> SyscallResult {
        unsafe { syscall(ctx.objs.thread.csyscallhandler(), ctx.args as *const _) }.into()
    }
}

pub struct SyscallContext<'a, 'b> {
    pub objs: &'a mut ThreadContext<'b>,
    pub args: &'a SysCallArgs,
}

pub trait SyscallHandlerFn<T> {
    fn call(self, ctx: &mut SyscallContext) -> SyscallResult;
}

impl<F, T0> SyscallHandlerFn<()> for F
where
    F: Fn(&mut SyscallContext) -> Result<T0, SyscallError>,
    T0: Into<SysCallReg>,
{
    fn call(self, ctx: &mut SyscallContext) -> SyscallResult {
        self(ctx).map(Into::into)
    }
}

impl<F, T0, T1> SyscallHandlerFn<(T1,)> for F
where
    F: Fn(&mut SyscallContext, T1) -> Result<T0, SyscallError>,
    T0: Into<SysCallReg>,
    T1: From<SysCallReg>,
{
    fn call(self, ctx: &mut SyscallContext) -> SyscallResult {
        self(ctx, ctx.args.get(0).into()).map(Into::into)
    }
}

impl<F, T0, T1, T2> SyscallHandlerFn<(T1, T2)> for F
where
    F: Fn(&mut SyscallContext, T1, T2) -> Result<T0, SyscallError>,
    T0: Into<SysCallReg>,
    T1: From<SysCallReg>,
    T2: From<SysCallReg>,
{
    fn call(self, ctx: &mut SyscallContext) -> SyscallResult {
        self(ctx, ctx.args.get(0).into(), ctx.args.get(1).into()).map(Into::into)
    }
}

impl<F, T0, T1, T2, T3> SyscallHandlerFn<(T1, T2, T3)> for F
where
    F: Fn(&mut SyscallContext, T1, T2, T3) -> Result<T0, SyscallError>,
    T0: Into<SysCallReg>,
    T1: From<SysCallReg>,
    T2: From<SysCallReg>,
    T3: From<SysCallReg>,
{
    fn call(self, ctx: &mut SyscallContext) -> SyscallResult {
        self(
            ctx,
            ctx.args.get(0).into(),
            ctx.args.get(1).into(),
            ctx.args.get(2).into(),
        )
        .map(Into::into)
    }
}

impl<F, T0, T1, T2, T3, T4> SyscallHandlerFn<(T1, T2, T3, T4)> for F
where
    F: Fn(&mut SyscallContext, T1, T2, T3, T4) -> Result<T0, SyscallError>,
    T0: Into<SysCallReg>,
    T1: From<SysCallReg>,
    T2: From<SysCallReg>,
    T3: From<SysCallReg>,
    T4: From<SysCallReg>,
{
    fn call(self, ctx: &mut SyscallContext) -> SyscallResult {
        self(
            ctx,
            ctx.args.get(0).into(),
            ctx.args.get(1).into(),
            ctx.args.get(2).into(),
            ctx.args.get(3).into(),
        )
        .map(Into::into)
    }
}

impl<F, T0, T1, T2, T3, T4, T5> SyscallHandlerFn<(T1, T2, T3, T4, T5)> for F
where
    F: Fn(&mut SyscallContext, T1, T2, T3, T4, T5) -> Result<T0, SyscallError>,
    T0: Into<SysCallReg>,
    T1: From<SysCallReg>,
    T2: From<SysCallReg>,
    T3: From<SysCallReg>,
    T4: From<SysCallReg>,
    T5: From<SysCallReg>,
{
    fn call(self, ctx: &mut SyscallContext) -> SyscallResult {
        self(
            ctx,
            ctx.args.get(0).into(),
            ctx.args.get(1).into(),
            ctx.args.get(2).into(),
            ctx.args.get(3).into(),
            ctx.args.get(4).into(),
        )
        .map(Into::into)
    }
}

impl<F, T0, T1, T2, T3, T4, T5, T6> SyscallHandlerFn<(T1, T2, T3, T4, T5, T6)> for F
where
    F: Fn(&mut SyscallContext, T1, T2, T3, T4, T5, T6) -> Result<T0, SyscallError>,
    T0: Into<SysCallReg>,
    T1: From<SysCallReg>,
    T2: From<SysCallReg>,
    T3: From<SysCallReg>,
    T4: From<SysCallReg>,
    T5: From<SysCallReg>,
    T6: From<SysCallReg>,
{
    fn call(self, ctx: &mut SyscallContext) -> SyscallResult {
        self(
            ctx,
            ctx.args.get(0).into(),
            ctx.args.get(1).into(),
            ctx.args.get(2).into(),
            ctx.args.get(3).into(),
            ctx.args.get(4).into(),
            ctx.args.get(5).into(),
        )
        .map(Into::into)
    }
}

mod export {
    use shadow_shim_helper_rs::notnull::*;

    use super::*;
    use crate::core::worker::Worker;

    #[no_mangle]
    pub extern "C-unwind" fn rustsyscallhandler_new() -> *mut SyscallHandler {
        Box::into_raw(Box::new(SyscallHandler::new()))
    }

    #[no_mangle]
    pub extern "C-unwind" fn rustsyscallhandler_free(handler_ptr: *mut SyscallHandler) {
        if handler_ptr.is_null() {
            return;
        }
        drop(unsafe { Box::from_raw(handler_ptr) });
    }

    #[no_mangle]
    pub extern "C-unwind" fn rustsyscallhandler_syscall(
        sys: *mut SyscallHandler,
        csys: *mut c::SysCallHandler,
        args: *const SysCallArgs,
    ) -> SyscallReturn {
        assert!(!sys.is_null());
        let sys = unsafe { &mut *sys };
        Worker::with_active_host(|host| {
            let mut objs =
                unsafe { ThreadContextObjs::from_syscallhandler(host, notnull_mut_debug(csys)) };
            objs.with_ctx(|ctx| sys.syscall(ctx, unsafe { args.as_ref().unwrap() }).into())
        })
        .unwrap()
    }
}
