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

struct trace_event_raw_sys_enter {
  unsigned short common_type;
  unsigned char common_flags;
  unsigned char common_preempt_count;
  int common_pid;
  long id;
  unsigned long args[6];
};

enum canary_match_mode {
  CANARY_MATCH_DIRECT = 0,
  CANARY_MATCH_STR_ARRAY = 1,
};

#define MAX_CANARY_ARRAY_ELEMS 16
#define MAX_CANARY_NEEDLE_LEN 32

struct canary_rule {
  __u32 arg_idx;
  __u32 match_mode;
  __u32 needle_len;
  __u32 _pad;
  char disallowed_str[MAX_CANARY_NEEDLE_LEN];
};

static __always_inline void copy_canary_needle(struct semsan_event *event,
                                               const struct canary_rule *rule) {
#pragma unroll
  for (int i = 0; i < MAX_CANARY_NEEDLE_LEN; i++) {
    if (i >= rule->needle_len)
      break;
    event->subject[i] = rule->disallowed_str[i];
  }
}

static __always_inline void emit_canary_event(const struct canary_rule *rule,
                                              __u32 syscall_id) {
  struct semsan_event *event = semsan_event_new(
      "canary", "syscall_arg_substring", SEMSAN_EVENT_ACTION_FINDING,
      (__s32)syscall_id, (__s32)rule->arg_idx);
  if (event == NULL)
    return;

  copy_canary_needle(event, rule);
  semsan_event_submit(event);
}

struct canary_search_ctx {
  char haystack[MAX_ARGSTRING_LEN];
  int hay_len;
  int found;
  const struct canary_rule *rule;
};

struct {
  __uint(type, BPF_MAP_TYPE_ARRAY);
  __type(key, __u32);
  __type(value, struct canary_rule);
  __uint(max_entries, 400);
} canaries SEC(".maps");

#define CANARY_CMP_AT(idx)                                                     \
  do {                                                                         \
    if (rule->needle_len > (idx) &&                                            \
        haystack[off + (idx)] != rule->disallowed_str[(idx)])                  \
      return 0;                                                                \
  } while (0)

static __always_inline int canary_match_at(const char *haystack, int hay_len,
                                           int off,
                                           const struct canary_rule *rule) {
  if (rule->needle_len == 0 || rule->needle_len > MAX_CANARY_NEEDLE_LEN)
    return 0;
  if (off + rule->needle_len > hay_len)
    return 0;

  CANARY_CMP_AT(0);
  CANARY_CMP_AT(1);
  CANARY_CMP_AT(2);
  CANARY_CMP_AT(3);
  CANARY_CMP_AT(4);
  CANARY_CMP_AT(5);
  CANARY_CMP_AT(6);
  CANARY_CMP_AT(7);
  CANARY_CMP_AT(8);
  CANARY_CMP_AT(9);
  CANARY_CMP_AT(10);
  CANARY_CMP_AT(11);
  CANARY_CMP_AT(12);
  CANARY_CMP_AT(13);
  CANARY_CMP_AT(14);
  CANARY_CMP_AT(15);
  CANARY_CMP_AT(16);
  CANARY_CMP_AT(17);
  CANARY_CMP_AT(18);
  CANARY_CMP_AT(19);
  CANARY_CMP_AT(20);
  CANARY_CMP_AT(21);
  CANARY_CMP_AT(22);
  CANARY_CMP_AT(23);
  CANARY_CMP_AT(24);
  CANARY_CMP_AT(25);
  CANARY_CMP_AT(26);
  CANARY_CMP_AT(27);
  CANARY_CMP_AT(28);
  CANARY_CMP_AT(29);
  CANARY_CMP_AT(30);
  CANARY_CMP_AT(31);

  return 1;
}

static long canary_search_cb(__u64 off, void *opaque) {
  struct canary_search_ctx *ctx = opaque;
  const struct canary_rule *rule = ctx->rule;

  if (off >= MAX_ARGSTRING_LEN)
    return 1;
  if (off + rule->needle_len > ctx->hay_len)
    return 1;
  if (!canary_match_at(ctx->haystack, ctx->hay_len, off, rule))
    return 0;

  ctx->found = 1;
  return 1;
}

static __noinline int canary_contains_substring(struct canary_search_ctx *ctx) {
  const struct canary_rule *rule = ctx->rule;

  if (rule->needle_len == 0 || rule->needle_len > MAX_CANARY_NEEDLE_LEN ||
      ctx->hay_len < rule->needle_len)
    return 0;

  ctx->found = 0;
  bpf_loop(ctx->hay_len, canary_search_cb, ctx, 0);
  return ctx->found;
}

static __noinline int canary_match_user_string(unsigned long arg_ptr,
                                               const struct canary_rule *rule,
                                               struct canary_search_ctx *ctx) {
  if (arg_ptr == 0)
    return 0;

  ctx->rule = rule;
  ctx->found = 0;

  long ret = bpf_probe_read_user_str(ctx->haystack, sizeof(ctx->haystack),
                                     (void *)arg_ptr);
  if (ret <= 1)
    return 0;

  int hay_len = ret - 1;
  if (hay_len > MAX_ARGSTRING_LEN - 1)
    hay_len = MAX_ARGSTRING_LEN - 1;
  ctx->hay_len = hay_len;

  return canary_contains_substring(ctx);
}

static __noinline int
canary_match_user_string_array(unsigned long array_ptr,
                               const struct canary_rule *rule,
                               struct canary_search_ctx *ctx) {
  if (array_ptr == 0)
    return 0;

#pragma clang loop unroll(disable)
  for (int i = 0; i < MAX_CANARY_ARRAY_ELEMS; i++) {
    unsigned long elem_ptr = 0;
    long ret = bpf_probe_read_user(&elem_ptr, sizeof(elem_ptr),
                                   (void *)(array_ptr + i * sizeof(elem_ptr)));
    if (ret < 0 || elem_ptr == 0)
      break;

    if (canary_match_user_string(elem_ptr, rule, ctx))
      return 1;
  }

  return 0;
}

SEC("tracepoint/raw_syscalls/sys_enter")
int canary_filter_wrapper(struct trace_event_raw_sys_enter *raw_ctx) {
  long raw_id = BPF_CORE_READ(raw_ctx, id);
  __u32 syscall_id = (__u32)raw_id;
  struct canary_rule *rule = bpf_map_lookup_elem(&canaries, &syscall_id);
  if (rule == NULL || rule->needle_len == 0)
    return 0;

  if (is_expected_comm() != 0)
    return 0;

  unsigned long arg_ptr;
  switch (rule->arg_idx) {
  case 0:
    BPF_CORE_READ_INTO(&arg_ptr, raw_ctx, args[0]);
    break;
  case 1:
    BPF_CORE_READ_INTO(&arg_ptr, raw_ctx, args[1]);
    break;
  case 2:
    BPF_CORE_READ_INTO(&arg_ptr, raw_ctx, args[2]);
    break;
  case 3:
    BPF_CORE_READ_INTO(&arg_ptr, raw_ctx, args[3]);
    break;
  case 4:
    BPF_CORE_READ_INTO(&arg_ptr, raw_ctx, args[4]);
    break;
  case 5:
    BPF_CORE_READ_INTO(&arg_ptr, raw_ctx, args[5]);
    break;
  default:
    return 0;
  }

  struct canary_search_ctx ctx = {};
  int matched = 0;
  switch (rule->match_mode) {
  case CANARY_MATCH_DIRECT:
    matched = canary_match_user_string(arg_ptr, rule, &ctx);
    break;
  case CANARY_MATCH_STR_ARRAY:
    matched = canary_match_user_string_array(arg_ptr, rule, &ctx);
    break;
  default:
    return 0;
  }

  if (matched) {
    emit_canary_event(rule, syscall_id);
    term_action();
  }

  return 0;
}
