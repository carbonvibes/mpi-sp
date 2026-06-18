//go:build ignore

#define __TARGET_ARCH_x86

#include "../lib/vmlinux.h" // IWYU pragma: keep

#include <linux/types.h>
#include <linux/limits.h>
#include <linux/stat.h>
#include <bpf/bpf_core_read.h>
#include <bpf/bpf_tracing.h>
#include "../lib/util.h"

char __license[] SEC("license") = "Dual MIT/GPL";

#define O_CREAT 00000100
#define O_TRUNC 00001000
#define O_NOFOLLOW 00400000

struct trace_event_sys_open {
  unsigned short common_type;
  unsigned char common_flags;
  unsigned char common_preempt_count;
  int common_pid;
  int __syscall_nr;
  const char *filename;
  int flags;
  umode_t mode;
};

struct trace_event_sys_openat {
  unsigned short common_type;
  unsigned char common_flags;
  unsigned char common_preempt_count;
  int common_pid;
  int __syscall_nr;
  int dfd;
  const char *filename;
  int flags;
  umode_t mode;
};

struct open_how {
  __u64 flags;
  __u64 mode;
  __u64 resolve;
};

struct trace_event_sys_openat2 {
  unsigned short common_type;
  unsigned char common_flags;
  unsigned char common_preempt_count;
  int common_pid;
  int __syscall_nr;
  int dfd;
  const char *filename;
  struct open_how *how;
  __u64 size;
};

static __always_inline void emit_dirownership_event(const char *operation,
                                                    const char *filename) {
  struct semsan_event *event = semsan_event_new(
      "dirownership", operation, SEMSAN_EVENT_ACTION_FINDING, -1, -1);
  if (event == NULL)
    return;

  semsan_event_set_subject_kernel(event, filename);
  semsan_event_submit(event);
}

static __always_inline void emit_dirownership_event_user(const char *operation,
                                                         const char *filename,
                                                         __s32 syscall_id) {
  struct semsan_event *event = semsan_event_new(
      "dirownership", operation, SEMSAN_EVENT_ACTION_FINDING, syscall_id, -1);
  if (event == NULL)
    return;

  semsan_event_set_subject_user(event, filename);
  semsan_event_submit(event);
}

static __always_inline unsigned char is_root() {
  __u64 gid_uid = bpf_get_current_uid_gid();
  __u32 uid = gid_uid & 0xFFFFFFFF;
  __u32 gid = gid_uid >> 32;
  if (uid == 0 || gid == 0) {
    return 1;
  }
  return 0;
}

static __always_inline unsigned char is_inode_root_owned(struct inode *inode) {
  __u32 uid = BPF_CORE_READ(inode, i_uid.val);
  __u32 gid = BPF_CORE_READ(inode, i_gid.val);

  if (uid == 0 || gid == 0) {
    return 1;
  }

  return 0;
}

static __always_inline char is_parent_root_owned(struct dentry *dentry) {
  struct inode *inode = BPF_CORE_READ(dentry, d_parent, d_inode);
  if (inode == NULL)
    return -22; // EINVAL

  return is_inode_root_owned(inode);
}

static __always_inline unsigned char
is_path_below_tmp_subdir(const char *path) {
  if (path[0] != '/' || path[1] != 't' || path[2] != 'm' || path[3] != 'p' ||
      path[4] != '/')
    return 0;

#pragma unroll
  for (int i = 5; i < SEMSAN_EVENT_TEXT_LEN; i++) {
    if (path[i] == '\0')
      return 0;
    if (path[i] == '/')
      return 1;
  }

  return 0;
}

static __always_inline int unsafe_openat_filter(struct context *sctx) {
  const char *filename = (const char *)sctx->args[1];
  int flags = (int)sctx->args[2];

  if ((flags & O_NOFOLLOW) != 0)
    return 0;

  if ((flags & (O_CREAT | O_TRUNC)) == 0)
    return 0;

  char filename_buf[SEMSAN_EVENT_TEXT_LEN];
  if (bpf_probe_read_user_str(filename_buf, sizeof(filename_buf), filename) < 0)
    return 0;

  if (is_path_below_tmp_subdir(filename_buf) == 0)
    return 0;

  emit_dirownership_event_user("openat_without_o_nofollow", filename,
                               sctx->syscall_id);
  term_action();

  return 0;
}

static __always_inline int unsafe_file_open_filter(struct context *sctx) {
  struct file *file = (struct file *)sctx->args[0];
  if (file == NULL)
    return 0;

  unsigned int flags = BPF_CORE_READ(file, f_flags);
  if ((flags & O_NOFOLLOW) != 0)
    return 0;

  if ((flags & (O_CREAT | O_TRUNC)) == 0)
    return 0;

  struct dentry *dentry = BPF_CORE_READ(file, f_path.dentry);
  if (dentry == NULL)
    return 0;

  const unsigned char *filename = BPF_CORE_READ(dentry, d_name.name);
  if (is_parent_root_owned(dentry) == 0) {
    emit_dirownership_event("open_without_o_nofollow", (const char *)filename);
    term_action();
  }

  return 0;
}

SEC("kprobe/security_file_open")
int BPF_KPROBE(unsafe_file_open_wrapper, struct file *file) {
  if (is_root() == 0)
    return 0;
  if (is_expected_comm() != 0)
    return 0;

  struct context sctx;
  INIT_KFUNC_CTX(&sctx, "security_file_open");
  INIT_KFUNC_CTX_ARG(&sctx, 0, file);

  return unsafe_file_open_filter(&sctx);
}

SEC("tracepoint/syscalls/sys_enter_open")
int unsafe_open_wrapper(struct trace_event_sys_open *raw_ctx) {
  if (is_root() == 0)
    return 0;
  if (is_expected_comm() != 0)
    return 0;

  struct context sctx;
  INIT_SYSCALL_TP_CTX(&sctx, raw_ctx->__syscall_nr);
  INIT_KFUNC_CTX_ARG(&sctx, 1, raw_ctx->filename);
  INIT_KFUNC_CTX_ARG(&sctx, 2, raw_ctx->flags);
  INIT_KFUNC_CTX_ARG(&sctx, 3, raw_ctx->mode);

  return unsafe_openat_filter(&sctx);
}

SEC("tracepoint/syscalls/sys_enter_openat")
int unsafe_openat_wrapper(struct trace_event_sys_openat *raw_ctx) {
  if (is_root() == 0)
    return 0;
  if (is_expected_comm() != 0)
    return 0;

  struct context sctx;
  INIT_SYSCALL_TP_CTX(&sctx, raw_ctx->__syscall_nr);
  INIT_KFUNC_CTX_ARG(&sctx, 0, raw_ctx->dfd);
  INIT_KFUNC_CTX_ARG(&sctx, 1, raw_ctx->filename);
  INIT_KFUNC_CTX_ARG(&sctx, 2, raw_ctx->flags);
  INIT_KFUNC_CTX_ARG(&sctx, 3, raw_ctx->mode);

  return unsafe_openat_filter(&sctx);
}

SEC("tracepoint/syscalls/sys_enter_openat2")
int unsafe_openat2_wrapper(struct trace_event_sys_openat2 *raw_ctx) {
  if (is_root() == 0)
    return 0;
  if (is_expected_comm() != 0)
    return 0;

  struct open_how how;
  if (bpf_probe_read_user(&how, sizeof(how), raw_ctx->how) < 0)
    return 0;

  struct context sctx;
  INIT_SYSCALL_TP_CTX(&sctx, raw_ctx->__syscall_nr);
  INIT_KFUNC_CTX_ARG(&sctx, 0, raw_ctx->dfd);
  INIT_KFUNC_CTX_ARG(&sctx, 1, raw_ctx->filename);
  INIT_KFUNC_CTX_ARG(&sctx, 2, how.flags);
  INIT_KFUNC_CTX_ARG(&sctx, 3, how.mode);

  return unsafe_openat_filter(&sctx);
}

static __always_inline int unsafe_move_mount_filter(struct context *sctx) {
  struct path *path = (struct path *)sctx->args[0];
  struct dentry *dentry = BPF_CORE_READ(path, dentry);
  if (dentry == NULL)
    return 0;

  const unsigned char *filename = BPF_CORE_READ(dentry, d_name.name);

  if (is_parent_root_owned(dentry) == 0) {
    emit_dirownership_event("move_mount", (const char *)filename);
    term_action();
  }

  return 0;
}

SEC("kprobe/do_move_mount")
int BPF_KPROBE(unsafe_move_mount_wrapper, struct path *old_path,
               struct path *new_path, char beneath) {
  if (is_root() == 0)
    return 0;
  if (is_expected_comm() != 0)
    return 0;

  struct context sctx;
  INIT_KFUNC_CTX(&sctx, "do_move_mount");
  INIT_KFUNC_CTX_ARG(&sctx, 0, old_path);
  INIT_KFUNC_CTX_ARG(&sctx, 1, new_path);
  INIT_KFUNC_CTX_ARG(&sctx, 2, beneath);

  return unsafe_move_mount_filter(&sctx);
}

// TODO(msanft): Add a sanitizer for bind mounts in the old mount API.
// Somehow ebpf-go doesn't want us to attach to `__do_loopback`.

static __always_inline int unsafe_chmod_filter(struct context *sctx) {
  const struct path *path = (const struct path *)sctx->args[0];
  struct dentry *dentry = BPF_CORE_READ(path, dentry);
  if (dentry == NULL)
    return 0;

  const unsigned char *filename = BPF_CORE_READ(dentry, d_name.name);

  if (is_parent_root_owned(dentry) == 0) {
    emit_dirownership_event("chmod", (const char *)filename);
    term_action();
  }

  return 0;
}

SEC("kprobe/chmod_common")
int BPF_KPROBE(unsafe_chmod_wrapper, const struct path *path, umode_t mode) {
  if (is_root() == 0)
    return 0;
  if (is_expected_comm() != 0)
    return 0;

  struct context sctx;
  INIT_KFUNC_CTX(&sctx, "chmod_common");
  INIT_KFUNC_CTX_ARG(&sctx, 0, path);
  INIT_KFUNC_CTX_ARG(&sctx, 1, mode);

  return unsafe_chmod_filter(&sctx);
}

static __always_inline int unsafe_chown_filter(struct context *sctx) {
  const struct path *path = (const struct path *)sctx->args[0];
  struct dentry *dentry = BPF_CORE_READ(path, dentry);
  if (dentry == NULL)
    return 0;

  const unsigned char *filename = BPF_CORE_READ(dentry, d_name.name);

  if (is_parent_root_owned(dentry) == 0) {
    emit_dirownership_event("chown", (const char *)filename);
    term_action();
  }

  return 0;
}

SEC("kprobe/chown_common")
int BPF_KPROBE(unsafe_chown_wrapper, const struct path *path, uid_t user,
               gid_t group) {
  if (is_root() == 0)
    return 0;
  if (is_expected_comm() != 0)
    return 0;

  struct context sctx;
  INIT_KFUNC_CTX(&sctx, "chown_common");
  INIT_KFUNC_CTX_ARG(&sctx, 0, path);
  INIT_KFUNC_CTX_ARG(&sctx, 1, user);
  INIT_KFUNC_CTX_ARG(&sctx, 2, group);

  return unsafe_chown_filter(&sctx);
}

static __always_inline int unsafe_rmdir_filter(struct context *sctx) {
  struct inode *dir_inode = (struct inode *)sctx->args[1];
  if (dir_inode == NULL)
    return 0;

  struct dentry *d = (struct dentry *)sctx->args[2];
  const unsigned char *filename = BPF_CORE_READ(d, d_name.name);

  if (is_inode_root_owned(dir_inode) == 0) {
    emit_dirownership_event("rmdir", (const char *)filename);
    term_action();
  }

  return 0;
}

SEC("kprobe/vfs_rmdir")
int BPF_KPROBE(unsafe_rmdir_wrapper, void *idmap, struct inode *dir,
               struct dentry *dentry) {
  if (is_root() == 0)
    return 0;
  if (is_expected_comm() != 0)
    return 0;

  struct context sctx;
  INIT_KFUNC_CTX(&sctx, "vfs_rmdir");
  INIT_KFUNC_CTX_ARG(&sctx, 0, idmap);
  INIT_KFUNC_CTX_ARG(&sctx, 1, dir);
  INIT_KFUNC_CTX_ARG(&sctx, 2, dentry);

  return unsafe_rmdir_filter(&sctx);
}

// TODO(msanft): Make this work with symlinks.
// Unfortunately, the current implementation hooks *after* the path is resolved,
// meaning that a situation where `some-user-owned-dir/symlink-to-bin-foo` is
// executed by root will not be detected if `/bin` (where `/bin/foo` is located)
// is owned by root.

static __always_inline int unsafe_execve_filter(struct context *sctx) {
  struct linux_binprm *b = (struct linux_binprm *)sctx->args[0];
  struct file *file = BPF_CORE_READ(b, file);
  if (file == NULL)
    return 0;

  struct dentry *dentry = BPF_CORE_READ(file, f_path.dentry);
  if (dentry == NULL)
    return 0;

  const char *filename = BPF_CORE_READ(b, filename);

  if (is_parent_root_owned(dentry) == 0) {
    emit_dirownership_event("execve", filename);
    term_action();
  }

  return 0;
}

SEC("kprobe/bprm_execve")
int BPF_KPROBE(unsafe_execve_wrapper, struct linux_binprm *bprm) {
  if (is_root() == 0)
    return 0;
  if (is_expected_comm() != 0)
    return 0;

  struct context sctx;
  INIT_KFUNC_CTX(&sctx, "bprm_execve");
  INIT_KFUNC_CTX_ARG(&sctx, 0, bprm);

  return unsafe_execve_filter(&sctx);
}
