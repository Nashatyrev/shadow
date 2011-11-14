/*
 * The Shadow Simulator
 *
 * Copyright (c) 2010-2011 Rob Jansen <jansen@cs.umn.edu>
 *
 * This file is part of Shadow.
 *
 * Shadow is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * Shadow is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with Shadow.  If not, see <http://www.gnu.org/licenses/>.
 */

#include <arpa/inet.h>
#include <stdlib.h>

#include "shd-filetransfer.h"

/* my global structure to hold all variable, node-specific application state */
FileTransfer* ft;

FileTransfer** filetransfer_init(FileTransfer* existingFT) {
	/* set our pointer to the global struct, and return its address so the
	 * caller can register it with shadow.
	 */
	ft = existingFT;
	return &ft;
}

static void _filetransfer_logCallback(enum service_filegetter_loglevel level, const gchar* message) {
	if(level == SFG_CRITICAL) {
		ft->shadowlib->log(G_LOG_LEVEL_CRITICAL, __FUNCTION__, "%s", message);
	} else if(level == SFG_WARNING) {
		ft->shadowlib->log(G_LOG_LEVEL_WARNING, __FUNCTION__, "%s", message);
	} else if(level == SFG_NOTICE) {
		ft->shadowlib->log(G_LOG_LEVEL_MESSAGE, __FUNCTION__, "%s", message);
	} else if(level == SFG_INFO) {
		ft->shadowlib->log(G_LOG_LEVEL_INFO, __FUNCTION__, "%s", message);
	} else if(level == SFG_DEBUG) {
		ft->shadowlib->log(G_LOG_LEVEL_DEBUG, __FUNCTION__, "%s", message);
	} else {
		/* we dont care */
	}
}

static in_addr_t _filetransfer_HostnameCallback(const gchar* hostname) {
	in_addr_t addr = 0;

	/* get the address in network order */
	if(g_strncasecmp(hostname, "none", 4) == 0) {
		addr = htonl(INADDR_NONE);
	} else if(g_strncasecmp(hostname, "localhost", 9) == 0) {
		addr = htonl(INADDR_LOOPBACK);
	} else {
		addr = ft->shadowlib->resolveHostname((gchar*) hostname);
	}

	return addr;
}

static void _filetransfer_wakeupCallback(gpointer data) {
	service_filegetter_activate((service_filegetter_tp) data, 0);
}

/* called from inner filegetter code when it wants to sleep for some seconds */
static void _filetransfer_sleepCallback(gpointer sfg, guint seconds) {
	/* schedule a callback from shadow to wakeup the filegetter */
	ft->shadowlib->createCallback(&_filetransfer_wakeupCallback, sfg, seconds*1000);
}

/* create a new node using this plug-in */
void filetransfer_new(int argc, char* argv[]) {
	ft->shadowlib->log(G_LOG_LEVEL_DEBUG, __FUNCTION__, "filetransfer_new called");

	ft->client = NULL;
	ft->server = NULL;

	const gchar* USAGE = "\nFiletransfer usage:\n"
			"\t'server serverListenPort pathToDocRoot'\n"
			"\t'client single fileServerHostname fileServerPort socksServerHostname(or 'none') socksServerPort nDownloads pathToFile'\n"
			"\t'client double fileServerHostname fileServerPort socksServerHostname(or 'none') socksServerPort pathToFile1 pathToFile2 pathToFile3(or 'none') secondsPause'\n"
			"\t'client multi pathToDownloadSpec socksServerHostname(or 'none') socksServerPort pathToThinktimeCDF(or 'none') secondsRunTime(or '-1')'\n";
	if(argc < 1) goto printUsage;

	/* parse command line args */
	gchar* mode = argv[0];


	if(g_strcasecmp(mode, "client") == 0) {
		/* check client args */
		if(argc < 2) goto printUsage;

		ft->client = g_new0(service_filegetter_t, 1);
		gint sockd = -1;

		gchar* clientMode = argv[1];

		if(g_strncasecmp(clientMode, "single", 6) == 0) {
			service_filegetter_single_args_t args;

			args.http_server.host = argv[2];
			args.http_server.port = argv[3];
			args.socks_proxy.host = argv[4];
			args.socks_proxy.port = argv[5];
			args.num_downloads = argv[6];
			args.filepath = argv[7];

			args.log_cb = &_filetransfer_logCallback;
			args.hostbyname_cb = &_filetransfer_HostnameCallback;

			service_filegetter_start_single(ft->client, &args, &sockd);
		} else if(g_strncasecmp(clientMode, "double", 6) == 0){
			service_filegetter_double_args_t args;

			args.http_server.host = argv[2];
			args.http_server.port = argv[3];
			args.socks_proxy.host = argv[4];
			args.socks_proxy.port = argv[5];
			args.filepath1 = argv[6];
			args.filepath2 = argv[7];
			args.filepath3 = argv[8];
			args.pausetime_seconds = argv[9];

			args.log_cb = &_filetransfer_logCallback;
			args.hostbyname_cb = &_filetransfer_HostnameCallback;
			args.sleep_cb = &_filetransfer_sleepCallback;

			service_filegetter_start_double(ft->client, &args, &sockd);
		} else if(g_strncasecmp(clientMode, "multi", 5) == 0) {
			service_filegetter_multi_args_t args;

			args.server_specification_filepath = argv[2];
			args.socks_proxy.host = argv[3];
			args.socks_proxy.port = argv[4];
			args.thinktimes_cdf_filepath = argv[5];
			args.runtime_seconds = argv[6];

			if(g_strncasecmp(args.thinktimes_cdf_filepath, "none", 4) == 0) {
				args.thinktimes_cdf_filepath = NULL;
			}

			args.log_cb = &_filetransfer_logCallback;
			args.hostbyname_cb = &_filetransfer_HostnameCallback;
			args.sleep_cb = &_filetransfer_sleepCallback;

			service_filegetter_start_multi(ft->client, &args, &sockd);
		} else {
			/* unknown client mode */
			g_free(ft->client);
			ft->client = NULL;
			goto printUsage;
		}

		/* successfull arguments */
		if(sockd >= 0) {
			service_filegetter_activate(ft->client, sockd);
		}
	} else if(g_strcasecmp(mode, "server") == 0) {
		/* check server args */
		if(argc < 3) goto printUsage;

		/* we are running a server */
		in_addr_t listenIP = INADDR_ANY;
		in_port_t listenPort = (in_port_t) atoi(argv[1]);
		gchar* docroot = argv[2];

		ft->server = g_new0(fileserver_t, 1);
		ft->shadowlib->log(G_LOG_LEVEL_INFO, __FUNCTION__, "serving '%s' on port %u", docroot, listenPort);
		enum fileserver_code res = fileserver_start(ft->server, htonl(listenIP), htons(listenPort), docroot, 1000);

		if(res == FS_SUCCESS) {
			ft->shadowlib->log(G_LOG_LEVEL_MESSAGE, __FUNCTION__, "fileserver running on at %s:%u", inet_ntoa((struct in_addr){listenIP}), listenPort);
		} else {
			ft->shadowlib->log(G_LOG_LEVEL_CRITICAL, __FUNCTION__, "fileserver error, not started!");
		}
	} else {
		/* not client or server... */
		goto printUsage;
	}

	return;
printUsage:
	ft->shadowlib->log(G_LOG_LEVEL_CRITICAL, __FUNCTION__, (gchar*)USAGE);
	return;
}

void filetransfer_free() {
	ft->shadowlib->log(G_LOG_LEVEL_DEBUG, __FUNCTION__, "filetransfer_free called");

	if(ft->client) {
		/* stop the client */
		service_filegetter_stop(ft->client);

		/* cleanup */
		g_free(ft->client);
		ft->client = NULL;
	}

	if(ft->server) {
		/* log statistics */
		ft->shadowlib->log(G_LOG_LEVEL_MESSAGE, __FUNCTION__,
				"fileserver stats: %lu bytes in, %lu bytes out, %lu replies",
				ft->server->bytes_received, ft->server->bytes_sent,
				ft->server->replies_sent);

		/* shutdown fileserver */
		ft->shadowlib->log(G_LOG_LEVEL_INFO, __FUNCTION__, "shutting down fileserver");
		fileserver_shutdown(ft->server);

		/* cleanup */
		g_free(ft->server);
		ft->server = NULL;
	}
}

void filetransfer_activate(gint socketDesriptor) {
	ft->shadowlib->log(G_LOG_LEVEL_DEBUG, __FUNCTION__, "activating socket %i", socketDesriptor);

	if (ft->client) {
		/* activate client */
		service_filegetter_activate(ft->client, socketDesriptor);
	} else if (ft->server) {
		/* activate server and print updated stats */
		enum fileserver_code result = fileserver_activate(ft->server, socketDesriptor);
		ft->shadowlib->log(G_LOG_LEVEL_DEBUG, __FUNCTION__,
				"fileserver activation result: %s (%lu bytes in, %lu bytes out, %lu replies)",
				fileserver_codetoa(result),  ft->server->bytes_received,
				ft->server->bytes_sent, ft->server->replies_sent);
	}
}
