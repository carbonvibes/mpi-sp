# Detailed End-to-End Execution Plan

## 1. Purpose

This document turns the project specification into a concrete, validated, week-by-week execution plan for a hard 10-week schedule, including the bonus direction only if time permits after the main milestones are stable.

The project goal is to build a performant prototype that lets a fuzzer mutate filesystem-based inputs and present them to an unmodified target program through a kernel-visible filesystem interface. The current best candidate is an in-memory virtual filesystem exposed through FUSE, with LibAFL driving mutations.

This plan assumes:

- the project must be executed within 10 weeks
- the first benchmarking phase is effectively complete
- the current FUSE benchmark result is about `14k ops/sec`
- the benchmark target in the original spec was `>= 1k ops/sec`
- therefore FUSE is currently good enough to continue with unless later integration reveals a hidden bottleneck

## 2. Current Status

Weeks 1 through 3 are complete:

- **Week 1**: benchmark baseline preserved (`~14k ops/sec`), `docs/vfs_design_v1.md` written, VFS v1 scope frozen
- **Week 2**: standalone in-memory VFS core implemented with full unit test suite — path resolution (including trailing-slash and ENAMETOOLONG regression), create/update/delete/mkdir/rmdir, deep-copy snapshot and restore
- **Week 3**: FUSE frontend wired to VFS core (`fuse_vfs/fuse_vfs.c`); read-only callbacks (`getattr`, `readdir`, `open`, `read`) and write callbacks (`create`, `write`, `truncate`, `mkdir`, `unlink`, `rmdir`, `utimens`) all implemented and tested; 40-check integration test suite passes; benchmark at `~13.8k ops/sec` (–6% vs counter baseline, well above floor)

The VFS already has the following mutation and reset primitives at the C API level:
- `vfs_create_file`, `vfs_update_file`, `vfs_delete_file`, `vfs_mkdir`, `vfs_rmdir` — full control-path mutation API
- `vfs_set_times` — mtime/atime set for fuzzer-controlled timestamp mutations
- `vfs_save_snapshot` / `vfs_reset_to_snapshot` — deep-copy snapshot and per-iteration restore

What does **not** yet exist is the external interface through which a fuzzer process sends mutation deltas to the VFS, the diff mechanism for capturing target-side writes, and the LibAFL integration layer.

The next effort is therefore the **control plane** (Week 4/5): the IPC or in-process API that bridges the fuzzer to the live VFS.

## 3. Final Deliverables

By the end of the main plan, the repository should contain at least:

- a documented in-memory VFS implementation
- a FUSE frontend exposing that VFS to unmodified target programs
- a control path that can apply filesystem mutations to the VFS
- a LibAFL mutator or equivalent integration layer producing filesystem mutations
- a minimal end-to-end demo target that crashes on a specific file content
- tests for the VFS core, FUSE behavior where practical, receiver/control APIs, and end-to-end integration
- benchmark results and methodology
- snapshotting support for restoring initial filesystem states
- a real-world fuzzing integration against the container-runtime use case

If time permits, the bonus deliverable is:

- feedback-guided mutation based on observed file accesses

## 4. Ground Rules For Every Phase

The most important rule for the whole project is this: do not stack unvalidated work. Every phase must be validated before the next phase begins.

For every week below, the following standards apply:

- every new module must have a short design note before major implementation starts
- every feature must be tested at the smallest reasonable level first
- every integration step must be checked with a narrow test harness before it is used in the main pipeline
- every performance-sensitive change must be benchmarked against the previous baseline
- all important assumptions must be written down
- bugs discovered late in a phase must be fixed before moving on, unless they are explicitly documented as accepted limitations

Required validation layers throughout the project:

- unit tests for data structures and mutation logic
- integration tests for subsystem boundaries
- system tests for mounted filesystem behavior
- repeatability checks for benchmarks
- regression tests for every bug that is fixed

## 5. Overall Phase Map

The project naturally breaks into these tracks:

1. kernel-facing side: FUSE plus in-memory VFS
2. fuzzer-facing side: filesystem mutation model plus LibAFL integration
3. system integration: connect both sides and prove end-to-end fuzzing works
4. MVP expansion: snapshotting plus real-world campaign
5. bonus work: feedback-guided mutation

The week plan below keeps these tracks coordinated so that no side gets too far ahead without validation.

## 6. Week-By-Week Plan

Because the schedule is only 10 weeks, the plan below compresses the work into a strict critical path. The first end-to-end milestone must happen no later than Week 7. Weeks 8 through 10 must be used for MVP completion, real-world integration, and evaluation. The bonus work is only allowed if the main pipeline is already stable.

### Week 1: Lock The Baseline And Finalize The VFS Scope

Objectives:

- preserve the completed FUSE benchmark baseline
- prevent design drift before implementation speeds up
- commit to a small VFS v1 feature set that can realistically be finished in 10 weeks

Concrete steps:

1. rerun the benchmark at least three times and record the spread
2. document the mount command, benchmark command, machine assumptions, and compiler flags
3. write `vfs_design_v1.md`
4. define supported node types:
   - root directory
   - directories
   - regular files
5. define supported v1 operations:
   - lookup
   - getattr
   - readdir
   - open
   - read
   - create file through control path
   - update file through control path
   - delete file through control path
   - mkdir through control path
   - reset to baseline
6. explicitly defer:
   - symlinks
   - hard links
   - xattrs
   - rich metadata fidelity
   - arbitrary target writes

Validation before proceeding:

- benchmark baseline is reproducible
- VFS v1 scope is written down clearly
- supported and unsupported behavior are explicit

Exit criteria:

- baseline note exists
- VFS design note exists

### Week 2: Implement The Standalone In-Memory VFS Core

Objectives:

- build the real filesystem state model in memory
- keep the logic testable without FUSE
- finish correctness at the core layer before mounting anything

Concrete steps:

1. implement core node structures
2. implement path resolution and normalization
3. implement read-only operations:
   - lookup
   - read file
   - list directory
4. implement mutating operations:
   - create file
   - write file
   - delete file
   - mkdir
5. implement baseline snapshot plus restore at the in-memory level, even if the external snapshot format comes later
6. enforce invariants:
   - parent must exist
   - duplicate entries are rejected
   - root is immutable
   - names are validated

Testing and validation:

- unit tests for path parsing and normalization
- unit tests for successful and failing lookup cases
- unit tests for partial reads and offset behavior
- unit tests for create-update-delete sequences
- unit tests for invalid operations
- unit tests for reset-to-baseline behavior
- randomized mutation-sequence tests if practical

Exit criteria:

- VFS core passes all unit tests
- reset is reliable
- no FUSE-specific logic is mixed into the core

### Week 3: Expose The VFS Through FUSE ✅ COMPLETE

Objectives:

- replace the toy counter backend with the VFS backend
- get a mounted read-only VFS working cleanly
- confirm the benchmark still stays in an acceptable range

Additional work completed beyond original scope:

- full write support added: `create`, `write` (partial and append), `truncate`, `mkdir`, `unlink`, `rmdir`, `utimens` (real POSIX implementation with `UTIME_NOW`/`UTIME_OMIT`)
- 40-check integration test suite in `fuse_vfs/test_mount.sh`
- architecture and results documented in `fuse_vfs/WEEK3.md`

Results:

- benchmark: `~13.8k ops/sec` vs `~14.7k ops/sec` counter baseline (–6.2%, well above 1k floor)
- all 40 integration checks pass
- mounted filesystem is fully writable from the target's perspective

Exit criteria met:

- VFS-backed FUSE mount works reliably for both reads and writes
- benchmark remains practically usable for fuzzing

### Pre-Week 4 Side Quest: Rename And Symlink Support ✅ COMPLETE

Two missing VFS/FUSE features implemented and validated before control plane
work begins. See [`docs/pre_week4_sidequest.md`](docs/pre_week4_sidequest.md)
for the original implementation spec.

**`vfs_rename` / `fvfs_rename` — done:**
- Full POSIX semantics: same-inode no-op, atomic overwrite of file/empty dir at destination
- Cycle detection via parent pointer walk (rejects moving a dir into its own subtree)
- Type mismatch guards: `-EISDIR`, `-ENOTDIR`, `-ENOTEMPTY` all correct
- 19 unit checks pass

**Symlinks — done:**
- `VFS_SYMLINK` kind added to `vfs_kind_t`; `link_target` field on `vfs_node_t`
- `vfs_symlink`, `vfs_readlink` in VFS core; `fvfs_symlink`, `fvfs_readlink` in FUSE layer
- `node_deepcopy` preserves symlinks across snapshot/restore
- `getattr` returns `S_IFLNK | 0777`; kernel follows symlinks before FUSE sees paths (no resolver changes needed)
- FUSE arg order handled correctly: `symlink(target, linkpath)` → `vfs_symlink(vfs, linkpath, target)`
- 14 unit checks pass

Validation: `make test` in `vfs/` — **439/439 checks pass**. `make` in `fuse_vfs/` — clean build, zero warnings.

---

### Week 4: Design The Mutation Model And Build The Control Plane ✅ COMPLETE

Objectives:

- define exactly what a "testcase" is in terms of a filesystem delta
- build the generator that creates initial corpus entries from scratch
- implement the control plane transport so the fuzzer can push deltas to the live VFS
- validate the full mutate → run target → reset cycle end to end

Context: the VFS already has all the low-level mutation primitives. What is missing is: (a) a defined data structure for a filesystem delta that LibAFL can generate and mutate, (b) the generator that creates initial valid deltas, and (c) the transport layer that delivers a delta to the running VFS.

Delta-driven mutation model (the per-iteration loop):

```
1. Load a concrete baseline filesystem into the VFS once (e.g. a minimal rootfs)
2. Save a snapshot of that baseline
3. Per fuzzing iteration:
   a. Fuzzer generator produces a delta: a list of typed ops
      (create file at path P with content C, update file at P, delete file at P, mkdir at P, rmdir at P)
   b. Control plane applies the delta to the live VFS via the existing VFS API
   c. Run the target — it reads (and possibly writes) through the FUSE mount
   d. Reset to the baseline snapshot for the next iteration
```

This model is more efficient than rebuilding the tree from scratch because reset cost is proportional to the delta size, not the full tree.

Concrete steps:

1. design the delta data structure:
   - define a `fs_delta_t` type: a list of `fs_op_t` entries, each being one of:
     `{ kind: CREATE_FILE | UPDATE_FILE | DELETE_FILE | MKDIR | RMDIR | SET_TIMES | TRUNCATE, path: string, content: bytes, mtime: timespec, atime: timespec }`
   - include `SET_TIMES` and `TRUNCATE` as first-class op kinds — the spec explicitly calls out mtime/atime as mutation targets and programs that `stat()` before `read()` are sensitive to size/content mismatches
   - document this in `docs/mutation_model.md`

2. **[Evaluate before committing] prototype the byte-buffer serialization format:**
   - sketch a compact binary layout: `[num_ops u32][op: kind u8 | path_len u16 | path_bytes | size u32 | data_len u32 | data_bytes | timestamps 32 bytes]...`
   - the question is whether standard AFL byte-flip mutations hitting path/op-kind bytes produce too much garbage
   - write a small test: generate 10k random byte mutations of a valid serialized delta, measure what fraction the deserializer accepts
   - if rejection rate is below ~70%, register the byte buffer as the LibAFL `Input` type — this gives AFL's full havoc/splice/minimize for free on file content with no extra mutator code
   - if rejection rate is too high, use structured `fs_delta_t` directly with custom mutator stages only
   - document the result and chosen format in `docs/mutation_model.md` before any mutator code is written

3. **[Required] implement `ensure_parents()` and delta ordering in the control plane:**
   - a flat op list can produce `CREATE_FILE /a/b/c` before `MKDIR /a/b` — this is a correctness issue
   - the control plane receiver must call `ensure_parents()` before any create op, creating missing intermediate directories automatically
   - deletes must be applied depth-first (deepest path first) so parent `RMDIR` does not fail because children still exist
   - the VFS core keeps its strict semantics unchanged; this fixup lives entirely in the control plane layer
   - document the ordering strategy in `docs/mutation_model.md`
   - add tests for out-of-order deltas

4. build the initial corpus generator:
   - produces a minimal valid delta from a known baseline (e.g. one file with seed content)
   - the generator must produce syntactically valid deltas (valid paths, non-empty content)

5. decide the control plane transport:
   - in-process shared-library API if the fuzzer and VFS run in the same process
   - Unix domain socket with a simple binary or text message protocol if process separation is needed

6. write `docs/control_plane.md` describing the transport choice and message wire format

7. implement the control plane receiver on the VFS side:
   - applies each op via the VFS mutation API with `ensure_parents()` fixup
   - returns success/failure per op or for the batch

8. **[Required] add baseline checksum and dry-run mode:**
   - compute a checksum of the serialized baseline tree at import time; store it in snapshot metadata
   - every saved testcase carries this checksum so a crash can be reproduced by anyone with the same baseline
   - add a `--dry-run` flag that applies a delta and dumps the resulting VFS tree without running a target — essential for eyeballing whether the mutator produces reasonable filesystems or noise

9. build a minimal test driver that sends hand-crafted deltas and verifies mounted filesystem updates
10. validate repeated mutate → reset cycles are deterministic and leave no residue

Testing and validation:

- malformed delta rejection tests (invalid path, unknown op type)
- out-of-order delta tests: `CREATE_FILE /a/b/c` before `MKDIR /a/b` must succeed via `ensure_parents()`
- delta apply and mounted read correctness verification
- repeated mutate-reset cycles with stale-state checks
- generator output validity tests (all generated deltas are well-formed)
- dry-run mode produces correct VFS tree dump

Results:

- `fs_delta_t` with all 7 op kinds implemented in `control_plane/delta.h` / `delta.c`
- binary wire format implemented with separate `size` / `data_len` fields so TRUNCATE does not bloat the buffer
- byte-buffer rejection rate measured: **16.7%** (1668 / 10 000 random mutations accepted) → byte-buffer format chosen for LibAFL Input
- `cp_ensure_parents()` and depth-first RMDIR ordering implemented and tested
- `cp_vfs_checksum()` (FNV-1a 64-bit) and `cp_dump_vfs()` (dry-run) implemented
- in-process transport (`libcontrol_plane.a`) — `cp_apply_delta()` is a direct function call
- **224 / 224 checks pass** in `control_plane/cp_test.c`; zero ASAN/UBSan errors
- `docs/mutation_model.md` and `docs/control_plane.md` written

Exit criteria met:

- `fs_delta_t` op kinds (including `SET_TIMES`, `TRUNCATE`) defined and documented
- byte-buffer rejection rate measured (16.7%) and serialization format chosen (byte-buffer)
- `ensure_parents()` implemented and tested
- baseline checksum and dry-run mode working
- control plane transport works end to end
- delta apply and reset are reliable and deterministic

### Week 5: Build The LibAFL Mutator Stages And Close The Feedback Loop

This week is split into two explicit phases. **Phase A is the priority and must
finish.** Phase B is best-effort and can slip into Week 6 without touching the
Week 7 milestone, because the demo harness only needs a working mutator and
dumb loop — not guidance.

Context: a LibAFL mutator is not something that ships ready-made. Each mutator
stage is a function that takes an existing `fs_delta_t` and returns a modified
one. Multiple stages are composed into a mutation pipeline. The generator from
Week 4 seeds the initial corpus; the mutator stages diversify it.

Concrete mutator stages to build:

- `ByteFlipFileContent` — pick a random `UPDATE_FILE` op in the delta, flip bytes in its content
- `ReplaceFileContent` — replace a file's content entirely with a random or dictionary-based value
- `AddFileOp` — append a new `CREATE_FILE` or `MKDIR` op with a random valid path
- `RemoveOp` — drop a random op from the delta (shrinks the testcase)
- `MutatePath` — change the path component of an existing op (tests path-sensitive behavior)
- `SpliceDelta` — take ops from two different deltas and combine them (LibAFL splice analog)

Feedback loop — per-iteration model (implemented in Phase B):

```
1. Clear the iteration log
2. Set target_running = true (enables write logging in FUSE callbacks)
3. Apply the fuzzer's delta to the live VFS
   (direct C API — bypasses FUSE entirely, so these writes are never logged)
4. Run the target — FUSE callbacks log events into the iteration log:
   - CREATE / WRITE       : target created or wrote to a file
   - MKDIR                : target created a directory
   - RENAME_FROM / RENAME_TO : both sides of a rename
   - UNLINK / RMDIR       : target deleted a path
   - ENOENT               : target requested a path that did not exist
5. Set target_running = false
6. Process the log:
   - Write-set (CREATE / WRITE / RENAME_TO / MKDIR) paths:
     read final content from the VFS and promote as a new corpus seed
   - UNLINK / RENAME_FROM paths:
     record as "recreate these paths" guidance for future mutations
     (the target reached code that acts on these paths — they are interesting)
   - ENOENT paths:
     bias the mutator toward creating these paths in the next iteration
7. Reset to the baseline snapshot for the next iteration
```

---

#### Phase A — Must Finish (Mutator Stages, Generator, Dumb Loop)

Objectives:

- implement all mutator stages with the guidance interface stubbed out
- clean up the serialization format
- run a working dumb loop: apply delta → run target → reset, no feedback

The mutator stages accept a `mutation_guidance_t *` parameter from the start
(so Phase B is pure wiring, not a refactor), but pass `NULL` for now:

```c
typedef struct {
    const char **enoent_paths;  /* bias AddFileOp toward these */
    size_t       n_enoent;
    const char **recreate_paths; /* UNLINK/RENAME_FROM signal */
    size_t       n_recreate;
} mutation_guidance_t;

fs_delta_t *mutate(const fs_delta_t *in, const mutation_guidance_t *guidance);
/*                                        ^ NULL during Phase A */
```

Concrete steps:

1. implement each mutator stage listed above as a separate LibAFL `MutationStage`,
   accepting `mutation_guidance_t *` (ignored when NULL)
2. manually test each mutator stage in isolation before composing them
3. implement the dumb per-iteration harness loop:
   - apply delta via control plane (direct VFS API)
   - fork/exec target
   - reset to baseline
   - no logging, no guidance — confirm the loop is stable over 10+ iterations
4. **clean up the serialization format:**
   - remove the magic number (`DELTA_MAGIC`, bytes 0–3) from the wire format
     and from `delta_serialize` / `delta_deserialize` — with custom mutators
     always re-serializing from a valid `fs_delta_t`, the magic check never
     fires and wastes 4 bytes of every corpus entry
   - remove the always-present timestamp fields from ops that are not `SET_TIMES`
     — currently every op carries 32 bytes of zeros for mtime/atime regardless
     of kind; replace with a presence flag byte so only `SET_TIMES` ops pay the
     timestamp cost; for a delta with 20 ops this removes ~608 bytes of zeros
     per corpus entry
   - remove or retire the `rejection_rate` test suite — keep the result
     documented in `docs/mutation_model.md` as historical rationale but stop
     running it as a live test
   - update `DELTA_OP_FIXED` and any size calculations; verify roundtrip tests pass
5. **measure reset cost and FUSE overhead:**
   - instrument `vfs_reset_to_snapshot` with a timer; record per-reset cost
   - if reset cost exceeds 1ms for the demo tree size, evaluate pulling the
     journal/CoW optimisation forward from Week 8
   - benchmark `vfs_read` in a tight loop with no FUSE mount (direct C API only);
     compare to the 13.8k ops/sec FUSE number — this ratio is the kernel FUSE
     overhead tax and goes directly in the paper

Phase A testing and validation:

- mutator stage unit tests: each stage produces a valid `fs_delta_t`
  (well-formed paths, non-empty ops list) with `guidance = NULL`
- dumb loop integration test: run 10 iterations with reset between each,
  confirm no stale state and the loop is deterministic
- serialization roundtrip tests still pass after format change
- reset cost and direct VFS vs FUSE overhead ratio recorded in
  `docs/benchmark_baseline.md`

Phase A exit criteria:

- all mutator stages produce valid deltas
- dumb loop runs stably over multiple iterations with reset
- serialization format cleaned up and roundtrip tests pass
- reset cost and FUSE overhead ratio measured and documented

Phase A scope actually delivered (beyond the minimum above — see
`docs/WEEK5.md` for the full walkthrough):

- live corpus with novel-checksum promotion, bounded at
  `MAX_LIVE_CORPUS = 128` with non-seed eviction; `SpliceDelta` draws from
  the same shared pool so promoted deltas become splice donors immediately
- content dictionary (magic bytes, trigger strings, boundary-sized fills)
  consumed by `ReplaceFileContent` (40% draw) and `UpdateExistingFile`
- real-content perturbation in `UpdateExistingFile` (bit-flip / append /
  truncate / dictionary-splice of live baseline bytes)
- `MutationGuidance` threaded through `MutatePath` (ENOENT → recreate →
  baseline preference on whole-swap) and `DestructiveMutator` (50%
  recreate-path bias on DeleteFile / Rmdir) — Phase A still runs with
  empty guidance, but the wiring is in place
- skip-early stage filtering via `can_apply(&FsDelta)` — no budget slot
  wasted on guaranteed-Skip stages; semantic yield climbed from ~95% to
  98% on a 200-iteration run with 192 corpus promotions

Four items are deliberately deferred (listed in `docs/WEEK5.md` §"Known
Limitations"):

- read-from-live-VFS content via `cp_read_file` FFI → Phase B
- success-weighted per-stage scheduling → Phase B (needs `StdFuzzer`)
- corpus minimization pass → Week 7
- `Rc<RefCell<_>>` → `Arc<Mutex<_>>` migration → Week 8 (parallel scale-up)

---

#### Phase B — Best Effort (FUSE Logging, Guidance Wiring)

Objectives:

- add per-iteration write logging to FUSE callbacks
- wire log output into `mutation_guidance_t` so the closed feedback loop is live

If Phase A runs long, Phase B slips to Week 6. The Week 6 demo and Week 7
milestone do not depend on it.

Concrete steps:

1. add the per-iteration write log to the FUSE layer:
   - define `fuse_iter_log_t`: a fixed-capacity array of
     `{char path[VFS_PATH_MAX], event_t kind}` entries where `event_t` is
     `LOG_CREATE | LOG_WRITE | LOG_MKDIR | LOG_RENAME_FROM | LOG_RENAME_TO | LOG_UNLINK | LOG_RMDIR | LOG_ENOENT`
   - add a global `bool g_target_running` flag (false by default)
   - add logging calls in `fvfs_create`, `fvfs_write`, `fvfs_mkdir`,
     `fvfs_rename` (emits both RENAME_FROM and RENAME_TO), `fvfs_unlink`,
     `fvfs_rmdir` — only when `g_target_running` is true
   - add ENOENT logging in `fvfs_getattr` when `g_target_running` is true
     and the return value is `-ENOENT`
   - deduplicate WRITE entries: multiple write calls to the same path collapse
     to a single LOG_WRITE entry (content is read from the VFS after the run,
     not copied per-callback)
   - expose `fuse_log_clear()`, `fuse_log_set_active(bool)`, and
     `fuse_log_get()` as the control interface
2. upgrade the harness loop from dumb to guided:
   - call `fuse_log_clear()` and `fuse_log_set_active(true)` before the target
   - call `fuse_log_set_active(false)` after the target exits
   - populate `mutation_guidance_t` from the log and pass it to the next mutate call
   - promote write-set paths as new corpus seeds
   - reset to baseline

Phase B testing and validation:

- write-log unit tests: verify each event kind is logged correctly when
  `g_target_running` is true
- suppression test: apply a delta with `g_target_running = false` and confirm
  no entries appear (fuzzer writes via direct VFS API must never be logged)
- deduplication test: two writes to the same path produce exactly one LOG_WRITE entry
- end-to-end test: fake target creates a file, writes to it, deletes another,
  and requests a missing path; verify the log captures all four event types
- feedback loop integration test: 10 guided iterations with reset, no stale state

Phase B exit criteria:

- per-iteration write log implemented in FUSE callbacks and tested
- fuzzer writes (via direct VFS API) confirmed absent from the log
- `mutation_guidance_t` populated from log and consumed by mutator stages
- closed feedback loop runs without stale state

### Week 6: Real LibAFL Integration + Minimal End-To-End Demo Harness

Objectives:

- replace the hand-rolled `fuzz.rs` main loop with a real LibAFL `StdFuzzer`
  driven by code-coverage feedback
- compose LibAFL's stock primitives wherever they fit; implement custom
  `Observer` / `Feedback` / `Executor` only where the FUSE+VFS world has no
  off-the-shelf equivalent
- build the smallest possible filesystem-backed fuzzing demo (`foobar`
  crasher) as the proof-of-life that the integration works end-to-end
- keep the scope narrow enough that Week 7 can be used for stabilization,
  not first-time debugging

Context: by end of Week 5 Phase B we have FsDelta mutators implementing
LibAFL's `Mutator<FsDelta, S>` trait, a working hand-rolled main loop, and
FUSE-write-log-driven `MutationGuidance`.  Week 6 swaps the hand-rolled
loop for LibAFL machinery while keeping the mutator stages and the
guidance signal exactly as they are — those are already in the right
shape.

#### Architectural Principle: Use The Kit, Don't Rebuild It

LibAFL is a kit, not a fuzzer.  Our value-add lives in three custom traits
(`Observer`, `Feedback`, `Executor`) that bridge the FUSE+VFS world into
LibAFL's loop — the rest of the loop (corpus, scheduler, stages, fuzzer)
is stock LibAFL.

| LibAFL primitive | Use as-is | Customize / extend |
|---|---|---|
| `Input` | — | `FsDelta` (semantic, not bytes) — already done |
| `Mutator` trait | — | 8 stages with `can_apply` — already done |
| `Observer` | (compose with `MultiMapObserver` for coverage) | new `FuseLogObserver` (drains the per-iter write log into `MutationGuidance`) |
| `Feedback` | `MapFeedback` over edge-coverage map | new `FsAccessFeedback` (treats novel ENOENT / write-set paths as interesting); compose via `feedback_or!` |
| `Executor` | — | new `VfsExecutor` (apply delta → spawn target → drain log → reset) |
| `Corpus` | `OnDiskCorpus<FsDelta>` (replaces `Rc<RefCell<Vec<FsDelta>>>`) | — |
| `Scheduler` | `IndexesLenTimeMinimizerScheduler` over a `QueueScheduler` base | — |
| `Stage` | `StdMutationalStage` (composes our existing mutators) | — |
| `Fuzzer` | `StdFuzzer` | — |

The hand-rolled `seen_checksums: HashSet<u64>` novelty gate is **retired**
when `MapFeedback` lands — it is a strictly weaker signal than coverage
and keeping it creates two competing novelty sources.

#### Up-Front Decisions (lock these on day 1, do not relitigate)

1. **Coverage source: SanitizerCoverage (`-fsanitize-coverage=trace-pc-guard`)**
   - reasons: LibAFL's default and best-supported path; works with any
     C target the user controls (we own the demo target); single-process
     in-memory hot loop with `InProcessExecutor` is the fastest config
   - rejected: forkserver (slower, more wiring); Intel PT (hardware
     coupling, debugging cost not justified for a Week 6 demo)
   - implementation: target compiled with
     `clang -fsanitize-coverage=trace-pc-guard,trace-cmp` and linked
     against LibAFL's `libafl_targets` runtime
2. **Executor type: `InProcessExecutor` for Week 6, `ForkserverExecutor` revisited in Week 8 if instability shows up**
   - reasons: in-process is the lowest-overhead path; the demo target is
     a tiny program we control; one crash recovery is fine for Week 6
   - migration trigger: if the campaign is unstable (process state leaks
     across iterations, signal handling races), switch to forkserver in
     Week 8 alongside the scale-snapshotting work
3. **Corpus type: `OnDiskCorpus<FsDelta>`** at a deterministic path under
   the run directory, plus `InMemoryOnDiskCorpus` if disk I/O dominates
   in benchmarks
4. **Scheduler: `IndexesLenTimeMinimizerScheduler` over `QueueScheduler`**
   — favours short, fast-executing inputs; standard AFL-style behaviour
5. **Mutation stage composition**: a single `StdMutationalStage` that
   wraps a `StdScheduledMutator` containing our 8 `Mutator<FsDelta, S>`
   impls.  `can_apply` precondition stays in place — `StdScheduledMutator`
   honours `MutationResult::Skipped`.
6. **No code coverage on the FUSE/VFS process itself.**  Coverage is
   collected on the *target binary* only; the FUSE driver and VFS are
   our infrastructure, not the fuzzed surface.

#### Concrete Steps

1. **Build the demo target** (`demo/target_foobar.c`):
   - opens a configured filesystem path, reads it, crashes (`abort()`)
     when the content contains `"foobar"`
   - compiled with SanCov: `clang -fsanitize-coverage=trace-pc-guard,trace-cmp -O1 -g`
   - linked against the LibAFL `libafl_targets` runtime so the coverage
     map is exposed to the harness
2. **Implement the three custom LibAFL traits** in
   `mutator/src/libafl_glue/`:
   - `vfs_executor.rs` — `impl<EM, Z> Executor<EM, FsDelta, S, Z> for VfsExecutor`:
     `run_target` does `apply_delta(vfs, input)` → spawn target →
     `vfs_reset_to_snapshot(vfs)` and returns the standard `ExitKind`
   - `fuse_log_observer.rs` — `impl Observer<FsDelta, S> for FuseLogObserver`:
     `pre_exec` calls `fuse_log_set_active(true)` + `fuse_log_clear()`;
     `post_exec` drains the log and stashes it on the executor for the
     next mutator pass to consume as `MutationGuidance`
   - `fs_access_feedback.rs` — `impl<S> Feedback<S> for FsAccessFeedback`:
     returns `is_interesting = true` when the drained log contains a
     never-before-seen ENOENT path or write-set path; tracked in a
     `HashSet<String>` on the feedback state
3. **Wire the fuzzer** in `mutator/src/bin/fuzz_libafl.rs` (new binary;
   leave `fuzz.rs` in place as a reference / regression target until
   Week 7):
   ```rust
   let mut feedback = feedback_or!(
       MaxMapFeedback::tracking(&edge_observer, true, false),
       FsAccessFeedback::new()
   );
   let mut objective = CrashFeedback::new();
   let scheduler = IndexesLenTimeMinimizerScheduler::new(QueueScheduler::new());
   let mut fuzzer  = StdFuzzer::new(scheduler, feedback, objective);
   let mut executor = VfsExecutor::new(vfs, target_cmd, edge_observer, fuse_log_observer);
   let mut stages = tuple_list!(StdMutationalStage::new(
       StdScheduledMutator::new(tuple_list!(/* our 8 mutators */))
   ));
   fuzzer.fuzz_loop(&mut stages, &mut executor, &mut state, &mut mgr)?;
   ```
4. **Retire the hand-rolled novelty gate.**  Drop `seen_checksums` and
   `MAX_LIVE_CORPUS` eviction from the new binary.  `cp_vfs_checksum`
   stays as a debug-print helper but no longer drives promotion.
5. **End-to-end smoke test**: run `fuzz_libafl` against `target_foobar`,
   confirm it finds the crash, save the crashing `FsDelta` as a
   regression artifact in `demo/regressions/`.
6. **Side-by-side baseline measurement**: run both binaries
   (`fuzz` hand-rolled with FUSE guidance, `fuzz_libafl` with coverage)
   on the same target for the same wall-clock budget; compare
   time-to-first-crash.  This number goes in the paper.

#### Testing and Validation

- direct manual crash reproduction outside the fuzzer (target run with a
  hand-crafted `foobar` file)
- `fuzz_libafl` smoke run with a crashing seed (must reproduce the crash
  on iteration 1)
- `fuzz_libafl` cold run from random seeds (must reach the crash within
  a documented iteration budget — record the median over 10 runs)
- `fuzz_libafl` near-miss input (`fooba`) confirms coverage map advances
  but objective does not fire
- multiple consecutive executions with reset between runs (no stale VFS
  state, no stale coverage map)
- side-by-side: `fuzz` (FUSE-guided hand-rolled) vs `fuzz_libafl`
  (coverage-guided) on `target_foobar` — both find the crash, the
  coverage-guided run is faster

#### Exit Criteria

- `fuzz_libafl` binary exists, compiles cleanly with `--release`,
  and finds the `foobar` crash from a cold corpus within the documented
  iteration budget
- the three custom traits (`VfsExecutor`, `FuseLogObserver`,
  `FsAccessFeedback`) each have a unit test that exercises them in
  isolation against a mock `State`
- `seen_checksums` and `MAX_LIVE_CORPUS` removed from the LibAFL binary;
  `MapFeedback` is the only novelty signal driving promotion
- side-by-side comparison numbers recorded in
  `docs/benchmark_baseline.md` §"Week 6"
- demo target source, build instructions, and a regression `FsDelta`
  for the crash committed under `demo/`

### Week 7: Close The First Major Milestone

Objectives:

- make the first end-to-end milestone fully reproducible
- remove instability before moving to the MVP work

Concrete steps:

1. connect LibAFL testcase generation to the control plane or in-process update path
2. run multiple clean end-to-end campaigns
3. confirm the fuzzer reaches the crash without manual help
4. save the crashing testcase and verify it reproduces
5. add regression coverage for the milestone path

Testing and validation:

- at least three clean reruns from an empty or reset state
- crash reproduction from saved testcase
- explicit confirmation that reset preserves determinism

Exit criteria:

- filesystem-backed fuzzing works end to end and is reproducible

This week is the latest acceptable point for achieving the first major milestone. If it slips beyond Week 7, the project scope must be reduced immediately.

### Week 8: Scale Snapshotting And Begin Real-World Integration

Objectives:

- replace the deep-copy snapshot restore with a journal/diff approach before scaling to real rootfs sizes
- import a real baseline filesystem (e.g. a minimal container rootfs) into the VFS
- begin integration with the container-runtime setup

Context: the current `vfs_reset_to_snapshot` deep-copies the entire tree — O(total filesystem size). For a full container rootfs this will be a bottleneck and must be replaced before real-world integration.

**[Evaluate before implementing] write a journal vs CoW design comparison first:**

Two approaches exist and both have real tradeoffs. Decide before writing any restore code:

- **Journal**: each VFS mutation pushes a reverse entry; restore replays in reverse — O(delta size). Incremental change to existing code. Risk: a single wrong reverse entry produces silently corrupted state after reset, which is extremely hard to debug.
- **Copy-on-Write tree**: each mutation creates new nodes up the path to root; unchanged subtrees are shared. Save snapshot = keep root pointer O(1). Restore = swap root pointer O(1). Mutation cost = O(tree depth, typically <15). No journal to get wrong. Risk: reference counting in C requires discipline; it is a full VFS core refactor.

Write the comparison in `docs/vfs_design_v2.md` before implementing either. If journal is chosen, add comprehensive journal-correctness tests (random mutation sequences, verify post-restore state matches a known-good deep copy). If CoW is chosen, prototype the refcounted node structure first.

**Large file design rule (enforce whichever approach is chosen):** only record a journal entry or create a new CoW node for files that are actually mutated. Never proactively copy unchanged content during tree walks. A rootfs has 50MB+ binaries — the cost of accidentally deep-copying them on every iteration is catastrophic.

Concrete steps:

1. write journal vs CoW design comparison in `docs/vfs_design_v2.md`; decide and implement the chosen approach:
   - measure restore time before and after against a large synthetic tree (1000 files) to confirm the speedup
   - verify post-restore state matches deep-copy result for correctness
2. implement snapshot import from a host directory tree:
   - walk a real directory, create corresponding VFS nodes, set metadata (mode, mtime)
   - this is how a container rootfs gets loaded as the concrete baseline
3. measure restore speed against the imported rootfs baseline
4. identify the integration point in Moritz's harness
5. perform smoke tests with an unmutated baseline rootfs — the target must execute cleanly
6. apply one small delta and verify the target sees the change

Testing and validation:

- snapshot-create and journal restore equivalence checks (result must match deep-copy result)
- repeated restore cycles with correctness assertions
- restore time measurement before and after journal optimization
- real-target smoke tests against the mounted baseline

Exit criteria:

- journal vs CoW comparison written in `docs/vfs_design_v2.md`; approach chosen and implemented
- restore time measured before and after against a large synthetic tree
- a real rootfs baseline can be imported into the VFS
- the real target executes cleanly against the mounted baseline

### Week 9: Real-World Campaign Bring-Up And Initial Evaluation

Objectives:

- move from integration smoke tests to a usable campaign
- start collecting real data early enough to react if something breaks

Concrete steps:

1. connect the mutation flow to the real target setup
2. run short controlled fuzzing sessions
3. record execution throughput, reset cost, and obvious bottlenecks
4. debug reproducibility issues immediately
5. set up and run comparison baselines:
   - **tmpfs + rsync per iteration**: mount a tmpfs, rsync the rootfs into it each iteration, run the target — this is what a practitioner would actually do without this tool; if the FUSE approach is not significantly faster than this, the contribution story weakens
   - **tmpfs + cp -a per iteration**: slightly faster naive alternative, also worth measuring
   - collect NyxFuzz published throughput numbers for comparable workloads; run a direct head-to-head comparison if the hardware and setup support it (NyxFuzz requires KVM/QEMU hypervisor support — treat direct comparison as best-effort)
6. measure concurrency behaviour of the real OCI target:
   - how many processes hit the FUSE mount simultaneously during a single target run?
   - if single-threaded FUSE serialisation is measurably slowing the target, evaluate enabling FUSE multithreading (`-o clone_fd`) with a pthread rwlock around VFS access
   - if it is not a bottleneck, leave single-threaded as-is

Testing and validation:

- repeated short campaigns from a clean baseline
- reproducible failures or crashes
- saved scripts for measurement and reruns
- comparison baseline numbers recorded in `docs/evaluation_plan.md`

Exit criteria:

- the real-world setup runs repeatedly under harness control
- initial evaluation numbers exist
- at least tmpfs + rsync comparison baseline measured and recorded
- NyxFuzz published numbers collected

### Week 10: Final Evaluation, Hardening, And Writeup Support

Objectives:

- collect the strongest data possible within the remaining time
- harden the pipeline enough that the results are defensible
- leave the repository in a state another person can run

Concrete steps:

1. run the final benchmark suite:
   - open-read-close baseline (counter_fs reference)
   - direct VFS API throughput (no FUSE, no mount) — quantifies the FUSE kernel overhead tax as a ratio
   - VFS-backed FUSE read throughput (~13.8k ops/sec baseline)
   - mutation application cost (time to apply a delta of N ops)
   - reset cost before and after journal/CoW optimisation
   - real-target throughput (iterations per second end to end)
   - tmpfs + rsync and tmpfs + cp -a comparison baselines
2. verify deterministic replay: apply a saved crashing testcase using the baseline checksum and delta, confirm crash reproduces
3. run longer fuzzing sessions if compute time allows
4. add missing regression tests for discovered bugs
5. improve logging and reproducibility documentation
6. prepare architecture notes, benchmark methodology, and result summaries

Testing and validation:

- multiple repeated performance runs
- confirmation that major demos still work after hardening changes
- final rerun of the first milestone and the real-world integration path

Exit criteria:

- enough evidence exists to support the claims
- the implementation is stable enough for demonstration and writeup

## 7. Bonus Plan If Time Permits

The bonus direction should only begin after the main pipeline is stable, reproducible, and evaluated.

### Bonus Week 1: Extend Logging To Read-Side Access Patterns

Objectives:

- add read-side signals to the iteration log (write-side and ENOENT are already
  in the main plan; this extends to successful reads and directory listings)
- understand whether read-frequency data improves mutation guidance beyond what
  ENOENT alone provides

Context: the main plan logs what the target *wrote* and what paths it *requested
but missed*. This bonus adds what it *successfully read*, which lets the mutator
bias content mutations toward files the target actually consumed (hot-file
weighting) rather than treating all files in the delta equally.

Concrete steps:

1. add LOG_READ and LOG_READDIR event kinds to `fuse_iter_log_t`
2. instrument `fvfs_read` to emit LOG_READ (deduplicated per path, with a hit counter)
3. instrument `fvfs_readdir` to emit LOG_READDIR for each directory listed
4. expose per-path read counts alongside the existing write-set and ENOENT sets
5. inspect whether targets repeatedly read the same files across iterations
   (high-count paths are prime content-mutation targets)

Validation:

- read-log entries are correct and do not corrupt normal execution
- additional overhead of read logging is measured and documented
- at least one example showing a hot file that ENOENT alone would not have identified

### Bonus Week 2: Design Feedback-Guided Mutation

Objectives:

- turn file-access observations into a mutation heuristic

Concrete steps:

1. discuss the approach with advisors before implementing
2. decide whether feedback affects:
   - file creation probability
   - mutation focus on touched files
   - directory expansion around requested paths
3. define fallback behavior so guidance does not collapse diversity

Validation:

- mutation policy is documented before code is written

### Bonus Week 3: Implement And Evaluate Feedback Guidance

Objectives:

- see whether access-aware mutation improves time to interesting behavior

Concrete steps:

1. implement the heuristic in the mutator
2. compare guided and unguided runs on the same targets
3. measure:
   - time to first crash
   - coverage growth if available
   - number of useful files created

Validation:

- results are compared over multiple runs
- heuristics can be disabled cleanly for ablation

## 8. Validation Checklist By Layer

This checklist should be reused throughout the project.

### VFS Core

- all core operations return correct success and error values
- path resolution is deterministic
- mutation sequences preserve invariants
- reset restores a clean baseline

### FUSE Layer

- mounted view matches VFS state
- directory listings are correct
- partial reads and offsets behave properly
- nonexistent paths return expected errors

### Control Plane

- invalid messages are rejected safely
- valid batches apply atomically or with documented semantics
- reset is reliable

### Fuzzer Integration

- testcase-to-filesystem mapping is deterministic
- crashes are attributable and reproducible
- seeds, corpus, and output directories are preserved

### Performance

- open-read-close throughput remains acceptable
- mutation application cost is measured
- snapshot restore time is measured
- logging and instrumentation overhead are known

## 9. Suggested Repository Artifacts

To keep the project organized, create or maintain documents like these as the work progresses:

- `docs/benchmark_baseline.md`
- `docs/vfs_design_v1.md`
- `docs/mutation_model.md`
- `docs/control_plane.md`
- `docs/evaluation_plan.md`
- `docs/real_world_integration.md`

Also maintain:

- reproducible benchmark scripts
- regression tests for every important bug
- one short README section describing how to run the major demos

## 10. Recommended Execution Order If Time Gets Tight

If time pressure becomes serious, the minimum sequence that still produces a credible project is:

1. preserve benchmark baseline
2. build in-memory VFS core
3. expose it through FUSE
4. add mutation application and reset
5. integrate minimal LibAFL harness
6. prove end-to-end crash discovery
7. add snapshotting
8. run at least one real-world fuzzing experiment

If time remains after that, do:

1. multi-file and richer mutation semantics
2. stronger evaluation
3. feedback-guided mutation bonus

## 11. Immediate Next Actions

Weeks 1–3, the pre-Week 4 side quest, and Week 4 are done. The immediate next sequence is:

1. **Week 5 Phase A — mutator stages + dumb loop + measurements (must finish)**:
   - implement LibAFL mutator stages (`ByteFlipFileContent`, `AddFileOp`, `RemoveOp`, `MutatePath`, `SpliceDelta`, `ReplaceFileContent`) with `mutation_guidance_t *` stubbed as `NULL`
   - run the dumb loop: apply delta → run target → reset, no feedback
   - clean up serialization format (remove magic number and always-present timestamps)
   - instrument `vfs_reset_to_snapshot` with a timer; record per-reset cost
   - run direct VFS API benchmark (no FUSE) and record FUSE overhead ratio

2. **Week 5 Phase B — FUSE logging + guidance wiring (best effort, can slip to Week 6)**:
   - implement per-iteration write log in FUSE callbacks (`fvfs_create`, `fvfs_write`,
     `fvfs_mkdir`, `fvfs_rename`, `fvfs_unlink`, `fvfs_rmdir`, ENOENT in `fvfs_getattr`)
     with `g_target_running` flag gating all logging
   - wire log output into `mutation_guidance_t` and enable the closed feedback loop

2. **Week 6 — demo harness**: minimal crash target, end-to-end harness run with seeded crashing input

3. **Week 8 — before implementing restore optimisation**: write journal vs CoW design comparison in `docs/vfs_design_v2.md`, decide, then implement

Remaining VFS/FUSE work — non-blocking for Weeks 5–6 but needed before OCI integration (Week 8):

- `chmod` / `mode` field on `vfs_node_t` — needed for permission-sensitive targets
- `release` no-op callback — flush semantics correctness
