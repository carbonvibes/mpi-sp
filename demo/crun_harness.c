/*
 * crun_harness.c — thin in-process wrapper around libcrun for fuzzing.
 *
 * libcrun.a is compiled with -fsanitize-coverage=trace-pc-guard,trace-cmp so
 * every code path inside crun (JSON parsing, OCI spec validation, namespace
 * setup, rootfs checks) updates EDGES_MAP directly in our process.
 *
 * Coverage scope:
 *   ✓ libocispec JSON parsing   (yajl + generated OCI spec parser)
 *   ✓ OCI spec validation       (field checks, namespace validation, caps)
 *   ✓ rootfs access checks      (stat/access before fork)
 *   ✓ namespace/cgroup setup    (pre-fork codepaths in the parent)
 *   ✗ container child process   (forks → separate address space, expected)
 *
 * fuzz_crun_run_container() is called once per fuzzing iteration and is
 * safe to call repeatedly: it loads config, runs the container, cleans up,
 * and returns the exit status.
 */

/* Pull in crun's generated config.h (HAVE_LIBSYSTEMD, HAVE_SECCOMP, ...) */
#include "config.h"
#include "src/libcrun/container.h"
#include "src/libcrun/error.h"

#include <stdlib.h>
#include <string.h>
#include <stdio.h>

/* Suppress all crun log output during fuzzing — noise kills throughput. */
static void
silent_handler (int errno_, const char *msg, int verbosity, void *arg)
{
  (void) errno_;
  (void) msg;
  (void) verbosity;
  (void) arg;
}

/*
 * fuzz_crun_run_container — run one container iteration in-process.
 *
 * @config_json  NUL-terminated OCI config.json bytes (may be mutated).
 *               root.path must already point to the FUSE rootfs path.
 * @state_root   directory for crun's container state (e.g. /tmp/crun-state-PID)
 * @id           unique container ID for this iteration
 *
 * Returns 0 on success, -1 on load/validation error, or the container exit
 * code.  A hard crash (SIGSEGV/SIGABRT) inside libcrun is surfaced as
 * ExitKind::Crash by LibAFL's InProcessExecutor.
 */
int
fuzz_crun_run_container (const char *config_json,
                         const char *state_root,
                         const char *id)
{
  libcrun_error_t err = NULL;

  /* Load and parse config.json from memory — no disk I/O, no file needed.
   * This exercises the full yajl JSON parser + OCI spec validation. */
  libcrun_container_t *container =
      libcrun_container_load_from_memory (config_json, &err);

  if (container == NULL)
    {
      /* Invalid JSON or spec validation failure — expected for fuzz inputs. */
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
    .force_no_cgroup    = true,   /* avoid cgroup setup — not needed for PoC */
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

  /* Clean up container state so the next iteration can reuse the same id
   * without "container already exists" errors. */
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
