# Week 5 — Mutator & Dumb Loop (Phase A)

## Overview

Week 5 built the Rust-side fuzzing layer that sits between LibAFL and the
in-memory VFS.  The deliverable is a working, measurable mutation → apply →
reset loop that exercises all seven mutation strategies across all seven
`FsOpKind` variants, with per-op failure reporting and semantic yield tracking.

The week is split into two phases:

- **Phase A (complete)** — Rust `FsDelta`/`FsOp` corpus types, seven mutator
  stages, full FFI bridge with per-op result inspection, `MAX_OPS` cap,
  `validate_delta` debug assertions, dumb loop harness with semantic yield
  metric, 22 unit + E2E integration tests, C serialization cleanup, benchmarks.
- **Phase B (Week 6)** — FUSE callback logging, `MutationGuidance` wiring,
  full closed loop.

---

## Architecture

```
LibAFL scheduler
     │
     ▼
  FsDelta  ←── 7 Rust mutator stages (mutators.rs)
     │           MAX_OPS = 20 cap enforced by AddFileOp,
     │           SpliceDelta, DestructiveMutator
     │
     │  ffi::apply_delta(vfs, &delta)
     │    validate_delta()  [debug_assert]
     │    delta_add_*() × n_ops  [all 7 FsOpKind variants]
     ▼
  C fs_delta_t
     │
     │  cp_apply_delta(vfs, delta, dry_run=0)
     ▼
  cp_result_t  { total_ops, succeeded, failed, results[] }
     │
     │  read succeeded/failed → DeltaResult
     │  cp_result_free()
     ▼
  Ok(DeltaResult)  or  Err(errno) on catastrophic failure only
     │
     │  cp_vfs_checksum(vfs)  →  semantic yield check
     │
     │  vfs_reset_to_snapshot(vfs)
     ▼
  baseline VFS state  (restored for next iteration)
```

The Rust layer never touches the C wire format.  `apply_delta()` translates a
`FsDelta` into a C `fs_delta_t` via `delta_add_*` convenience calls, passes it
to `cp_apply_delta`, reads the `cp_result_t` fields directly (transparent
`#[repr(C)]` layout), and frees both.  This means:

- No serialization round-trip on the hot path.
- Rust owns the corpus (`FsDelta`); C owns the live VFS.
- Per-op failures are surfaced as `DeltaResult.failed`, not as `Err`.
- The C control plane is unchanged from Week 4.

---

## File Map

| Path | Purpose |
|---|---|
| `mutator/src/lib.rs` | Crate root; `pub mod` declarations |
| `mutator/src/delta.rs` | `FsOpKind`, `FsOp` (7 constructors), `FsDelta`, `generate_seed`, `initial_corpus_pool` |
| `mutator/src/guidance.rs` | `MutationGuidance` — FUSE log stub (populated in Phase B) |
| `mutator/src/mutators.rs` | `MAX_OPS`, `PATH_COMPONENTS`, 7 mutator stages, 17 unit tests |
| `mutator/src/ffi.rs` | C type bindings, `DeltaResult`, `apply_delta()`, 5 E2E integration tests |
| `mutator/src/bin/fuzz.rs` | Dumb loop harness with semantic yield tracking |
| `mutator/src/bin/vfs_bench.rs` | Direct VFS benchmark (no FUSE, no mutators) |
| `mutator/build.rs` | Builds `libcontrol_plane.a`, links it into the Rust crate |
| `mutator/Cargo.toml` | `libafl 0.15`, `libafl_bolts 0.15`, `serde`, `libc` |
| `control_plane/delta.h` | Wire format (no magic, conditional timestamps) |
| `control_plane/delta.c` | serialize/deserialize matching new format |

---

## `delta.rs` — Corpus Types

### `FsOpKind`

```rust
pub enum FsOpKind {
    CreateFile,   // create file with content
    UpdateFile,   // replace entire file content
    DeleteFile,   // unlink file
    Mkdir,        // create directory
    Rmdir,        // remove empty directory
    SetTimes,     // set mtime and/or atime
    Truncate,     // resize file
}
```

Maps 1:1 to `fs_op_kind_t` in `control_plane/delta.h`.

### `FsOp`

```rust
pub struct FsOp {
    pub kind:       FsOpKind,
    pub path:       String,    // absolute, starts with '/'
    pub content:    Vec<u8>,   // CreateFile / UpdateFile only; empty otherwise
    pub size:       usize,     // content.len() for file ops; new_size for Truncate; 0 otherwise
    pub mtime_sec:  i64,       // SetTimes only; 0 otherwise
    pub mtime_nsec: i64,
    pub atime_sec:  i64,
    pub atime_nsec: i64,
}
```

All seven constructors:

| Constructor | Kind | Notable |
|---|---|---|
| `create_file(path, content)` | `CreateFile` | sets `size = content.len()` |
| `update_file(path, content)` | `UpdateFile` | sets `size = content.len()` |
| `delete_file(path)` | `DeleteFile` | content/size zero |
| `mkdir(path)` | `Mkdir` | content/size zero |
| `rmdir(path)` | `Rmdir` | content/size zero |
| `truncate(path, new_size)` | `Truncate` | `size = new_size`, no content |
| `set_times(path, mtime_sec, mtime_nsec, atime_sec, atime_nsec)` | `SetTimes` | all four timestamp fields set |

### `FsDelta`

```rust
pub struct FsDelta { pub ops: Vec<FsOp> }
```

Implements `libafl::inputs::Input` via `generate_name()` →
`"delta_{id}_ops{n}"`.  Derives `Clone`, `Debug`, `Hash`, `Serialize`,
`Deserialize`.

### Seed Helpers

```rust
pub fn generate_seed() -> FsDelta
// → FsDelta [ CreateFile("/input", b"seed") ]

pub fn initial_corpus_pool() -> Vec<FsDelta>
// → 3 structurally diverse deltas used by SpliceDelta before a real
//   corpus is accumulated (Phase A only)
```

---

## `guidance.rs` — Mutation Guidance Stub

```rust
pub struct MutationGuidance {
    pub enoent_paths:   Vec<String>,  // paths target tried to open but ENOENT'd
    pub recreate_paths: Vec<String>,  // paths target deleted or renamed away
}
```

All fields default to empty in Phase A.  `AddFileOp` checks `has_enoent()` and
biases 70% of new path choices toward `enoent_paths` when populated.
Phase B will fill this from the FUSE write log after each target execution.

---

## `mutators.rs` — The Seven Mutator Stages

### Constants

```rust
pub const MAX_OPS: usize = 20;
```

`AddFileOp`, `SpliceDelta`, and `DestructiveMutator` return
`MutationResult::Skipped` rather than grow the delta past this limit.
Prevents unbounded op accumulation across corpus entries.

### Path vocabulary

```rust
static PATH_COMPONENTS: &[&str] = &[
    "a", "b", "c", "d",
    "etc", "tmp", "var", "lib", "usr",
    "input", "output", "config", "data", "test", "run",
];
```

`random_path()` assembles 1–3 components into an absolute path.
`random_content()` generates 1–64 random bytes.

### All seven implement

```rust
impl<S: HasRand> Mutator<FsDelta, S> for $Stage
impl Named for $Stage   // → static &'static str name
```

---

### 1. `ByteFlipFileContent`

Picks a random `CreateFile` or `UpdateFile` op with non-empty content; XORs
1–4 randomly chosen bytes with a random single-bit mask.  Skips if no file
content op exists.

**Why**: off-by-one / bit-error mutations without changing file size.

---

### 2. `ReplaceFileContent`

Picks a random `CreateFile` or `UpdateFile` op; replaces its entire content
with 1–64 fresh random bytes.  Keeps `op.size` in sync with the new length.
Skips if no file content op exists.

**Why**: structurally valid but semantically surprising content — exercises
parser error paths.

---

### 3. `AddFileOp`

Appends a new `CreateFile` (70%) or `Mkdir` (30%) op.  Path is chosen from
`PATH_COMPONENTS` unless `MutationGuidance.enoent_paths` is non-empty, in
which case 70% of paths are drawn from there.  Skips if
`ops.len() >= MAX_OPS`.

**Why**: creates files at paths the target tried to access, maximising the
chance of reaching previously uncovered code.

---

### 4. `RemoveOp`

Removes one randomly chosen op.  Skips if `ops.len() <= 1` (empty delta is
invalid).

**Why**: produces minimal reproducing inputs; counteracts the growth from
`AddFileOp` and `SpliceDelta`.

---

### 5. `MutatePath`

Replaces one component of the path of a randomly chosen op with a different
component from `PATH_COMPONENTS`.  Skips if the delta is empty.

**Why**: explores path-dependent behaviour without changing the op's semantic
type.

---

### 6. `SpliceDelta`

Takes a random prefix of ops from a donor delta in `corpus_pool` and appends
them to the current delta, capped to `MAX_OPS` total.  Skips if the pool is
empty or the delta is already at the cap.

**Why**: AFL-style splice; recombines structural diversity from the corpus.
In Phase A the pool is the three hard-coded deltas from
`initial_corpus_pool()`; in Phase B it draws from the live LibAFL corpus.

---

### 7. `DestructiveMutator`

Appends one destructive or metadata op with a random path, chosen uniformly
from:

| Choice | Op | Detail |
|---|---|---|
| 0 | `DeleteFile(path)` | path is random |
| 1 | `Rmdir(path)` | path is random |
| 2 | `Truncate(path, size)` | `size` ∈ [0, 1023] |
| 3 | `SetTimes(path, mtime, 0, atime, 0)` | random UNIX timestamps |

Skips if `ops.len() >= MAX_OPS`.

**Why**: the other six stages never generate `DeleteFile`, `Rmdir`,
`Truncate`, or `SetTimes` as primary ops.  `DestructiveMutator` ensures every
`FsOpKind` variant is reachable through the mutation pipeline.

---

## `ffi.rs` — FFI Bridge

### C type bindings

| Rust type | C type | Layout |
|---|---|---|
| `VfsT` | `vfs_t` | opaque (`[u8; 0]`) |
| `FsDeltaC` | `fs_delta_t` | opaque (`[u8; 0]`) |
| `CpOpResultT` | `cp_op_result_t` | transparent `#[repr(C)]` |
| `CpResultT` | `cp_result_t` | transparent `#[repr(C)]` |

`CpResultT` fields (must stay in sync with `control_plane/control_plane.h`):

```rust
pub struct CpResultT {
    pub total_ops: c_int,
    pub succeeded: c_int,
    pub failed:    c_int,
    pub results:   *mut CpOpResultT,  // array[total_ops]
}
```

### All `extern "C"` bindings

**VFS lifecycle:**

| Binding | Signature |
|---|---|
| `vfs_create` | `() -> *mut VfsT` |
| `vfs_destroy` | `(*mut VfsT)` |
| `vfs_save_snapshot` | `(*mut VfsT) -> c_int` |
| `vfs_reset_to_snapshot` | `(*mut VfsT) -> c_int` |
| `vfs_create_file` | `(*mut VfsT, path, content, len) -> c_int` |
| `vfs_mkdir` | `(*mut VfsT, path) -> c_int` |

**Delta construction:**

| Binding | FsOpKind |
|---|---|
| `delta_create` / `delta_free` | — |
| `delta_add_create_file(d, path, content, len)` | `CreateFile` |
| `delta_add_update_file(d, path, content, len)` | `UpdateFile` |
| `delta_add_delete_file(d, path)` | `DeleteFile` |
| `delta_add_mkdir(d, path)` | `Mkdir` |
| `delta_add_rmdir(d, path)` | `Rmdir` |
| `delta_add_set_times(d, path, *mtime, *atime)` | `SetTimes` |
| `delta_add_truncate(d, path, new_size)` | `Truncate` |

All 7 `FsOpKind` variants are covered; no op kind is silently dropped.

**Control plane:**

| Binding | Purpose |
|---|---|
| `cp_apply_delta(vfs, delta, dry_run) -> *mut CpResultT` | apply and get per-op results |
| `cp_result_free(r)` | free the result struct |
| `cp_vfs_checksum(vfs) -> u64` | FNV-1a hash of VFS tree for yield tracking |

### `DeltaResult`

```rust
pub struct DeltaResult {
    pub succeeded: usize,
    pub failed:    usize,
}
impl DeltaResult {
    pub fn all_ok(&self) -> bool { self.failed == 0 }
}
```

Filled directly from `CpResultT.succeeded` / `CpResultT.failed` after each
`cp_apply_delta` call.

### `validate_delta` (debug builds only)

```
debug_assert: ops non-empty
debug_assert: every path starts with '/'
debug_assert: op.size == op.content.len()  for CreateFile / UpdateFile
```

Called at the top of every `apply_delta` invocation.  Zero overhead in
release builds.

### `apply_delta` semantics

```rust
pub fn apply_delta(vfs: *mut VfsT, delta: &FsDelta) -> Result<DeltaResult, i32>
```

| Return | Meaning |
|---|---|
| `Ok(dr)` with `dr.failed == 0` | all ops succeeded at VFS level |
| `Ok(dr)` with `dr.failed > 0` | some ops failed at VFS level (ENOENT, EEXIST, …) — normal fuzzer noise |
| `Err(-ENOMEM)` | OOM allocating the C delta or null from `cp_apply_delta` |
| `Err(-EINVAL)` | interior NUL byte in an op path |
| `Err(-errno)` | `delta_add_*` returned a non-zero error during construction |

The `SetTimes` path constructs two `libc::timespec` values from the `FsOp`
timestamp fields and passes them by pointer to `delta_add_set_times`.

---

## C Serialization Cleanup

The Week 4 wire format included a 4-byte magic number at the start of every
buffer and a fixed 32-byte timestamp block after every op (even non-`SET_TIMES`
ops).  With semantic Rust mutators the raw wire format is never byte-flipped;
it is only used for checksum tagging and potential cross-process transfer.

**Changes made:**

| Before | After |
|---|---|
| Header: `magic(4) \| n_ops(4)` = 8 bytes | Header: `n_ops(4)` = 4 bytes |
| Per op: always 32 bytes of timestamps | Per op: `has_ts(1)` flag; 32 bytes only if `has_ts == 1` |
| `DELTA_OP_FIXED = 43` | `DELTA_OP_FIXED = 12` |
| `test_rejection_rate` (10 000 random byte-flip trials) | Removed (superseded by semantic mutators) |

All 223 C-side checks still pass after the format change.

---

## Dumb Loop Harness (`fuzz.rs`)

Validates the full mutation → apply → reset cycle without FUSE or corpus
management.  Uses a `DumbState` struct that satisfies `HasRand` (wraps
`StdRand`) as the minimal LibAFL state needed by the mutators.

**Loop steps:**

1. Create VFS; populate baseline (`/input`, `/etc/`, `/etc/config`).
2. Save snapshot; record `baseline_checksum = cp_vfs_checksum(vfs)`.
3. For each iteration:
   a. Clone seed delta (`CreateFile("/input", b"seed")`).
   b. Pick one of the 7 mutators at random; call `mutate(state, &mut delta)`.
   c. Call `apply_delta(vfs, &delta)` → `Result<DeltaResult, i32>`.
   d. Call `cp_vfs_checksum(vfs)`; if ≠ `baseline_checksum` → semantic yield.
   e. Time `vfs_reset_to_snapshot(vfs)`; accumulate reset cost statistics.
4. `assert!(n_reset_err == 0)` — any reset failure is a stale-state bug.
5. Print summary.

**Counters reported:**

| Counter | Meaning |
|---|---|
| `apply ok` | `apply_delta` returned `Ok` |
| `apply partial` | `Ok` but `result.failed > 0` (some ops rejected at VFS level) |
| `apply err` | catastrophic `Err` (OOM / null pointer) |
| `reset ok / err` | `vfs_reset_to_snapshot` outcome |
| `semantic yield` | iterations where VFS checksum changed after apply |

Usage: `cargo run --release --bin fuzz -- [iterations]` (default 50).

**Sample run (100 iterations):**

| Metric | Value |
|---|---|
| apply ok | 100/100 |
| apply partial | 84/100 |
| reset ok | 100/100 |
| semantic yield | 39/100 (39.0%) |
| reset mean | 338 ns |
| reset max | 2 440 ns |

The high partial count is expected: `DestructiveMutator` frequently targets
paths that don't exist in the baseline tree (e.g. `DeleteFile /var/lib`),
which fails at VFS level but is correctly counted, not panicked.

---

## Direct VFS Benchmark (`vfs_bench.rs`)

Measures the raw cost of `vfs_reset_to_snapshot` and `apply_delta` with no
FUSE layer, mutator, or target execution overhead.

Usage: `cargo run --release --bin vfs_bench -- [iterations]` (default 1000).

**Baseline tree:** `/input` (4 B), `/etc/`, `/etc/config` (20 B),
`/data/`, `/data/a.bin` (4 B).

Deltas used:
- **small** — 1 op: `UpdateFile("/input", 21 B)`
- **medium** — 3 ops: update + create + mkdir
- **large** — 10 ops: 8 new files + update + mkdir

See [`benchmark_baseline.md`](./benchmark_baseline.md) §"Week 5" for full
tables.  Key figures (2000-iter, `--release`):

| Scenario | Combined mean |
|---|---|
| reset only (baseline tree) | 321 ns |
| apply (1 op) + reset | ~504 ns |
| apply (3 ops) + reset | ~1.1 µs |
| apply (10 ops) + reset | ~4.3 µs |

At 1 µs/iter the VFS contributes **< 0.1%** of a typical fuzzing iteration
once target execution (milliseconds) is included.

---

## Unit & Integration Tests

Run with `cargo test`.  **22 tests, 0 failures.**

### `mutators::tests` — 17 structural unit tests

| Test | Verifies |
|---|---|
| `byte_flip_mutates_content` | content changes, returns `Mutated` |
| `byte_flip_skips_when_no_file_ops` | `Skipped` on mkdir-only delta |
| `replace_file_content_changes_content_and_size` | `op.size == op.content.len()` after replace |
| `replace_file_content_skips_when_no_file_ops` | `Skipped` on rmdir-only delta |
| `add_file_op_grows_delta` | op count +1; new op is `CreateFile` or `Mkdir`; path starts with `/` |
| `add_file_op_uses_guidance_enoent_paths` | guided path chosen within 50 tries (70% bias) |
| `add_file_op_skips_at_max_ops` | `Skipped` when `ops.len() == MAX_OPS` |
| `remove_op_shrinks_delta` | op count −1 |
| `remove_op_skips_single_op_delta` | `Skipped`; delta unchanged |
| `mutate_path_changes_a_component` | path changes within 20 tries; stays absolute |
| `mutate_path_skips_empty_delta` | `Skipped` |
| `splice_delta_appends_ops_from_donor` | op count grows by ≥ 1 |
| `splice_delta_skips_empty_pool` | `Skipped` |
| `splice_delta_skips_at_max_ops` | `Skipped` when `ops.len() == MAX_OPS` |
| `destructive_mutator_grows_delta` | op count +1; new op is `DeleteFile\|Rmdir\|Truncate\|SetTimes` |
| `destructive_mutator_generates_all_four_kinds` | all four destructive kinds seen within 200 tries |
| `destructive_mutator_skips_at_max_ops` | `Skipped` when `ops.len() == MAX_OPS` |

### `ffi::tests` — 5 E2E integration tests

These build a real VFS, call `apply_delta()` through the full FFI bridge, and
assert on VFS-level outcomes — not just struct mutation.

| Test | Verifies |
|---|---|
| `e2e_create_file_succeeds` | `succeeded > 0`, `failed == 0` for a fresh `CreateFile` |
| `e2e_mkdir_and_create_file_succeeds` | `succeeded == 2`, `failed == 0` for a 2-op delta |
| `e2e_update_existing_file_succeeds` | `UpdateFile` on a baseline file returns `succeeded == 1` |
| `e2e_set_times_on_existing_file_succeeds` | `SetTimes` FFI path reaches the VFS and succeeds |
| `e2e_failed_op_is_counted_not_panicked` | `DeleteFile` on non-existent path → `Ok` with `failed == 1`; no panic |

The `e2e_set_times` test confirms the SetTimes FFI bridge is wired end-to-end
(the old code silently returned 0 and never called `delta_add_set_times`).

---

## What Phase B Will Add

- `vfs/fvfs.c`: log writes in `fvfs_write`, `fvfs_create`, `fvfs_mkdir`,
  `fvfs_unlink`, `fvfs_rename` behind a `g_target_running` flag.
- `control_plane/`: `fuse_iter_log_t` struct, `cp_collect_log()` to drain it.
- `mutators.rs`: populate `MutationGuidance.enoent_paths` and
  `recreate_paths` from the collected log before each mutation.
- `fuzz.rs`: replace dumb loop with a real LibAFL `StdFuzzer` loop that
  forks the target through the FUSE mount.

The `MutationGuidance` interface is already in place; Phase B is pure wiring.
The semantic yield metric from the dumb loop will serve as a baseline to
confirm guidance improves the yield rate once FUSE logging is connected.
