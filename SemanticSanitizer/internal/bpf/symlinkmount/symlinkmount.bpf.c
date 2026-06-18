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

static __always_inline void emit_symlinkmount_event(const char *symlink,
                                                    const char *target) {
  struct semsan_event *event = semsan_event_new(
      "symlinkmount", "mount", SEMSAN_EVENT_ACTION_FINDING, -1, -1);
  if (event == NULL)
    return;

  semsan_copy_text(event->subject, symlink);
  semsan_copy_text(event->object, target);
  semsan_event_submit(event);
}

struct trace_event_sys_mount {
  unsigned short common_type;
  unsigned char common_flags;
  unsigned char common_preempt_count;
  int common_pid;
  int __syscall_nr;
  const char *dev_name;
  const char *dir_name;
  const char *type;
  unsigned long flags;
  const void *data;
};

typedef struct {
  __u64 inode;
  uintptr_t name;
} symlink_event_t;

struct {
  __uint(type, BPF_MAP_TYPE_LRU_HASH);
  __type(key, __u64);
  __type(value, symlink_event_t);
  __uint(max_entries, 1024);
} symlink_events SEC(".maps");

struct {
  __uint(type, BPF_MAP_TYPE_ARRAY);
  __type(key, __u32);
  __type(value, char[NAME_MAX]);
  __uint(max_entries, 1);
} last_symlink SEC(".maps");

struct {
  __uint(type, BPF_MAP_TYPE_ARRAY);
  __type(key, __u32);
  __type(value, char[NAME_MAX]);
  __uint(max_entries, 1);
} dev_name SEC(".maps");

static __always_inline int trace_vfs_symlink_filter(struct context *sctx) {
  struct inode *dir = (struct inode *)sctx->args[1];
  struct dentry *dentry = (struct dentry *)sctx->args[2];

  unsigned long i_ino;
  if (BPF_CORE_READ_INTO(&i_ino, dir, i_ino) < 0)
    return 1;

  struct qstr d_name = {0};
  if (BPF_CORE_READ_INTO(&d_name, dentry, d_name) < 0)
    return 1;

  symlink_event_t symlink_event = {
      .inode = i_ino,
      .name = (uintptr_t)d_name.name,
  };
  __u64 pid = bpf_get_current_pid_tgid();
  if (bpf_map_update_elem(&symlink_events, &pid, &symlink_event, BPF_ANY) < 0)
    return 1;

  return 0;
}

SEC("kprobe/vfs_symlink")
int BPF_KPROBE(trace_vfs_symlink_wrapper, void *idmap, struct inode *dir,
               struct dentry *dentry, const char *oldname) {
  if (is_expected_comm() != 0)
    return 1;

  struct context sctx;
  INIT_KFUNC_CTX(&sctx, "vfs_symlink");
  INIT_KFUNC_CTX_ARG(&sctx, 0, idmap);
  INIT_KFUNC_CTX_ARG(&sctx, 1, dir);
  INIT_KFUNC_CTX_ARG(&sctx, 2, dentry);
  INIT_KFUNC_CTX_ARG(&sctx, 3, oldname);

  return trace_vfs_symlink_filter(&sctx);
}

// TODO(msanft): Trace closing and remove files from map

static __always_inline int symlink_mount_filter(struct context *sctx) {
  __u64 pid = bpf_get_current_pid_tgid();
  symlink_event_t *symlink_event = bpf_map_lookup_elem(&symlink_events, &pid);
  if (symlink_event == NULL)
    return 0; // PID hasn't symlinked anything yet.

  char symlink_str[127], dev_name_str[127];
  bpf_probe_read_str(&symlink_str, sizeof(symlink_str),
                     (const char *)symlink_event->name);
  bpf_probe_read_str(&dev_name_str, sizeof(dev_name_str),
                     (const char *)sctx->args[0]);

  if (dev_name_str[0] == '\0')
    return 0; // dev_name is empty e.g.

  if (_strncmp(symlink_str, dev_name_str, sizeof(last_symlink)) != 0)
    return 0;

  // The last symlink is being mounted
  emit_symlinkmount_event(symlink_str, dev_name_str);
  term_action();

  return 0;
}

SEC("tracepoint/syscalls/sys_enter_mount")
int symlink_mount_wrapper(struct trace_event_sys_mount *raw_ctx) {
  if (is_expected_comm() != 0)
    return 1;

  struct context sctx;
  INIT_SYSCALL_TP_CTX(&sctx, raw_ctx->__syscall_nr);
  INIT_KFUNC_CTX_ARG(&sctx, 0, raw_ctx->dev_name);
  INIT_KFUNC_CTX_ARG(&sctx, 1, raw_ctx->dir_name);
  INIT_KFUNC_CTX_ARG(&sctx, 2, raw_ctx->type);
  INIT_KFUNC_CTX_ARG(&sctx, 3, raw_ctx->flags);
  INIT_KFUNC_CTX_ARG(&sctx, 4, raw_ctx->data);

  return symlink_mount_filter(&sctx);
}
