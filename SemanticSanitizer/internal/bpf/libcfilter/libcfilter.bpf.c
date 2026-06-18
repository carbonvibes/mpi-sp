//go:build ignore

#include "../lib/vmlinux.h" // IWYU pragma: keep
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_core_read.h>
#include "../lib/util.h"

char __license[] SEC("license") = "Dual MIT/GPL";

static __always_inline void emit_libcfilter_event(void) {
  struct semsan_event *event = semsan_event_new(
      "libcfilter", "libc", SEMSAN_EVENT_ACTION_FINDING, -1, -1);
  if (event == NULL)
    return;

  semsan_copy_text(event->subject, "__gets_chk");
  semsan_event_submit(event);
}

static __always_inline int libc_filter(struct context *sctx) {
  emit_libcfilter_event();
  term_action();

  return 0;
}

SEC("uprobe/libc:__gets_chk")
int libc_filter_wrapper(struct pt_regs *raw_ctx) {
  if (is_expected_comm() != 0)
    return 1;

  struct context sctx;
  INIT_UPROBE_CTX(&sctx, "__gets_chk");

  return libc_filter(&sctx);
}
