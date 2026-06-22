/*
 * crun_harness.c — in-process wrapper around libcrun for fuzzing.
 *
 * libcrun.a is built with -fsanitize-coverage=trace-pc-guard,trace-cmp, so
 * crun's pre-fork code (JSON parse, OCI validation, ns/cgroup setup, rootfs
 * checks) feeds EDGES_MAP in-process. The container child forks into a
 * separate address space, so its coverage is not captured.
 *
 * fuzz_crun_run_container() runs one iteration; safe to call repeatedly.
 */

/* crun's generated config.h (HAVE_LIBSYSTEMD, HAVE_SECCOMP, ...) */
#include "config.h"
#include "src/libcrun/container.h"
#include "src/libcrun/error.h"

#include <stdlib.h>
#include <string.h>
#include <stdio.h>

/* drop crun log output; noise kills fuzz throughput */
static void
silent_handler (int errno_, const char *msg, int verbosity, void *arg)
{
  (void) errno_;
  (void) msg;
  (void) verbosity;
  (void) arg;
}

/*
 * Run one container iteration in-process.
 *   config_json: NUL-terminated OCI config; root.path must point at FUSE rootfs.
 *   state_root:  crun state dir (e.g. /tmp/crun-state-PID)
 *   id:          unique container ID for this iteration
 * Returns 0, -1 on load/validation error, or the container exit code.
 */
int
fuzz_crun_run_container (const char *config_json,
                         const char *state_root,
                         const char *id)
{
  libcrun_error_t err = NULL;

  /* parse config from memory: no disk I/O */
  libcrun_container_t *container =
      libcrun_container_load_from_memory (config_json, &err);

  if (container == NULL)
    {
      /* bad JSON or spec; expected for fuzz inputs */
      if (err)
        {
          free (err->msg);
          free (err);
        }
      return -1;
    }

  struct libcrun_context_s ctx = {
    .state_root         = state_root,
    .id                 = id,
    .bundle             = NULL,
    .console_socket     = NULL,
    .pid_file           = NULL,
    .notify_socket      = NULL,
    .handler            = NULL,
    .preserve_fds       = 0,
    .listen_fds         = 0,
    .output_handler     = silent_handler,
    .output_handler_arg = NULL,
    .fifo_exec_wait_fd  = -1,
    .systemd_cgroup     = false,
    .detach             = false,
    .no_new_keyring     = true,
    .force_no_cgroup    = true,   /* skip cgroup setup, not needed */
    .no_pivot           = false,
    .argv               = NULL,
    .argc               = 0,
    .handler_manager    = NULL,
  };

  int ret = libcrun_container_run (&ctx, container, 0, &err);

  if (err)
    {
      free (err->msg);
      free (err);
    }

  /* clear state so next iter can reuse id without "already exists" */
  libcrun_error_t del_err = NULL;
  libcrun_container_delete (&ctx, NULL, id, /*force=*/true, &del_err);
  if (del_err)
    {
      free (del_err->msg);
      free (del_err);
    }

  libcrun_container_free (container);
  return ret;
}
