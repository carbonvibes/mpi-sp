//go:build ignore

#include "../lib/vmlinux.h" // IWYU pragma: keep
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_core_read.h>
#include "../lib/util.h"

char __license[] SEC("license") = "Dual MIT/GPL";

#define SYSCALL_DISALLOWED 1

static __always_inline void emit_syscallfilter_event(__s32 syscall_id) {
  struct semsan_event *event = semsan_event_new(
      "syscallfilter", "syscall", SEMSAN_EVENT_ACTION_FINDING, syscall_id, -1);
  if (event == NULL)
    return;

  semsan_event_submit(event);
}

struct trace_event_raw_sys_enter {
  unsigned short common_type;
  unsigned char common_flags;
  unsigned char common_preempt_count;
  int common_pid;
  long id;
  unsigned long args[6];
};

struct {
  __uint(type, BPF_MAP_TYPE_ARRAY);
  __type(key, __u32);
  __type(value, __u8);
  __uint(max_entries, 400); // should be more than enough for all syscalls.
} disallowed_syscalls SEC(".maps");

static __always_inline int syscall_filter(struct context *sctx) {
  __u8 *ret = bpf_map_lookup_elem(&disallowed_syscalls, &sctx->syscall_id);
  if (ret == NULL || *ret != SYSCALL_DISALLOWED)
    return 1;

  emit_syscallfilter_event((__s32)sctx->syscall_id);
  term_action();

  return 0;
}

SEC("tracepoint/raw_syscalls/sys_enter")
int syscall_filter_wrapper(struct trace_event_raw_sys_enter *raw_ctx) {
  if (is_expected_comm() != 0)
    return 1;

  struct context sctx;
  INIT_SYSCALL_CTX(&sctx, raw_ctx);

  return syscall_filter(&sctx);
}
