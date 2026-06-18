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



struct test_struct { char field1[64]; int field2; };



static __always_inline int sys_enter_filter(struct context *sctx) {
  __u8 is_violated = 0;

  // Sanitizer logic goes here

  if (is_violated)
    term_action();

  return 0;
}

SEC("tracepoint/raw_syscalls/sys_enter")
int sys_enter_wrapper(struct trace_event_raw_sys_enter *raw_ctx) {
  if (is_expected_comm() != 0)
    return 1;

  struct context sctx;
  INIT_SYSCALL_CTX(&sctx, raw_ctx);

  return sys_enter_filter(&sctx);
}



