# Week 5 - Mutator and Dumb Loop (Phase A)

## Overview

Week 5 built the Rust-side fuzzing layer that sits between LibAFL and the
in-memory VFS.  The deliverable is a working, measurable mutation → apply →
reset loop that exercises all eight mutation strategies across all seven
`FsOpKind` variants, with per-op failure reporting, semantic yield tracking,
baseline-path targeting, and insertion-order-independent checksums.

The week is split into two phases:

- **Phase A (complete)** — Rust `FsDelta`/`FsOp` corpus types, eight mutator
  stages (including `UpdateExistingFile` that targets real baseline files),
  full FFI bridge with per-op result inspection, `MAX_OPS` cap,
  `validate_delta` debug assertions, baseline path enumeration via
  `cp_enumerate_paths` (files / dirs / all), sorted `cp_vfs_checksum`
  (insertion-order independent), 7-seed diverse corpus, multi-mutation per
  iteration (1–3 mutations on the same delta before VFS apply), op-type-aware
  path selection in `DestructiveMutator`, `MutatePath` whole-path swap,
  `SpliceDelta` random start offset, **live corpus with novel-checksum
  promotion** (`SpliceDelta` draws from the same pool, bounded at 128 entries
  with random eviction of non-seed slots), **content dictionary** (trigger
  strings, magic bytes, boundary sizes — 40% of `ReplaceFileContent`
  outputs), **real-content perturbation** in `UpdateExistingFile` (bit-flip /
  append / truncate / dictionary-splice of the live baseline content),
  **guidance threading** through `MutatePath` (enoent-path preference in
  whole-swap) and `DestructiveMutator` (recreate-path bias on `DeleteFile` /
  `Rmdir`), **skip-early stage filtering** via `can_apply` precondition
  checks (no wasted mutation-budget slots on guaranteed skips), dumb loop
  harness with 98% semantic yield, 40 unit + E2E integration tests, C
  serialization cleanup, benchmarks.
- **Phase B** — FUSE callback logging, `MutationGuidance` population
  from the write log, full closed loop.  The guidance hooks are already in
  place in Phase A; Phase B only adds the producer side.

---

## Generator-First Walkthrough

The easiest way to understand Week 5 Phase A is to follow one input from birth
to execution:

```text
build live_corpus = generate_seed_corpus(baseline_file_paths) + initial_corpus_pool()
  -> pick a random seed from the LIVE corpus each iteration
  -> apply 1–3 random mutations from the applicable-mutator subset
     (skip-early filtering via can_apply precondition check)
  -> apply_delta(vfs, &delta)
  -> count per-op VFS successes/failures
  -> compare VFS checksum with the baseline
  -> if the post-apply checksum is novel, promote the delta into live_corpus
     (bounded at MAX_LIVE_CORPUS = 128 with random non-seed eviction)
  -> reset VFS back to the saved snapshot
```

SpliceDelta draws its donor pool from the same `live_corpus`, so promoted
deltas become splice donors on the next iteration — this is the "dumb
fuzzer" version of corpus evolution.

The fuzzer does **semantic mutation**.  It does not byte-flip a serialized C
buffer.  The thing being mutated is a Rust `FsDelta`, which is just a list of
filesystem operations:

```text
FsDelta {
  ops: [
    FsOp,
    FsOp,
    ...
  ]
}
```

A delta can be read like a mini filesystem script:

```text
[
  CreateFile("/input", "seed"),
  Mkdir("/tmp"),
  CreateFile("/tmp/config", "mode=fast\n"),
  Truncate("/input", 2),
]
```

That semantic shape is the whole point of the Week 5 mutator.  It lets the
fuzzer make meaningful changes such as "replace this file's content", "move
this op to another path", or "append a truncate op" while keeping the input
structurally valid.

### 1. Seed Corpus: `generate_seed_corpus()`

Instead of a single hard-coded seed, Phase A builds a 7-family corpus at
startup from the paths actually present in the baseline VFS:

```rust
pub fn generate_seed_corpus(baseline_files: &[String]) -> Vec<FsDelta> {
    let primary   = baseline_files.first()...;   // "/input"
    let secondary = baseline_files.get(1)...;    // "/etc/config"
    vec![
        // 1. UpdateFile primary — ByteFlip/Replace have a live target
        FsDelta::new(vec![FsOp::update_file(primary, b"seed".to_vec())]),
        // 2. UpdateFile secondary (config the target reads)
        FsDelta::new(vec![FsOp::update_file(secondary, b"[settings]\nverbose=1\n".to_vec())]),
        // 3. Truncate primary — exercises size-change paths
        FsDelta::new(vec![FsOp::truncate(primary, 2)]),
        // 4. Touch timestamps — exercises metadata-only code paths
        FsDelta::new(vec![FsOp::set_times(primary, 1_700_000_000, 0, 1_700_000_000, 0)]),
        // 5. Multi-op: update content + metadata in one delta
        FsDelta::new(vec![
            FsOp::update_file(primary, b"fuzzed content".to_vec()),
            FsOp::set_times(primary, 1_000_000_000, 0, 1_000_000_000, 0),
        ]),
        // 6. CreateFile at a fresh path (no EEXIST risk)
        FsDelta::new(vec![FsOp::create_file("/fuzz_input", b"new".to_vec())]),
        // 7. Directory + child creation sequence
        FsDelta::new(vec![
            FsOp::mkdir("/fuzz_dir"),
            FsOp::create_file("/fuzz_dir/file", b"hello".to_vec()),
        ]),
    ]
}
```

Key invariant: seed family 1 uses `UpdateFile`, not `CreateFile`, because
`/input` already exists in the baseline.  A `CreateFile` on an existing path
always fails with EEXIST, making `ByteFlipFileContent` and `ReplaceFileContent`
operate on a dead op — the single biggest correctness bug in the original design.

### 2. Dumb Loop: Live Corpus + Multi-Mutation Per Iteration

`mutator/src/bin/fuzz.rs` builds the **live corpus** once at startup from
the seed families and the splice donor pool, then both the per-iteration
seed selection and the `SpliceDelta` mutator draw from the same shared
`Rc<RefCell<Vec<FsDelta>>>`:

```rust
let mut initial: Vec<FsDelta> = generate_seed_corpus(&baseline_file_paths);
let seed_count = initial.len();                     // preserve for eviction
initial.extend(initial_corpus_pool());
let live_corpus: LiveCorpus = Rc::new(RefCell::new(initial));
```

Then every iteration does:

```rust
// 1. Pick a starting delta from the live corpus (seeds + promoted deltas).
let (seed_idx, mut delta) = {
    let corpus = live_corpus.borrow();
    let idx    = state.rand_mut().below(nz(corpus.len()));
    (idx, corpus[idx].clone())
};

// 2. Apply 1–3 mutations, each drawn from the subset that can_apply.
let n_mut = 1 + state.rand_mut().below(nz(3));
for _ in 0..n_mut {
    let applicable: Vec<usize> = (0..mutators.len())
        .filter(|&k| mutators[k].can_apply(&delta))
        .collect();
    let m_idx = applicable[state.rand_mut().below(nz(applicable.len()))];
    mutators[m_idx].mutate(&mut state, &mut delta);
}

// 3. Apply + reset; if post-apply checksum is novel, promote.
if post_checksum != baseline_checksum && seen_checksums.insert(post_checksum) {
    let mut corpus = live_corpus.borrow_mut();
    if corpus.len() < MAX_LIVE_CORPUS { corpus.push(delta.clone()) }
    else {
        let victim = seed_count + state.rand_mut().below(nz(corpus.len() - seed_count));
        corpus[victim] = delta.clone();   // evict random non-seed slot
    }
}
```

Three things this combines:

- **Live corpus evolution.** Novel-checksum deltas become seeds and splice
  donors in subsequent iterations.  The `seen_checksums: HashSet<u64>` gate
  rejects duplicates so the corpus fills with diverse inputs.  Once the cap
  (`MAX_LIVE_CORPUS = 128`) is hit, promotions evict a random *non-seed*
  entry — the original 7 seed families are never evicted, so structural
  diversity survives corpus churn.
- **Multi-mutation per iteration.** 1–3 mutations on the same delta lets
  `RemoveOp` shrink an earlier-`AddFileOp` growth in the same iteration, and
  lets `AddFileOp → ByteFlipFileContent` chain into a new-op + content-flip
  in one pass.
- **Skip-early filtering.** Before each mutation we compute the applicable
  subset via `can_apply(&delta)` (e.g. `RemoveOp.can_apply` iff `ops.len()>1`;
  `ByteFlipFileContent.can_apply` iff there is a non-empty file op).  This
  avoids burning a mutation-budget slot on a guaranteed `Skipped` — the
  harness's "iters skipped" count drops to zero in practice.

The VFS is reset to the same saved baseline after every iteration:

```text
/input  /etc/  /etc/config
```

### 3. Mutator Pool

The dumb loop randomly chooses one of the eight stages:

| Stage | High-level behavior |
|---|---|
| `ByteFlipFileContent` | Flip 1 to 4 bits inside existing file content |
| `ReplaceFileContent` | Replace an entire file content buffer |
| `AddFileOp` | Append a `CreateFile` or `Mkdir` op |
| `RemoveOp` | Remove one op, unless that would make the delta empty |
| `MutatePath` | Replace one path component |
| `SpliceDelta` | Append a prefix from a donor delta |
| `DestructiveMutator` | Append delete/rmdir/truncate/set-times ops; targets real baseline paths |
| `UpdateExistingFile` | Append `UpdateFile` on a file known to exist in the baseline |

Each stage returns:

| Return | Meaning |
|---|---|
| `Mutated` | The Rust `FsDelta` changed |
| `Skipped` | The stage could not safely apply |
| `Err` | Unexpected LibAFL-level error |

`Skipped` is a normal outcome.  For example, `RemoveOp` skips on the generator
seed because removing the only op would create an invalid empty delta.

### 4. Concrete Mutator Examples From the Seed

All examples start from the primary seed family:

```text
D0 = [ UpdateFile("/input", "seed", size=4) ]
```

#### `ByteFlipFileContent`

Flips bits in file content without changing path or size:

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After:  [ UpdateFile("/input", "semd", size=4) ]
```

The real bytes may be non-printable.  The important invariant is that the
content length stays 4.

#### `ReplaceFileContent`

Replaces the whole content buffer and updates `size`.  With 40% probability
draws from `CONTENT_DICTIONARY` (trigger strings, magic bytes, boundary
sizes); otherwise generates 1–64 random bytes:

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After (random):     [ UpdateFile("/input", [de ad be ef 00], size=5) ]
After (dictionary): [ UpdateFile("/input", "foobar", size=6) ]
After (dictionary): [ UpdateFile("/input", [0xAA; 64], size=64) ]
```

The dictionary carries values that are structurally interesting to parsers
(magic numbers, path-traversal markers, format strings, long fill patterns
sized at 64B / 256B / 4KB for boundary behaviour) as well as the Week 6
demo trigger string `"foobar"`.

#### `AddFileOp`

Appends a new file or directory:

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After:  [
  UpdateFile("/input", "seed", size=4),
  Mkdir("/tmp/run")
]
```

When `MutationGuidance.enoent_paths` is populated (Phase B), 70% of new
paths are drawn from there with a 90% file bias (vs 70% for random paths)
because the target tried to *open* a file at that path, not create a directory.

#### `RemoveOp`

Starting from the raw seed, it skips:

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After:  Skipped, because len <= 1
```

Within a multi-mutation chain, it can shrink a delta grown earlier in the
same iteration:

```text
Mutation 1 (AddFileOp): [ UpdateFile("/input", ...), Mkdir("/tmp/run") ]
Mutation 2 (RemoveOp):  [ Mkdir("/tmp/run") ]
```

This counterbalances `AddFileOp` and `SpliceDelta`.

#### `MutatePath`

Two modes, selected randomly:

- **Whole-path swap** (30% when any target-pool is non-empty): replaces the
  entire path with a known-interesting path.  Preference order:
  `guidance.enoent_paths` → `guidance.recreate_paths` → `baseline_paths`.
  In Phase A the guidance lists are empty so this always draws from
  `baseline_paths`; in Phase B the ENOENT paths the target actually tried to
  open take precedence, and the mutator immediately converts failing random
  paths into paths the target is known to care about.
- **Component swap** (otherwise): replaces one segment with a `PATH_COMPONENTS`
  word, exploring the neighbourhood of the current path.

```text
Before: [ UpdateFile("/random/path", ...) ]
After (whole swap, baseline):   [ UpdateFile("/etc/config", ...) ]       ← known good
After (whole swap, ENOENT):     [ UpdateFile("/wanted/by/target", ...) ] ← Phase B signal
After (component swap):         [ UpdateFile("/random/config", ...) ]    ← neighbour
```

#### `SpliceDelta`

Picks a **random start offset** in a donor delta drawn from the **live
corpus** (not a fixed pool — the same `Rc<RefCell<Vec<FsDelta>>>` the
harness uses for seed selection).  Appends a contiguous slice of ops:

```text
Donor: [
  UpdateFile("/etc/config", "verbose=1"),  ← index 0
  SetTimes("/input", 1700000000, 0, ...),  ← index 1
  Truncate("/input", 2),                   ← index 2
]
start = 1  →  slice = [ SetTimes(...), Truncate(...) ]
```

Possible output:

```text
[
  UpdateFile("/input", "seed"),
  SetTimes("/input", 1700000000, 0, ...),
  Truncate("/input", 2),
]
```

The random offset means late-donor ops (metadata ops at the tail of a
sequence) are reachable independently, not only when the entire prefix is
also spliced.  `cp_ensure_parents` makes any slice structurally safe.  And
because the donor pool is the *live* corpus, any delta the harness promotes
(novel-checksum yield) becomes available as a splice donor on the next
iteration — the splice distribution shifts as the corpus evolves.

#### `DestructiveMutator`

Appends one destructive or metadata op using **op-type-aware path selection**:

| Op | Path drawn from |
|---|---|
| `DeleteFile` | `baseline_file_paths` (files only) — 70% bias |
| `Rmdir` | `baseline_dir_paths` (dirs only) — 70% bias |
| `Truncate` | `baseline_file_paths` (files only) — 70% bias |
| `SetTimes` | `baseline_all_paths` (any node) — 70% bias |

Drawing file paths for `Rmdir` would always produce ENOTDIR; drawing dir
paths for `Truncate` would always produce EISDIR.  The three separate lists
ensure each op gets semantically correct path candidates.

`SetTimes` timestamps use `pick_timestamp()`, which draws from a set of
interesting edge cases 40% of the time:
- `0` (epoch), `-1` (pre-epoch), `i32::MAX` (2038 boundary),
  `2_000_000_000` (post-2038 far future), `1_700_000_000` (~Nov 2023).

**Guidance bias.** `DeleteFile` and `Rmdir` both check
`guidance.recreate_paths` first and, with 50% probability when populated,
draw from there — the target has already shown it acts on these paths, so
re-deleting them exercises the same code path again.  Phase A runs with an
empty recreate list; Phase B feeds this from the FUSE `UNLINK` /
`RENAME_FROM` log.

```text
[
  UpdateFile("/input", "seed"),
  Truncate("/input", new_size=2),        ← file path used, not /etc
]
```

#### `UpdateExistingFile`

Appends an `UpdateFile` op on a path drawn exclusively from
`baseline_file_paths`.  Content selection follows a three-way strategy:

| Strategy | Probability | Behaviour |
|---|---|---|
| **Real-content perturbation** | 50% when baseline content is available | read the live baseline content for the chosen path; apply one of bit-flip / append / truncate / dictionary-splice |
| **Dictionary draw** | 30% otherwise | pick an entry from `CONTENT_DICTIONARY` |
| **Random bytes** | 70% otherwise | 1–64 uniform random bytes |

This is the highest-value mutation for reaching deep parser state:
targets that read structured content (e.g. `/etc/config`) keep most of the
structure intact under perturbation and reach downstream logic that random
bytes would never reach.  Constructor chain:

```rust
UpdateExistingFile::new(baseline_file_paths)
    .with_baseline_contents(vec![
        ("/input".into(),      b"seed".to_vec()),
        ("/etc/config".into(), b"[settings]\nverbose=0\n".to_vec()),
    ])
```

Example perturbations from the `/etc/config` baseline
`b"[settings]\nverbose=0\n"` (20 bytes):

```text
bit-flip:           "[settings]\nvdrbose=0\n"         (20 B, 1 bit differs)
append:             "[settings]\nverbose=0\n\x3a\xf1" (22 B)
truncate:           "[settin"                         (7 B)
dictionary-splice:  "[settings]\nverbose=0\nfoobar"   (26 B, "foobar" inserted)
```

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After:  [
  UpdateFile("/input",      "seed", size=4),
  UpdateFile("/etc/config", "[settings]\nverbose=1\n", size=21)   ← perturb
]
```

Skips when `baseline_file_paths` is empty or the delta is at `MAX_OPS`.

### 5. Apply, Yield, Reset

After mutation, the loop calls:

```rust
apply_delta(vfs, &delta)
```

`apply_delta()` converts each Rust op into the matching C `delta_add_*` call,
then calls `cp_apply_delta(vfs, c_delta, dry_run=0)`.

Important distinction:

| Result | Meaning |
|---|---|
| `Ok(DeltaResult { failed: 0, ... })` | bridge worked and every VFS op succeeded |
| `Ok(DeltaResult { failed: n, ... })` | bridge worked, but `n` individual VFS ops failed |
| `Err(errno)` | catastrophic bridge/control-plane failure |

Per-op failure is normal fuzzing behavior.  For example:

```text
DeleteFile("/does_not_exist.txt")
```

should be counted as a failed op, not treated as a harness crash.

Then the loop compares:

```rust
cp_vfs_checksum(vfs)
```

against the baseline checksum.  If the checksum changed, the iteration had
**semantic yield**.  This is stronger than "the mutator returned `Mutated`"
because a structurally mutated delta can still fail all VFS operations and
leave the filesystem unchanged.

Finally:

```rust
vfs_reset_to_snapshot(vfs)
```

restores the baseline before the next iteration.

---

## Architecture

```
  VFS baseline  (after populate_baseline + vfs_save_snapshot)
     │
     │  cp_enumerate_paths(vfs, filter=1)  →  baseline_file_paths
     │  cp_enumerate_paths(vfs, filter=2)  →  baseline_dir_paths
     │  cp_enumerate_paths(vfs, filter=0)  →  baseline_all_paths
     │  known at populate time              →  baseline_contents (path, bytes)
     │
     │  live_corpus = generate_seed_corpus(…) + initial_corpus_pool()
     │      wrapped in Rc<RefCell<Vec<FsDelta>>>; shared with SpliceDelta
     │      bounded at MAX_LIVE_CORPUS = 128
     ▼
  per iteration
     │  ► pick a starting delta from live_corpus
     │  ► pick 1–3 mutators from the can_apply subset (skip-early filter)
     │  ► apply each mutation to the delta in sequence
     │
     ▼
  FsDelta  ←── 8 Rust mutator stages (mutators.rs)
     │           MAX_OPS = 20 cap enforced by AddFileOp, SpliceDelta,
     │           DestructiveMutator, UpdateExistingFile
     │           DestructiveMutator: op-type-aware (files/dirs/all, 70% bias);
     │                               50% recreate_paths bias on DeleteFile/Rmdir
     │           UpdateExistingFile: path from baseline_file_paths;
     │                               content = perturb(baseline) 50% / dict 30% / random 70%
     │           MutatePath: 30% whole-path swap (enoent → recreate → baseline)
     │           SpliceDelta: random start offset; donors drawn from live_corpus
     │           ReplaceFileContent: 40% dictionary / 60% random
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
     │    (alphabetically sorted walk — insertion-order independent)
     │
     │  if checksum is novel: push delta → live_corpus
     │                        (evict random non-seed entry when at cap)
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
- Baseline path enumeration is a one-time call at startup; the resulting
  `Vec<String>` is cheaply cloned into mutators that need it.

---

## File Map

| Path | Purpose |
|---|---|
| `mutator/src/lib.rs` | Crate root; `pub mod` declarations |
| `mutator/src/delta.rs` | `FsOpKind`, `FsOp` (7 constructors), `FsDelta`, `generate_seed`, `generate_seed_corpus`, `initial_corpus_pool`, 6 unit tests |
| `mutator/src/guidance.rs` | `MutationGuidance` — FUSE log stub (populated in Phase B) |
| `mutator/src/mutators.rs` | `MAX_OPS`, `MAX_LIVE_CORPUS`, `LiveCorpus`, `CONTENT_DICTIONARY`, `PATH_COMPONENTS`, `pick_or_random`, `pick_timestamp`, `perturb_bytes`, 8 mutator stages (each with `can_apply`), 29 unit tests |
| `mutator/src/ffi.rs` | C type bindings, `DeltaResult`, `apply_delta()`, `enumerate_vfs_file_paths`, `enumerate_vfs_dir_paths`, `enumerate_vfs_all_paths`, 5 E2E integration tests |
| `mutator/src/bin/fuzz.rs` | Dumb loop harness with live-corpus promotion and semantic yield tracking |
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
/// Single minimal seed (still exported; used in some tests).
pub fn generate_seed() -> FsDelta
// → FsDelta [ UpdateFile("/input", b"seed") ]

/// 7-family diverse seed corpus — used by the dumb loop harness.
/// baseline_files is enumerated from the VFS at startup.
pub fn generate_seed_corpus(baseline_files: &[String]) -> Vec<FsDelta>
// → [UpdateFile(primary), UpdateFile(secondary), Truncate, SetTimes,
//    multi-op, CreateFile("/fuzz_input"), mkdir+create sequence]

/// Fixed pool for SpliceDelta before a real corpus is accumulated.
pub fn initial_corpus_pool() -> Vec<FsDelta>
// → 4 structurally diverse deltas (all use UpdateFile on baseline paths;
//   no CreateFile on already-existing paths → no EEXIST)
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
pub const MAX_OPS:         usize = 20;
pub const MAX_LIVE_CORPUS: usize = 128;
pub type LiveCorpus = Rc<RefCell<Vec<crate::delta::FsDelta>>>;
```

- `MAX_OPS` — `AddFileOp`, `SpliceDelta`, `DestructiveMutator`, and
  `UpdateExistingFile` return `MutationResult::Skipped` rather than grow the
  delta past this limit.  Prevents unbounded op accumulation across corpus
  entries.
- `MAX_LIVE_CORPUS` — upper bound on the shared live corpus.  When a novel
  delta would push the pool past the cap, the harness evicts a random
  *non-seed* slot (the first `seed_count` entries are preserved forever).
- `LiveCorpus` — shared-mutable handle to the pool.  `SpliceDelta` stores a
  clone and uses it for donor selection, so promotions show up immediately
  as splice donors.  Phase A is single-threaded so `Rc<RefCell<_>>` is
  sufficient; Phase B will migrate to `Arc<Mutex<_>>` when the harness runs
  LibAFL stages in parallel.

### Content dictionary

```rust
static CONTENT_DICTIONARY: &[&[u8]] = &[ ... ];
```

A static slice of 15 interesting byte sequences shared by `ReplaceFileContent`
(40% draw) and `UpdateExistingFile` / `perturb_bytes` (dictionary-splice mode):

| Category | Examples |
|---|---|
| Week 6 triggers | `b"foobar"` |
| Magic bytes | `b"\x7fELF"`, `b"#!/bin/sh\n"`, `b"%PDF-1.0\n"` |
| Path traversal | `b"../../../etc/passwd"` |
| Format specifiers | `b"%s%s%s%s"`, `b"%n"` |
| Boundary fills | `&[0xAA; 64]`, `&[0x00; 256]`, `&[0x41; 4096]` |
| Edge cases | `b""`, `b"\n"`, `b"\0"`, `b"A"` |

The dictionary is used on both content replacement (wholesale swap) and
real-content perturbation (splice into live baseline bytes).

### `perturb_bytes(rand, base: &[u8]) -> Vec<u8>`

Four equally-likely modes used by `UpdateExistingFile`'s real-content
strategy:

1. **bit-flip** — clone `base`, XOR 1–4 random bytes with a random bit mask.
2. **append** — clone `base`, push 1–32 random bytes at the end.
3. **truncate** — slice `base` to a shorter prefix (≥ 1 byte when possible).
4. **dictionary-splice** — pick a `CONTENT_DICTIONARY` entry; insert it at a
   random offset in `base`.

Handles the empty-base case (returns dictionary entry or random bytes).

### Path vocabulary and helpers

```rust
static PATH_COMPONENTS: &[&str] = &[
    "a", "b", "c", "d",
    "etc", "tmp", "var", "lib", "usr",
    "input", "output", "config", "data", "test", "run",
];
```

`random_path()` assembles 1–3 components into an absolute path.
`random_content()` generates 1–64 random bytes.

```rust
/// Pick from `baseline` with `bias_pct`% probability; fall back to random_path.
fn pick_or_random(rand, baseline: &[String], bias_pct: usize) -> String

/// 40% chance: pick from interesting UNIX timestamps (0, -1, i32::MAX,
/// 2_000_000_000, 1_700_000_000); otherwise random u32 as i64.
fn pick_timestamp(rand) -> i64
```

These helpers are shared across `DestructiveMutator`, `MutatePath`, and
`AddFileOp` to keep the baseline-bias logic in one place.

### All eight implement

```rust
impl<S: HasRand> Mutator<FsDelta, S> for $Stage
impl Named for $Stage   // → static &'static str name
fn can_apply(&self, d: &FsDelta) -> bool   // via AnyMutator wrapper in fuzz.rs
```

Each stage carries a cheap precondition predicate.  The dumb loop computes
the applicable subset before picking the next mutation, so no iteration
budget is wasted on guaranteed-Skip outcomes (e.g. `RemoveOp.can_apply` iff
`ops.len() > 1`; `ByteFlipFileContent.can_apply` iff there is a non-empty
`CreateFile` / `UpdateFile` op).

---

### 1. `ByteFlipFileContent`

Picks a random `CreateFile` or `UpdateFile` op with non-empty content; XORs
1–4 randomly chosen bytes with a random single-bit mask.  Skips if no file
content op exists.

**Why**: off-by-one / bit-error mutations without changing file size.

---

### 2. `ReplaceFileContent`

Picks a random `CreateFile` or `UpdateFile` op; replaces its entire content.
With 40% probability the new content is drawn from `CONTENT_DICTIONARY`
(magic bytes, trigger strings, long fill patterns); otherwise it is 1–64
fresh random bytes.  Keeps `op.size` in sync with the new length.  Skips if
no file content op exists.

**Why**: random bytes alone rarely line up with magic numbers, triggers, or
boundary-sized fills.  The dictionary directly biases the fuzzer toward
structurally interesting inputs (e.g. `"foobar"` — the Week 6 demo crash
trigger — and `[0xAA; 64]` for 64-byte stack-buffer probes) while still
leaving 60% of outputs random so the search doesn't collapse onto the
dictionary.

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

Two modes selected randomly:

- **Whole-path swap** (30% when any target pool is non-empty): replace the
  entire path with a known-interesting path.  Preference order:
  `guidance.enoent_paths` → `guidance.recreate_paths` → `baseline_paths`.
  In Phase A the guidance lists are empty and the swap always draws from
  `baseline_paths`.  Once Phase B wires the FUSE log, the swap prefers paths
  the target actually tried to open but failed with ENOENT — turning
  wasted-op failures into exactly the paths that will reach uncovered code.
- **Component swap** (70%, or always when no pool is populated): replace one
  segment with a `PATH_COMPONENTS` word.

Builder chain:

```rust
MutatePath::new()
    .with_baseline(baseline_all_paths.clone())
    .with_guidance(MutationGuidance::default())
```

Skips if the delta is empty.

**Why**: whole-path swap rescues ENOENT-prone ops; component swap explores
path-space neighbours without abandoning the current path structure.  The
guidance-first preference means a single FUSE log cycle in Phase B will
redirect the swap distribution without changing any mutator internals.

---

### 6. `SpliceDelta`

```rust
pub struct SpliceDelta {
    pub guidance:    MutationGuidance,
    pub corpus_pool: LiveCorpus,   // Rc<RefCell<Vec<FsDelta>>>
}
impl SpliceDelta {
    pub fn new(pool: LiveCorpus) -> Self { ... }
    pub fn new_fixed(pool: Vec<FsDelta>) -> Self { ... }   // legacy test helper
}
```

Picks a random donor from the **live corpus** (same `Rc<RefCell<_>>` the
harness draws seeds from) and a **random start offset** inside the donor,
then appends a contiguous slice of ops capped to `MAX_OPS` total.  Skips
if the pool is empty, the donor is empty, or the delta is already at the
cap.

**Why**: AFL-style splice at the filesystem-operation level.  The random
start offset makes late-donor ops (e.g. a `Truncate` at the tail) reachable
independently of earlier ops in the donor.  Because the pool is the *live*
corpus, any delta the harness promotes (novel-checksum yield) becomes
available as a splice donor on the very next iteration — the splice
distribution evolves with the corpus instead of being frozen at
`initial_corpus_pool()`.

---

### 7. `DestructiveMutator`

Appends one destructive or metadata op, chosen uniformly from:

| Choice | Op | Path pool | Detail |
|---|---|---|---|
| 0 | `DeleteFile(path)` | `baseline_file_paths` | files only |
| 1 | `Rmdir(path)` | `baseline_dir_paths` | dirs only |
| 2 | `Truncate(path, size)` | `baseline_file_paths` | `size` ∈ [0, 1023] |
| 3 | `SetTimes(path, mtime, 0, atime, 0)` | `baseline_all_paths` | any node |

Builder chain:

```rust
DestructiveMutator::new()
    .with_baseline(file_paths, dir_paths, all_paths)
    .with_guidance(MutationGuidance::default())
```

70% of ops draw from the matching baseline list; 30% use a random
`PATH_COMPONENTS` path.  Op-type-aware selection avoids EISDIR on Truncate
and ENOTDIR on Rmdir.

`SetTimes` uses `pick_timestamp()`: 40% interesting edge cases, 60% random.

**Guidance bias.** For `DeleteFile` and `Rmdir` the mutator first checks
`guidance.recreate_paths` (paths the target already acted on — e.g. ones
it unlinked or renamed away during a prior iteration).  When populated, 50%
of ops draw from `recreate_paths` instead of the baseline list.  Phase A
runs with an empty list, so this currently behaves identically to the
baseline path flow; Phase B populates it from the FUSE `UNLINK` /
`RENAME_FROM` log.

Skips if `ops.len() >= MAX_OPS`.

**Why**: the other seven stages never generate `DeleteFile`, `Rmdir`,
`Truncate`, or `SetTimes` as primary ops.  `DestructiveMutator` ensures every
`FsOpKind` variant is reachable through the mutation pipeline, and the
`recreate_paths` bias directs destructive attention at paths the target has
already shown interest in.

---

### 8. `UpdateExistingFile`

Appends an `UpdateFile` op on a path drawn from `baseline_file_paths` (the
set of regular files that exist in the baseline VFS at startup, enumerated via
`enumerate_vfs_file_paths`).

```rust
pub struct UpdateExistingFile {
    pub guidance:            MutationGuidance,
    pub baseline_file_paths: Vec<String>,
    pub baseline_contents:   Vec<(String, Vec<u8>)>,   // (path, bytes)
}
impl UpdateExistingFile {
    pub fn with_baseline_contents(mut self, c) -> Self { ... }
    pub fn has_baseline(&self) -> bool
    fn lookup_baseline<'a>(&'a self, path: &str) -> Option<&'a [u8]>
}
```

Content selection follows a three-way strategy:

| Strategy | Probability | Behaviour |
|---|---|---|
| **Real-content perturbation** | 50% when `lookup_baseline(path)` returns bytes | call `perturb_bytes(rand, &base)` — bit-flip / append / truncate / dictionary-splice of the live baseline content |
| **Dictionary draw** | 30% otherwise | pick an entry from `CONTENT_DICTIONARY` |
| **Random bytes** | 70% otherwise | 1–64 uniform random bytes |

Skips if `baseline_file_paths` is empty or `ops.len() >= MAX_OPS`.

**Why**: `AddFileOp` only creates new files at random paths, so existing
baseline files (`/input`, `/etc/config`) were never mutated in-place.
`UpdateExistingFile` targets exactly these files — those the target process
is guaranteed to read — and guarantees the VFS op succeeds (100% semantic
yield for this stage in practice).  Real-content perturbation is the
highest-value mutation for reaching deep parser state: a config file with
`[settings]\nverbose=0\n` keeps most of its structure when a single bit
flips, so the parser advances past its header check and the mutator
exercises downstream logic that random bytes never reach.

Phase A feeds `baseline_contents` from a hard-coded map that mirrors
`populate_baseline`.  When Phase B adds a `cp_read_file` FFI the list can
be populated from the live VFS after each iteration, which will also catch
any mutation-induced content drift.

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
| `cp_vfs_checksum(vfs) -> u64` | FNV-1a hash of VFS tree (sorted, insertion-order independent) |
| `cp_enumerate_paths(vfs, filter, **paths_out, *n_out) -> c_int` | enumerate VFS paths by kind |
| `cp_enumerate_paths_free(paths, n)` | free the array from `cp_enumerate_paths` |

`cp_enumerate_paths` filter values:

| Value | Returns |
|---|---|
| 0 | all paths (files + directories + symlinks) |
| 1 | regular files only |
| 2 | directories only |

**Safe Rust wrappers:**

```rust
pub fn enumerate_vfs_file_paths(vfs: *mut VfsT) -> Vec<String>  // filter=1
pub fn enumerate_vfs_dir_paths(vfs: *mut VfsT)  -> Vec<String>  // filter=2
pub fn enumerate_vfs_all_paths(vfs: *mut VfsT)  -> Vec<String>  // filter=0
```

All three call `cp_enumerate_paths` with the appropriate filter via the shared
`collect_paths(vfs, filter)` helper, convert the resulting `char **` array to
`Vec<String>`, and free the C allocation.  `enumerate_vfs_dir_paths` is needed
by `DestructiveMutator` to populate `baseline_dir_paths` for Rmdir op-type-aware
path selection.

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

### `cp_vfs_checksum` — Sorted Walk

The checksum implementation was updated to be insertion-order independent.
The old implementation visited children in `vfs_readdir` order (insertion
order), so two VFS instances with the same files created in different op
sequences could produce different hashes.

The new implementation uses a two-pass approach per directory:
1. **Collect** — readdir callback appends every child (name, absolute path, stat) into a flat array.
2. **Sort** — `qsort` alphabetically by entry name.
3. **Hash** — walk the sorted array and mix path, timestamps, and file content into FNV-1a.

This means corpus deduplication in Phase B (which will use checksums to
identify unique VFS states) will not be polluted by duplicate entries that
differ only in creation order.  All 223 C-side tests pass with the new
implementation.

---

## Dumb Loop Harness (`fuzz.rs`)

Validates the full mutation → apply → reset cycle without FUSE or corpus
management.  Uses a `DumbState` struct that satisfies `HasRand` (wraps
`StdRand`) as the minimal LibAFL state needed by the mutators.

**Loop steps:**

1. Create VFS; populate baseline (`/input`, `/etc/`, `/etc/config`).
2. Save snapshot; record `baseline_checksum = cp_vfs_checksum(vfs)`.
3. Call `enumerate_vfs_file_paths`, `enumerate_vfs_dir_paths`, and
   `enumerate_vfs_all_paths`; print path counts.  Build
   `baseline_contents: Vec<(String, Vec<u8>)>` matching the bytes written
   in step 1 (hard-coded in Phase A — a `cp_read_file` FFI can replace this
   in Phase B).
4. Build the live corpus:
   ```rust
   let mut initial = generate_seed_corpus(&baseline_file_paths);
   let seed_count  = initial.len();            // preserved from eviction
   initial.extend(initial_corpus_pool());
   let live_corpus: LiveCorpus = Rc::new(RefCell::new(initial));
   ```
5. Call `build_mutator_pool(baseline_file_paths, baseline_dir_paths,
   baseline_all_paths, live_corpus.clone(), baseline_contents)` — note that
   `SpliceDelta` receives the same `Rc` the harness holds, so promotions
   are immediately visible to it.
6. For each iteration:
   a. Borrow `live_corpus`; pick a random entry; clone it as the starting
      delta.
   b. Apply `n_mut ∈ [1, 3]` mutations.  Before each, compute the
      `applicable` subset via `AnyMutator::can_apply(&delta)` — stages that
      cannot safely apply are filtered out so no budget slot is wasted.
   c. Call `apply_delta(vfs, &delta)` → `Result<DeltaResult, i32>`.
   d. Call `cp_vfs_checksum(vfs)`; if ≠ `baseline_checksum` → semantic yield.
      If the checksum is also novel (`seen_checksums.insert(h)`), push
      `delta.clone()` into `live_corpus` — evicting a random non-seed slot
      (index `≥ seed_count`) if the pool is already at `MAX_LIVE_CORPUS`.
   e. Time `vfs_reset_to_snapshot(vfs)`; accumulate reset cost statistics.
7. `assert!(n_reset_err == 0)` — any reset failure is a stale-state bug.
8. Print summary (including corpus final size and `n_promoted`).

**Counters reported:**

| Counter | Meaning |
|---|---|
| `apply ok` | `apply_delta` returned `Ok` |
| `apply partial` | `Ok` but `result.failed > 0` (some ops rejected at VFS level) |
| `apply err` | catastrophic `Err` (OOM / null pointer) |
| `reset ok / err` | `vfs_reset_to_snapshot` outcome |
| `semantic yield` | iterations where VFS checksum changed after apply |
| `promoted` | novel-checksum deltas added to the live corpus |
| `corpus final` | live corpus size at end of run (≤ `MAX_LIVE_CORPUS`) |

Usage: `cargo run --release --bin fuzz -- [iterations]` (default 50).

**Sample run (200 iterations):**

```
Baseline: 2 file(s), 1 dir(s), 3 total — ["/input", "/etc", "/etc/config"]
Seed corpus: 7 families
Live corpus initial: 11 (7 seeds + 4 splice donors)
```

| Metric | Value |
|---|---|
| apply ok | 200/200 |
| apply partial | ~80/200 (some ops ENOENT/EEXIST at VFS level — normal) |
| reset ok | 200/200 |
| semantic yield | 196/200 (98%) |
| promoted | 192 (novel-checksum deltas added to live corpus) |
| corpus final | 128 (= `MAX_LIVE_CORPUS` — random non-seed eviction active) |
| reset mean | ~350 ns |
| reset max | ~700 ns |

The high yield is a direct result of three things working together: the
corpus starts from `UpdateFile` on existing baseline files (families 1, 2,
5); mutators like `UpdateExistingFile` and `DestructiveMutator` target real
paths; and `can_apply` filtering means the 1–3 mutation budget is never
burned on a guaranteed `Skipped`.  The live corpus saturates at 128 within
the first ~180 iterations, after which promotions evict older non-seed
entries — original seed families are never evicted, so structural
diversity survives.  Partial failures come from random-path ops (e.g.
`AddFileOp` creating a file in a non-existent parent) — expected fuzzer
noise, not bugs.

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

Run with `cargo test`.  **40 tests, 0 failures.**

### `delta::tests` — 6 unit tests

| Test | Verifies |
|---|---|
| `generate_seed_corpus_returns_seven_families` | exactly 7 families produced |
| `generate_seed_corpus_all_deltas_non_empty` | every family has ≥ 1 op |
| `generate_seed_corpus_all_ops_have_absolute_paths` | all op paths start with `/` |
| `generate_seed_corpus_uses_fallback_when_empty` | still produces 7 families with empty input |
| `seed_one_uses_update_not_create` | first family is `UpdateFile` — eliminates EEXIST bug |
| `initial_corpus_pool_has_no_eexist_collision` | no `CreateFile` on a baseline path in any donor |

### `mutators::tests` — 29 structural unit tests

| Test | Verifies |
|---|---|
| `byte_flip_mutates_content` | content changes, returns `Mutated` |
| `byte_flip_skips_when_no_file_ops` | `Skipped` on mkdir-only delta |
| `replace_file_content_changes_content_and_size` | `op.size == op.content.len()` after replace |
| `replace_file_content_skips_when_no_file_ops` | `Skipped` on rmdir-only delta |
| `replace_file_content_uses_dictionary_sometimes` | a dictionary entry is produced within 200 tries (40% draw active) |
| `add_file_op_grows_delta` | op count +1; new op is `CreateFile` or `Mkdir`; path starts with `/` |
| `add_file_op_uses_guidance_enoent_paths` | guided path chosen within 50 tries (70% bias) |
| `add_file_op_skips_at_max_ops` | `Skipped` when `ops.len() == MAX_OPS` |
| `remove_op_shrinks_delta` | op count −1 |
| `remove_op_skips_single_op_delta` | `Skipped`; delta unchanged |
| `mutate_path_changes_a_component` | path changes within 20 tries; stays absolute |
| `mutate_path_skips_empty_delta` | `Skipped` |
| `mutate_path_whole_swap_uses_baseline_path` | whole-path swap picks a baseline path in ≤ 50 tries |
| `mutate_path_whole_swap_prefers_enoent_paths` | with non-empty `guidance.enoent_paths`, whole-swap prefers them over the baseline list |
| `splice_delta_appends_ops_from_donor` | op count grows by ≥ 1 |
| `splice_delta_skips_empty_pool` | `Skipped` |
| `splice_delta_skips_at_max_ops` | `Skipped` when `ops.len() == MAX_OPS` |
| `splice_delta_sees_live_corpus_updates` | pushing a new delta into the shared `Rc<RefCell<_>>` makes it visible to future `SpliceDelta` calls |
| `destructive_mutator_grows_delta` | op count +1; new op is `DeleteFile\|Rmdir\|Truncate\|SetTimes` |
| `destructive_mutator_generates_all_four_kinds` | all four destructive kinds seen within 200 tries |
| `destructive_mutator_skips_at_max_ops` | `Skipped` when `ops.len() == MAX_OPS` |
| `destructive_mutator_uses_baseline_paths` | baseline path chosen within 50 tries (70% bias, 3-list API) |
| `destructive_mutator_truncate_targets_file_paths` | Truncate kind appears and uses a file path |
| `destructive_mutator_delete_prefers_recreate_paths` | with non-empty `guidance.recreate_paths`, DeleteFile path chosen from that list at least once in 200 tries |
| `update_existing_file_appends_update_op` | kind is `UpdateFile`; path from baseline; `size == content.len()` |
| `update_existing_file_perturbs_baseline_content` | when `baseline_contents` is set, observed content differs from the raw baseline within 200 tries (perturbation active) |
| `update_existing_file_skips_when_no_baseline` | `Skipped`; delta unchanged |
| `update_existing_file_skips_at_max_ops` | `Skipped` when `ops.len() == MAX_OPS` |
| `perturb_bytes_handles_empty_base` | `perturb_bytes(rand, &[])` returns a non-empty `Vec<u8>` across modes |

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

---

## Known Limitations (Deferred — Phase B or Later)

Phase A intentionally stops short of four improvements that were identified
during the pre-Phase-B review.  Each is a clean follow-up, not a Phase A
bug; listing them here so the scope they belong to is obvious.

1. **Read-from-live-VFS content perturbation.**  `UpdateExistingFile`
   currently perturbs a *hard-coded* `baseline_contents` map that mirrors
   `populate_baseline`.  Once the target process actually writes to a
   baseline file (Phase B onward), that map goes stale.  The fix is a
   small `cp_read_file(vfs, path, **out, *n_out)` FFI; once present, the
   harness can re-populate `baseline_contents` from the live VFS each
   iteration, so perturbations always start from what the target last
   wrote.  **Belongs to Phase B** (same iteration as the FUSE log drain).
2. **Success-weighted stage scheduling.**  All eight mutator stages are
   currently picked with uniform probability (after `can_apply` filtering).
   AFL-style favoring would track per-stage yield and bias selection
   toward stages that have historically produced novel checksums.  This
   is a ~30-line change in the real `StdFuzzer` loop once promotions are
   tracked per-stage.  **Belongs to Phase B** when the harness switches
   to `StdFuzzer`.
3. **Corpus minimization pass.**  The live corpus currently accumulates
   deltas up to `MAX_LIVE_CORPUS = 128` and evicts random non-seed slots.
   A minimization pass would periodically drop deltas whose removal does
   not change the set of reachable checksums, keeping the pool small and
   splice donors pointed at minimal evidence.  **Fits naturally in Week 7
   (milestone close)** once the demo harness is running.
4. **`Rc<RefCell<_>>` → `Arc<Mutex<_>>` migration.**  Phase A is
   single-threaded so `Rc<RefCell<LiveCorpus>>` is sufficient.  When the
   harness later runs multiple workers in parallel (Week 8 scale-up), the
   pool becomes `Arc<Mutex<_>>`.  The rest of the mutator API is
   unchanged — `SpliceDelta::new(pool)` accepts either behind a single
   type alias.  **Belongs to Week 8 (scale snapshotting).**

Each lands cleanly on top of Phase A without changing the mutator trait
surface.  Items (1) and (2) are the plan-of-record for Phase B; items (3)
and (4) are tracked here so they don't get lost during Week 6 / Week 7 /
Week 8.
