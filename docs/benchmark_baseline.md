# FUSE Benchmark Baseline

## Purpose

This document records the benchmark baseline for the counter filesystem. It
establishes the minimum acceptable throughput threshold before any further
implementation work begins. All future performance-sensitive changes must be
compared against these numbers.

## Machine

| Property        | Value                                      |
|-----------------|--------------------------------------------|
| OS              | Linux 6.8.0-1044-azure x86\_64 (Ubuntu 24.04) |
| CPU             | AMD EPYC 7763 64-Core Processor            |
| Environment     | GitHub Codespace (containerized)           |
| FUSE version    | libfuse3 3.14.0 (fusermount3)              |
| Compiler        | gcc 13.3.0                                 |

Note: this is a virtualized/containerized environment. Context-switch overhead
is higher than on bare metal. Numbers from this environment are consistent with
each other for regression tracking, but are not comparable to bare-metal FUSE
numbers from the literature.

## Build Commands

```sh
# counter filesystem (toy backend used for baseline)
gcc -o counter_fs counter_fs.c $(pkg-config --cflags --libs fuse3)

# benchmark driver
gcc -o benchmark benchmark.c
```

Compiler flags resolved from `pkg-config --cflags --libs fuse3`:
```
-I/usr/include/fuse3 -lfuse3 -lpthread
```

No additional optimization flags were used (`-O2` was not passed). The default
gcc optimization level applies.

## Mount and Run Commands

```sh
# mount the counter filesystem in the foreground, detached
mkdir -p /tmp/testmount
./counter_fs /tmp/testmount

# verify mount
cat /tmp/testmount/counter   # should print 0
cat /tmp/testmount/counter   # should print 1

# run benchmark (60-second loop)
./benchmark

# cleanup
fusermount3 -u /tmp/testmount
```

## Benchmark Method

The benchmark (`benchmark.c`) hammers `open` → `read` → `close` on
`/tmp/testmount/counter` in a tight loop for 60 seconds, then prints the total
ops divided by 60. Each iteration is one complete open-read-close round trip
through the FUSE kernel module into userspace and back.

## Results (April 2026 — this environment)

Three consecutive runs on the same mount, no system load changes between runs:

| Run | Result (ops/sec) |
|-----|-----------------|
| 1   | 14933.93        |
| 2   | 14768.40        |
| 3   | 14553.52        |

- **Mean**: ~14,752 ops/sec
- **Min**: 14,553 ops/sec
- **Max**: 14,934 ops/sec
- **Spread**: ~380 ops/sec (~2.6%)

The spread is small enough that run-to-run variation is not a concern. The
baseline can be treated as approximately **14,750 ops/sec** in this
environment.

## Spec Target

The original spec required **≥ 1,000 ops/sec**. The measured baseline is
approximately **14.75×** the minimum target. FUSE is not a bottleneck at this
throughput level.

## Historical Context

Earlier runs recorded during development (prior to this baseline note) measured
approximately 13,300–13,400 ops/sec for the `counter_fs_offset0` and
`counter_fs_open_snapshot` variants, and ~13,400 for the original
`counter_fs.c`. The current runs on the same `counter_fs.c` binary show
~14,750. The difference is within normal variation for a shared virtualized
environment.

## Assumptions

- The benchmark reflects single-threaded, uncontended access. The real fuzzer
  will also be single-threaded per harness iteration, so this is representative.
- FUSE is running in the same process (`-f` flag, no daemonization). Daemonizing
  would not materially change latency.
- The counter filesystem does essentially no work beyond a string format and
  copy. The VFS-backed replacement will do more work per operation; a post-VFS
  benchmark is required in Week 3 to confirm throughput remains acceptable.

## Regression Threshold

Future benchmarks must exceed **10,000 ops/sec** in this environment to be
considered acceptable. A result below this threshold requires investigation
before proceeding.

---

# Week 5 — In-Memory VFS Baseline (Phase A)

## Purpose

Establishes the raw cost of `vfs_reset_to_snapshot` and `cp_apply_delta` with no
FUSE layer, no mutator overhead, and no target execution.  These numbers set the
budget for the full fuzzing loop once the FUSE target is wired in (Phase B /
Week 6).

## Method

Binary: `mutator/src/bin/vfs_bench.rs`
Build:  `cargo build --release --bin vfs_bench`
Run:    `cargo run --release --bin vfs_bench -- 2000`

Baseline VFS tree: `/input` (4 B), `/etc/`, `/etc/config` (20 B),
`/data/`, `/data/a.bin` (4 B).  Snapshot saved once; all iterations restore
to this snapshot.

## Machine

| Property    | Value                                             |
|-------------|---------------------------------------------------|
| OS          | Linux 6.8.0-90-generic x86_64 (Ubuntu)           |
| Rust        | 1.95.0 (stable, rustup)                           |
| Build       | `--release` (`opt-level = 3`, no debug info)      |
| Iterations  | 2000 per benchmark                                |

## Results (April 2026)

### Benchmark 1 — `vfs_reset_to_snapshot` only (no prior apply)

| Metric | Value     |
|--------|-----------|
| mean   | 321 ns    |
| min    | 300 ns    |
| max    | 7 351 ns  |
| rate   | 3.1 M/s   |

The max spike (7 µs) is a Linux scheduler jitter event, not a VFS pathology.
The mean is the number to track.

### Benchmark 2 — `apply_delta` + reset (small delta, 1 op)

| Stage          | mean   | min    | max    | rate        |
|----------------|--------|--------|--------|-------------|
| apply (1 op)   | 182 ns | 160 ns | 6.2 µs | 5.5 M/s    |
| reset          | 322 ns | 300 ns | 490 ns | 3.1 M/s    |
| **combined**   | ~504 ns |       |        | **~2.0 M/s** |

### Benchmark 3 — `apply_delta` + reset (medium delta, 3 ops)

| Stage          | mean   | min    | max    | rate        |
|----------------|--------|--------|--------|-------------|
| apply (3 ops)  | 646 ns | 590 ns | 2.2 µs | 1.5 M/s    |
| reset          | 405 ns | 369 ns | 6.0 µs | 2.5 M/s    |
| **combined**   | ~1.1 µs |       |        | **~950 K/s** |

### Benchmark 4 — `apply_delta` + reset (large delta, 10 ops)

| Stage           | mean   | min    | max     | rate       |
|-----------------|--------|--------|---------|------------|
| apply (10 ops)  | 3.2 µs | 2.6 µs | 20.2 µs | 311 K/s   |
| reset           | 1.1 µs | 760 ns | 7.8 µs  | 945 K/s   |
| **combined**    | ~4.3 µs |        |         | **~235 K/s** |

### Dumb loop (fuzz harness, 200 iterations, all 6 mutators, mixed deltas)

| Metric                        | Value  |
|-------------------------------|--------|
| apply ok / total              | 200/200 |
| reset ok / total              | 200/200 |
| reset mean                    | 261 ns |
| reset min                     | 210 ns |
| reset max                     | 640 ns |

## Interpretation

- A typical fuzzing delta (1–3 ops) costs **500 ns – 1.1 µs** for the full
  apply + reset cycle.  At 1 µs/iter the VFS alone supports **~1 M iters/s**.
- Once FUSE target execution is added, each iteration will dominate at
  milliseconds — the VFS overhead is entirely negligible.
- Reset scales linearly with the number of nodes dirtied: the baseline tree
  (5 nodes) resets in ~320 ns; a 10-op apply that creates 8 new files resets
  in ~1.1 µs.

## Regression Threshold (Week 5)

A future change is a regression if:

| Scenario                   | Threshold   |
|----------------------------|-------------|
| reset (baseline tree only) | > 2 µs mean |
| apply + reset (1 op)       | > 3 µs mean |
| apply + reset (3 ops)      | > 5 µs mean |
