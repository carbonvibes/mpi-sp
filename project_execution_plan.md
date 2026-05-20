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

#### Phase B — LibAFL Integration + Real Campaign (Immediate Next Priority)

**Priority change:** LibAFL integration is pulled forward from Week 6 and
happens immediately after Phase A, before FUSE log guidance wiring.  The
reason: the hand-rolled loop has served its purpose as a validation scaffold.
Before adding the complexity of FUSE log guidance, we need to know that the
full LibAFL machinery (StdFuzzer, custom Executor, Feedback, Observer) works
end-to-end and that a real campaign runs stably.  Guidance wiring on a broken
harness is wasted work.

Objectives:

- replace the hand-rolled `fuzz.rs` main loop with a real LibAFL `StdFuzzer`
  driven by code-coverage feedback
- run a real fuzzing campaign against the demo target to validate the full
  stack end-to-end
- observe throughput, corpus growth, and stability before adding FUSE log
  guidance

Context: the 9 mutator stages already implement `Mutator<FsDelta, S>` and are
in the right shape.  This phase swaps the loop, wires the three custom traits,
and runs.  The FUSE log guidance (Phase C below) adds on top of a working,
validated harness.

Architectural principle: use LibAFL's kit, don't rebuild it.  Custom work lives
in three traits only — everything else is stock LibAFL.

| LibAFL primitive | Use as-is | Customize |
|---|---|---|
| `Input` | — | `FsDelta` — already done |
| `Mutator` trait | — | 9 stages with `can_apply` — already done |
| `Observer` | `MultiMapObserver` for coverage | new `FuseLogObserver` (Phase C) |
| `Feedback` | `MapFeedback` over edge-coverage map | new `FsAccessFeedback` (Phase C) |
| `Executor` | — | new `VfsExecutor` (apply delta → spawn target → reset) |
| `Corpus` | `OnDiskCorpus<FsDelta>` | — |
| `Scheduler` | `IndexesLenTimeMinimizerScheduler` | — |
| `Stage` | `StdMutationalStage` | — |
| `Fuzzer` | `StdFuzzer` | — |

Up-front decisions (lock on day 1):

1. **Coverage: SanitizerCoverage (`-fsanitize-coverage=trace-pc-guard,trace-cmp`)** — LibAFL's default path, works with any C target we control
2. **Executor: `InProcessExecutor`** — lowest overhead for the demo target; revisit forkserver in Week 8 if instability appears
3. **Corpus: `OnDiskCorpus<FsDelta>`** — replaces `Rc<RefCell<Vec<FsDelta>>>`
4. **Scheduler: `IndexesLenTimeMinimizerScheduler` over `QueueScheduler`**
5. **Retire `seen_checksums` and `MAX_LIVE_CORPUS` eviction** — `MapFeedback` is the only novelty signal; keeping both creates competing promotion sources

Concrete steps:

1. **Build the demo target** (`demo/target_foobar.c`):
   - opens a configured filesystem path, reads it, crashes (`abort()`) when content contains `"foobar"`
   - compiled with SanCov: `clang -fsanitize-coverage=trace-pc-guard,trace-cmp -O1 -g`
   - linked against LibAFL's `libafl_targets` runtime

2. **Implement `VfsExecutor`** in `mutator/src/libafl_glue/vfs_executor.rs`:
   - `run_target`: `apply_delta(vfs, input)` → spawn target → `vfs_reset_to_snapshot(vfs)` → return `ExitKind`

3. **Wire the fuzzer** in `mutator/src/bin/fuzz_libafl.rs` (new binary; leave `fuzz.rs` as a regression reference):
   ```rust
   let mut feedback = MaxMapFeedback::tracking(&edge_observer, true, false);
   let mut objective = CrashFeedback::new();
   let scheduler = IndexesLenTimeMinimizerScheduler::new(QueueScheduler::new());
   let mut fuzzer  = StdFuzzer::new(scheduler, feedback, objective);
   let mut executor = VfsExecutor::new(vfs, target_cmd, edge_observer);
   let mut stages = tuple_list!(StdMutationalStage::new(
       StdScheduledMutator::new(tuple_list!(/* 9 mutators */))
   ));
   fuzzer.fuzz_loop(&mut stages, &mut executor, &mut state, &mut mgr)?;
   ```

4. **Run a real campaign** — start from cold corpus, let it run, observe:
   - time-to-first-crash against `target_foobar`
   - iterations/sec, corpus growth rate, stage hit distribution
   - any instability (state leaks, signal races) to catch before Phase C

5. **Record side-by-side numbers**: `fuzz` (hand-rolled dumb loop) vs `fuzz_libafl` (coverage-guided) on `target_foobar` — time-to-first-crash comparison; these go in the paper

Phase B exit criteria:

- `fuzz_libafl` binary compiles with `--release` and finds the `foobar` crash from a cold corpus
- `VfsExecutor` has a unit test exercising it against a mock state
- `seen_checksums` and `MAX_LIVE_CORPUS` eviction removed from the LibAFL binary
- campaign observation notes recorded (throughput, corpus growth, any instability)
- crashing `FsDelta` saved as a regression artifact under `demo/regressions/`

---

### Week 6: Symlink Op + Crun-Targeted Mutators + Op Vocabulary Expansion

Context: the real crun campaign is complete. The VFS and FUSE layers already
have full symlink support (pre-Week 4 sidequest: `vfs_symlink`, `fvfs_symlink`,
`fvfs_readlink` implemented and tested). What is missing is the Rust-side
plumbing — and, more importantly, a set of mutators and seeds that specifically
target the bug classes symlinks expose in crun.

**Why symlinks matter for crun specifically:**

Before `pivot_root`, crun walks the rootfs as seen from the *host* filesystem to
create mount destinations (`mkdir -p /rootfs/proc`, `/rootfs/dev`, etc.), set up
devices, resolve `/etc/passwd`/`/etc/group`, and validate the executable path.
If any symlink in those paths escapes the rootfs boundary at this stage, crun
follows it into the host filesystem. This overlaps with the same family of
runtime path-resolution and `/proc/self/exe` escape hazards, including runc-style
attacks (CVE-2019-5736), but the specific crun target here is pre-pivot and
mount-destination path handling. The dangerous window is pre-pivot — the fuzzer
has direct control over every symlink crun will encounter in this window through
the FUSE rootfs.

No new op types are needed beyond `CreateSymlink`. Mount destination replacement
(e.g. `/proc` → symlink) is expressed as `rmdir("/proc") + create_symlink("/proc",
target)` using existing ops. The `SymlinkChainMutator` emits N `CreateSymlink` ops
in sequence. The richness comes from the mutators and seeds, not from new op
vocabulary.

Objectives:

- add `CreateSymlink` as a first-class `FsOp` through the full Rust stack
- add a `replace_with_symlink` Rust-side helper that reliably replaces any path (including non-empty dirs) with a symlink
- add 6 crun-targeted mutator stages covering the key symlink bug classes, ordered by bug-finding value
- add a rich crun-specific seed corpus targeting pre-pivot escape scenarios

#### Part A — CreateSymlink Op ✓ COMPLETE

The change is mechanical and self-contained:

1. Add `CreateSymlink { path, target }` variant to `FsOpKind` enum in `mutator/src/delta.rs`
2. Add `FsOp::create_symlink(path, target)` constructor
3. Add Rust FFI binding in `mutator/src/ffi.rs`; wire into `apply_delta()` match arm
   calling `vfs_symlink(vfs, linkpath, target)` — the C function already exists in the VFS
4. Confirm `fvfs_symlink` is registered in `fuse_operations` (pre-Week 4 sidequest wired it,
   verify it's still connected after any refactors)

Testing:
- `FsOp::create_symlink` round-trips through serialization
- `apply_delta` with `CreateSymlink` op: FUSE mount shows symlink with correct target via `readlink`
- symlink loop (`/loop → /loop`) does not hang VFS or FUSE layer
- absolute symlink target (`/proc/self/exe`) serializes and applies correctly

#### Part A.2 — `replace_with_symlink` Helper ✓ COMPLETE

The naive pattern `rmdir(path) + create_symlink(path, target)` only works when `path`
is an empty directory. For `/etc`, `/bin`, `/usr`, `/lib`, and any other non-empty
baseline dir, `rmdir` returns `ENOTEMPTY` and `create_symlink` then fails with `EEXIST`.
Many high-value seeds silently produce no symlink at all without this helper.

Add a Rust-side helper (not a serialized `FsOpKind`) that expands into correct primitive ops:

```rust
fn replace_with_symlink(path: &str, target: &str) -> Vec<FsOp> {
    // Existing file or symlink → delete_file, then create_symlink
    // Empty directory         → rmdir, then create_symlink
    // Non-empty directory     → delete children in postorder, rmdir, then create_symlink
}
```

The helper should be built around a **baseline path-kind index** — a pre-computed table of
the baseline VFS tree that seed generation and mutators can query without inspecting the live
VFS at mutation time. LibAFL mutators cannot safely walk the live VFS during fuzzing, so the
index is the only practical source of path metadata.

```rust
enum PathKind { File, Dir, Symlink }

struct PathInfo {
    path: String,
    kind: PathKind,
    children: Vec<String>,  // direct children; empty for File/Symlink
}
```

Seed generation inspects the baseline VFS directly to build this index. Mutators use the
cached `PathInfo` table plus the current `FsDelta` (to account for ops already in the
delta that may have changed path state) to decide the correct expansion.

This helper is used by every mutator that replaces an existing path with a symlink.
Without it, mount-destination and parent-component seeds will fail silently and the
fuzzer's high-value inputs will never reach crun.

#### Part B — Crun-Targeted Mutators ✓ COMPLETE

Six mutator stages ordered by crun-specific bug-finding value:

**1. `MountDestinationSymlinkMutator`** — highest priority

Targets the paths crun *always* accesses pre-pivot to create mount destinations:
`/proc`, `/dev`, `/sys`, `/tmp`, `/etc`. For each selected path:
- Uses `replace_with_symlink(path, target)` — not raw `rmdir + create_symlink`
  (raw rmdir fails on non-empty dirs like `/etc`, silently leaving no symlink)
- Target selection (weighted):
  - Relative escape to same-named host path (35%): `../../proc`, `../../dev`, `../../sys`
  - Absolute target (25%): `/proc`, `/dev`, `/proc/self/fd`, `/proc/self/exe`
  - Cross-type (20%): symlink to wrong-type path: `/proc → /etc/passwd`, `/dev → /bin/true`
  - Dangling (15%): `/proc → /nonexistent`, `/dev → /missing`
  - Special file (5%): `/proc → /dev/null`, `/dev → /proc/self/fd`

This mutator fires on every corpus entry. Mount destination setup is exercised
every crun iteration — highest density of pre-pivot escape opportunities.

**2. `MountOptionSymlinkMutator`** — crun-specific, second priority

crun has explicit logic for bind mount symlink-aware options. This mutator
coordinates config and rootfs state, but the source/destination split matters:

OCI bind mount `source` is a **host path** (resolved from the host namespace before
pivot). `destination` is a **container path** (resolved from the FUSE rootfs).
This means source and destination symlinks are created in different namespaces:

- **Source-related options** (`copy-symlink`, `src-nofollow`): create a temporary
  symlink on the host filesystem (e.g. `/tmp/fuzz_src_symlink → /etc/passwd`) and
  set `mount.source` to that host path. A symlink created only inside the FUSE
  rootfs will NOT exercise these branches — crun reads `mount.source` from the
  host before entering the container rootfs.
- **Destination-related options** (`dest-nofollow`): create a symlink inside the
  FUSE rootfs at `mount.destination`. This is correctly served through FUSE.

Option combinations to generate:
  - `["bind", "copy-symlink"]` — crun copies the symlink itself, not its target
  - `["bind", "src-nofollow"]` — crun does not follow symlink at source
  - `["bind", "dest-nofollow"]` — crun does not follow symlink at destination
  - `["bind", "copy-symlink", "src-nofollow"]` — invalid combination
  - `["bind", "copy-symlink", "dest-nofollow"]` — invalid combination

This directly exercises crun's symlink-aware bind mount branches, which are
unlikely to be reached by any other mutator. Invalid combinations test error
handling in mount option parsing.

**3. `ExecutableSymlinkMutator`** — executable path coordination

Coordinates both config and rootfs to create the "exec path is a symlink" scenario
that cannot be found by independent config and rootfs mutation:

1. Pick a synthetic executable path (`/bin/target`, `/usr/local/bin/x`)
2. Override `process.args[0]` in `CombinedInput.config` via `override_args` field
   on `CombinedInput` (applied by `CombinedConverter` alongside `root.path` override)
3. Create that path in the rootfs as a symlink to an interesting target

Interesting symlink targets for the executable path:
- `/proc/self/exe` — classic runc-family escape vector
- `/proc/self/mem` — process memory
- `/proc/self/fd/0` — stdin
- `/dev/zero` — exec of `/dev/zero`
- `../../usr/bin/python3` — escapes rootfs to host binary
- `/dev/null` — dead exec target
- `/nonexistent` — dangling, tests ENOENT handling on exec

This is the only mutator that systematically explores "config says run X, rootfs
has X as a symlink to Y." Without it, the two dimensions only coordinate by accident.

**4. `ParentComponentSymlinkMutator`** — historically dangerous class

Most fuzzers only create leaf symlinks. This mutator replaces a *non-leaf* path
component — the class that historically caused container escapes.

Targets non-leaf components of paths crun is likely to access:
- parent of `process.args[0]` (e.g. `/bin` if binary is `/bin/true`)
- parent of each mount destination (e.g. `/usr` for `/usr/lib`)
- parent of `/etc/passwd` → `/etc`
- parent of `/etc/group` → `/etc`
- parent of `/dev/null` → `/dev`
- parent of `/dev/console` → `/dev`

Uses `replace_with_symlink` (non-empty dirs like `/etc`, `/bin`, `/usr` must use
recursive deletion). Example replacements:
- `/bin → ../../bin` — host's `/bin/true` resolves instead of rootfs `/bin/true`
- `/etc → ../../etc` — host's `/etc/passwd` resolves when crun reads user info
- `/usr → ../../usr` — host library paths resolve
- `/lib → ../../lib` — dynamic linker escapes
- `/dev → /proc/self/fd` — absolute target to proc fd dir

**5. `SymlinkEscapeMutator`** — relative and absolute target dictionary

Generates symlink targets crafted to escape the rootfs. Two modes:

*Relative mode* — parameterized by escape depth (2–8 `../` hops):
```rust
static RELATIVE_TARGETS: &[&str] = &[
    "etc/passwd",
    "etc/shadow",
    "proc/self/exe",
    "proc/self/mem",
    "proc/sysrq-trigger",
    "dev/sda",
    "dev/zero",
    "run/containerd",
    "var/run/docker.sock",
    "proc/self/fd",
];
// construction: "../".repeat(depth) + target
```

*Absolute mode* — targets where pre-pivot vs post-pivot path resolution differs:
```rust
static ABSOLUTE_TARGETS: &[&str] = &[
    "/proc",
    "/proc/self",
    "/proc/self/exe",
    "/proc/self/fd",
    "/proc/self/fd/0",
    "/dev",
    "/dev/null",
    "/sys",
    "/etc/passwd",
    "/bin/sh",
];
```

Absolute symlinks are important because crun's safe-open logic before the rootfs
boundary is enforced may handle them differently from relative ones. If a
pre-pivot absolute symlink is followed from host context, the escape is direct.

Applied at a random path in the current delta that is a file or leaf directory,
using `replace_with_symlink` when replacing an existing path.

**6. `LoopAndDepthMutator`** — robustness, lower priority

Creates symlink loops and deep chains. Useful for error-handling coverage but less
crun-specific than the mutators above. Lower mutation weight.

Chain lengths: `{1, 5, 10, 39, 40, 41}`. Chain-100 removed — it only confirms
`ELOOP` handling and wastes mutation energy.

Interesting cases:
- Self-loop: `/loop → /loop`
- Two-cycle: `/a → /b`, `/b → /a`
- Length 40: kernel limit, should succeed
- Length 41: should return `ELOOP` — tests every crun call site
- Long target strings near `PATH_MAX`
- Targets with repeated slashes: `////proc//self//exe`

The chain is anchored at a path crun will access (`/bin/true`, `/proc`, `/dev`).
Emits N `CreateSymlink` ops in sequence within the delta.

**Also extend existing mutators:**

- `AddFileOp`: 25% probability to emit `CreateSymlink` instead of `CreateFile`;
  when `MutationGuidance` provides an `ENOENT` path, bias toward symlink (30%)
- `DestructiveMutator`: new arm — `replace_with_symlink(path, proc_or_dev_target)`
  to replace a real file with a symlink pointing to a proc/dev special file
- Guidance: treat `ELOOP`, `ENOTDIR`, `EEXIST`, and `EINVAL` as meaningful signals
  (not just noise) — they indicate crun reached a symlink-handling branch

#### Part C — Crun-Specific Seeds ✓ COMPLETE

Seeds are organized by the crun pre-pivot operation they target:

Note: all seeds use `replace_with_symlink(path, target)` — a helper that
expands into the correct primitive ops based on what the VFS snapshot shows
at that path (empty dir → rmdir+symlink; non-empty dir → recursive delete+rmdir+symlink;
file → delete_file+symlink). Using raw `rmdir + create_symlink` on non-empty dirs like
`/etc` or `/bin` silently fails: `rmdir` returns ENOTEMPTY, `create_symlink` returns
EEXIST, and no symlink is created.

```rust
// ── Mount destination escapes — relative targets (highest priority) ───────────
// crun always calls make_parent_directories for each mount destination pre-pivot.
// replace_with_symlink handles non-empty baseline dirs correctly.
replace_with_symlink("/proc", "../../proc"),
replace_with_symlink("/dev",  "../../dev"),
replace_with_symlink("/sys",  "../../sys"),
replace_with_symlink("/tmp",  "../../tmp"),

// All mount destinations simultaneously — exercises combined error handling
FsDelta::from(vec![
    replace_with_symlink("/proc", "../../proc"),
    replace_with_symlink("/dev",  "../../dev"),
    replace_with_symlink("/sys",  "../../sys"),
].concat()),

// ── Mount destination escapes — absolute targets ──────────────────────────────
// Absolute symlinks: pre-pivot vs post-pivot resolution may differ.
// If crun's safe-open logic doesn't enforce rootfs boundary before pivot,
// an absolute symlink is a direct escape — no ../.. counting needed.
replace_with_symlink("/proc", "/proc"),
replace_with_symlink("/dev",  "/proc/self/fd"),
replace_with_symlink("/sys",  "/sys"),
replace_with_symlink("/proc", "/proc/self/exe"),

// ── Mount destination → wrong type ───────────────────────────────────────────
replace_with_symlink("/proc", "/etc/passwd"),    // dir expected, symlink to file
replace_with_symlink("/dev",  "/bin/true"),

// ── Mount destination → dangling ─────────────────────────────────────────────
replace_with_symlink("/proc", "/nonexistent"),
replace_with_symlink("/dev",  "/missing"),

// ── Parent component symlinks — non-leaf, historically dangerous ──────────────
// replace_with_symlink recurses through /etc, /bin, /usr children before replacing
replace_with_symlink("/etc", "../../etc"),       // /etc/passwd → host
replace_with_symlink("/bin", "../../bin"),       // /bin/true → host
replace_with_symlink("/lib", "../../lib"),       // dynamic linker → host
replace_with_symlink("/usr", "../../usr"),       // /usr/bin/python3 → host
replace_with_symlink("/dev", "/proc/self/fd"),   // absolute parent escape

// ── Binary path → proc/dev special files ─────────────────────────────────────
// crun validates the binary before pivot, then execs it after — TOCTOU window
// /bin/true is a file so replace_with_symlink emits delete_file + create_symlink
replace_with_symlink("/bin/true", "/proc/self/exe"),
replace_with_symlink("/bin/true", "/proc/self/mem"),
replace_with_symlink("/bin/true", "/proc/self/fd/0"),
replace_with_symlink("/bin/true", "/dev/zero"),
replace_with_symlink("/bin/true", "/dev/null"),
replace_with_symlink("/bin/true", "../../usr/bin/python3"),

// ── Config-reading files → host escape ───────────────────────────────────────
// crun reads /etc/passwd and /etc/group pre-pivot for uid/gid resolution
replace_with_symlink("/etc/passwd", "../../etc/passwd"),
replace_with_symlink("/etc/passwd", "/etc/passwd"),        // absolute
replace_with_symlink("/etc/passwd", "../../../etc/shadow"),
replace_with_symlink("/etc/group",  "../../etc/group"),

// ── Bind mount with symlink-aware options ────────────────────────────────────
// Seeds for MountOptionSymlinkMutator — coordinate config + rootfs
// (expressed here as conceptual pairs; mutator generates the actual CombinedInput)
// source: symlink in rootfs, config: bind mount with copy-symlink option
// source: symlink in rootfs, config: bind mount with src-nofollow option
// destination: symlink in rootfs, config: bind mount with dest-nofollow option

// ── ELOOP boundary cases ──────────────────────────────────────────────────────
// Chain of 40: kernel limit — should succeed
// Chain of 41: should ELOOP — does crun handle this at every call site?
// (generated programmatically by LoopAndDepthMutator seeds)
FsDelta::new(vec![FsOp::create_symlink("/loop", "/loop")]),  // self-loop
FsDelta::new(vec![
    FsOp::create_symlink("/a", "/b"),
    FsOp::create_symlink("/b", "/c"),
    FsOp::create_symlink("/c", "/a"),
]),

// ── Relative escape via deep path ────────────────────────────────────────────
replace_with_symlink("/etc/passwd", "../../../etc/shadow"),
FsDelta::new(vec![FsOp::create_symlink("/bin/sh", "../../../proc/sysrq-trigger")]),

// ── Dangling symlink at a path crun opens ────────────────────────────────────
FsDelta::new(vec![FsOp::create_symlink("/bin/sh", "/nonexistent")]),
replace_with_symlink("/proc", "/nonexistent"),

// ── Targets with path noise (repeated slashes, trailing dots) ────────────────
FsDelta::new(vec![FsOp::create_symlink("/bin/x", "////proc//self//exe")]),
FsDelta::new(vec![FsOp::create_symlink("/bin/x", "../../../proc/./self/./exe")]),
```

#### Part D — Guidance Integration Note (prep for Week 7)

When Week 7's FUSE logging is live, the guidance must handle symlinks explicitly:
- When FUSE log shows `ENOENT` for a path matching a known crun access pattern
  (mount destinations, binary paths, config files), the next iteration should
  create a **symlink** at that path rather than a regular file
- When FUSE log shows a `READ` from path X, the next iteration should sometimes
  replace X with a symlink — currently the guidance only biases toward regular
  file creation; the symlink bias must be added explicitly in Week 7

Testing and validation:

- `FsOp::create_symlink` round-trips through serialization
- `apply_delta` with `CreateSymlink`: FUSE mount shows symlink with correct target via `readlink`
- absolute symlink target (`/proc/self/exe`) serializes and applies correctly
- symlink loop (`/loop → /loop`) does not hang VFS or FUSE layer
- `replace_with_symlink` on non-empty dir produces correct ops (children deleted first)
- `replace_with_symlink` on a file produces `delete_file + create_symlink`
- `MountDestinationSymlinkMutator` uses `replace_with_symlink`, not raw `rmdir`
- `MountOptionSymlinkMutator` generates valid and invalid option combinations
- `ExecutableSymlinkMutator` produces `CombinedInput` where config path and rootfs symlink are aligned
- `LoopAndDepthMutator` produces chains of correct length; chain-41 applies without crash
- guidance treats `ELOOP`, `ENOTDIR`, `EEXIST`, `EINVAL` from crun as meaningful signals

Exit criteria:

- `CreateSymlink` implemented through full stack: `FsOp` → FFI → `cp_apply_delta` → VFS → FUSE
- baseline `PathInfo` index built from VFS snapshot; available to both seed generation and mutators
- `replace_with_symlink` helper implemented using `PathInfo`; used by all mutators that replace existing paths
- all 6 crun-targeted mutators implemented and unit-tested
- `MountOptionSymlinkMutator` correctly uses host-side temp symlinks for source options
  (`copy-symlink`, `src-nofollow`) and FUSE rootfs symlinks for destination options (`dest-nofollow`)
- crun-specific seeds present in `rootfs_seeds()` as expanded `Vec<FsOp>` (not unexpanded helper calls);
  covering mount destinations, parent components, executable paths, bind mount options,
  absolute targets, dangling, and loop cases
- `SetXattr`/`RemoveXattr`, `Chmod`, `Chown` deferred to Week 8 (not Week 6 scope)

### Week 7: FUSE Logging + FuseLogObserver + FsAccessFeedback

Context: the campaign has validated the harness end-to-end. Week 7 closes the
guidance loop — adding per-iteration FUSE access logging and wiring it into
LibAFL as an `Observer` + `Feedback`. This is the core research contribution:
making the fuzzer aware of what the target actually touched on the filesystem,
and biasing future mutations toward those paths.

Objectives:

- add per-iteration write logging to FUSE callbacks (gated by `g_target_running`)
- wire log output into `MutationGuidance` so the closed feedback loop is live on top of the validated LibAFL harness
- implement `FuseLogObserver` and `FsAccessFeedback`
- measure guided vs unguided coverage growth — this is the key evaluation result

#### Part A — FUSE Per-Iteration Log

Concrete steps:

1. add the per-iteration write log to the FUSE layer:
   - define `fuse_iter_log_t`: a fixed-capacity array of
     `{char path[VFS_PATH_MAX], event_t kind}` entries where `event_t` is
     `LOG_CREATE | LOG_WRITE | LOG_MKDIR | LOG_RENAME_FROM | LOG_RENAME_TO | LOG_UNLINK | LOG_RMDIR | LOG_ENOENT | LOG_SYMLINK`
   - add a global `bool g_target_running` flag (false by default)
   - add logging calls in `fvfs_create`, `fvfs_write`, `fvfs_mkdir`,
     `fvfs_rename` (emits both RENAME_FROM and RENAME_TO), `fvfs_unlink`,
     `fvfs_rmdir`, `fvfs_symlink` — only when `g_target_running` is true
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

#### Part B — LibAFL Observer + Feedback

3. **Implement `FuseLogObserver`** in `mutator/src/libafl_glue/fuse_log_observer.rs`:
   - `pre_exec`: `fuse_log_set_active(true)` + `fuse_log_clear()`
   - `post_exec`: drain log, stash as `MutationGuidance` for next mutator pass

4. **Implement `FsAccessFeedback`** in `mutator/src/libafl_glue/fs_access_feedback.rs`:
   - `is_interesting = true` when log contains a never-before-seen ENOENT path
     or write-set path; tracked in a `HashSet<String>` on the feedback state

5. **Compose into the existing fuzzer**:
   ```rust
   let mut feedback = feedback_or!(
       MaxMapFeedback::tracking(&edge_observer, true, false),
       FsAccessFeedback::new()
   );
   ```

6. **Measure guidance impact**: run the same campaign with and without
   `FsAccessFeedback` active; compare coverage growth rate and time-to-crash.
   This is the core evidence that the guidance signal adds value — goes directly
   in the paper.

Testing and validation:

- write-log unit tests: verify each event kind is logged correctly when `g_target_running` is true
- suppression test: apply a delta with `g_target_running = false` and confirm no entries appear (fuzzer writes via direct VFS API must never be logged)
- deduplication test: two writes to the same path produce exactly one LOG_WRITE entry
- end-to-end test: fake target creates a file, writes to it, deletes another, and requests a missing path; verify the log captures all four event types
- feedback loop integration test: 10 guided iterations with reset, no stale state
- end-to-end guided vs unguided comparison numbers recorded in `docs/benchmark_baseline.md`

Exit criteria:

- per-iteration write log implemented in FUSE callbacks and tested
- fuzzer writes (via direct VFS API) confirmed absent from the log
- `FuseLogObserver` and `FsAccessFeedback` implemented and unit-tested
- `MutationGuidance` populated from log and consumed by mutator stages
- closed feedback loop runs without stale state on top of the LibAFL harness
- guided vs unguided comparison numbers recorded

### Week 8: Scale Snapshotting + Real-World Integration

Context: Week 8 bridges to the real target. The op vocabulary is expanding (Week 6)
and the guidance loop is live (Week 7). Two remaining blockers before any meaningful
campaign against a real OCI runtime: (1) the deep-copy snapshot restore is
O(total filesystem size) and will be a bottleneck on a full rootfs; (2) a real
baseline filesystem must be importable into the VFS for real-world integration.

#### Part A — Scale Snapshotting

Context: the current `vfs_reset_to_snapshot` deep-copies the entire tree
— O(total filesystem size). For a full container rootfs this will be a
bottleneck and must be replaced before real-world integration.

**[Evaluate before implementing] write a journal vs CoW design comparison first:**

Two approaches exist and both have real tradeoffs. Decide before writing
any restore code:

- **Journal**: each VFS mutation pushes a reverse entry; restore replays
  in reverse — O(delta size). Incremental change to existing code.
  Risk: a single wrong reverse entry produces silently corrupted state
  after reset, which is extremely hard to debug.
- **Copy-on-Write tree**: each mutation creates new nodes up the path to
  root; unchanged subtrees are shared. Save snapshot = keep root pointer
  O(1). Restore = swap root pointer O(1). Mutation cost = O(tree depth,
  typically <15). No journal to get wrong. Risk: reference counting in C
  requires discipline; it is a full VFS core refactor.

Write the comparison in `docs/vfs_design_v2.md` before implementing
either. If journal is chosen, add comprehensive journal-correctness tests
(random mutation sequences, verify post-restore state matches a known-good
deep copy). If CoW is chosen, prototype the refcounted node structure
first.

**Large file design rule (enforce whichever approach is chosen):** only
record a journal entry or create a new CoW node for files that are
actually mutated. Never proactively copy unchanged content during tree
walks. A rootfs has 50MB+ binaries — the cost of accidentally
deep-copying them on every iteration is catastrophic.

Concrete steps:

1. write journal vs CoW design comparison in `docs/vfs_design_v2.md`;
   decide and implement the chosen approach:
   - measure restore time before and after against a large synthetic tree
     (1000 files) to confirm the speedup
   - verify post-restore state matches deep-copy result for correctness
2. implement snapshot import from a host directory tree:
   - walk a real directory, create corresponding VFS nodes, set metadata
     (mode, mtime, xattrs, symlink targets)
   - this is how a container rootfs gets loaded as the concrete baseline;
     must handle all op kinds including the new ones from Week 6
3. measure restore speed against the imported rootfs baseline

#### Part B — Real-World Integration Begin

4. identify the integration point in the harness
5. perform smoke tests with an unmutated baseline rootfs — the target must execute cleanly
6. apply one small delta (including at least one symlink op) and verify the target sees the change
7. measure concurrency behaviour of the real OCI target:
   - how many processes hit the FUSE mount simultaneously during a single target run?
   - if single-threaded FUSE serialisation is measurably slowing the target, evaluate enabling FUSE multithreading (`-o clone_fd`) with a pthread rwlock around VFS access
   - if it is not a bottleneck, leave single-threaded as-is

Testing and validation:

- snapshot-create and restore equivalence checks (result must match deep-copy result) for all op kinds including new ones from Week 6
- repeated restore cycles with correctness assertions
- restore time measurement before and after optimisation
- real-target smoke tests against the mounted baseline
- new-op E2E FFI tests (one per new op kind, all passing before integration begins)
- `FuseLogObserver` extended to capture symlink and xattr access events

Exit criteria:

- journal vs CoW comparison written in `docs/vfs_design_v2.md`; approach chosen and implemented
- restore time measured before and after against a large synthetic tree
- a real rootfs baseline (including symlinks and xattrs) can be imported into the VFS
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

Weeks 1–3, the pre-Week 4 side quest, Week 4, Week 5 Phase A, Week 5 Phase B
(LibAFL integration), and the real crun fuzzing campaign are all complete.

1. **✅ DONE — Week 5 Phase A**: 9 mutator stages, live corpus, content dictionary,
   real-content perturbation, guidance threading (consumer side), 46 tests, 98% semantic yield

2. **✅ DONE — Week 5 Phase B**: LibAFL integration (`fuzz_combined_afl`), real
   crun campaign running across 6 instances (Campaign 1 + Campaign 3 with FUSE rootfs mutation),
   throughput and coverage observed over multi-day runs

3. **IMMEDIATE — Week 6 (Symlink Op + Crun-Targeted Mutators)**:
   - add `CreateSymlink { path, target }` to `FsOpKind` in `mutator/src/delta.rs`
   - add `FsOp::create_symlink(path, target)` constructor; add `target: String` field to `FsOp`
   - add Rust FFI binding in `mutator/src/ffi.rs`; wire into `apply_delta()` match arm
   - implement `replace_with_symlink(path, target)` Rust-side helper (handles non-empty dirs)
   - implement 6 crun-targeted mutators: `MountDestinationSymlinkMutator`,
     `MountOptionSymlinkMutator`, `ExecutableSymlinkMutator`, `ParentComponentSymlinkMutator`,
     `SymlinkEscapeMutator` (relative + absolute), `LoopAndDepthMutator`
   - extend `AddFileOp` and `DestructiveMutator` to emit symlink ops
   - add crun-specific seeds to `rootfs_seeds()` covering all scenario groups
   - `SetXattr`/`RemoveXattr`, `Chmod`, `Chown` deferred to Week 8

4. **NEXT — Week 7 (FUSE Logging + FuseLogObserver + FsAccessFeedback)**:
   - implement `fuse_iter_log_t` and `g_target_running` in FUSE layer
   - instrument `fvfs_create`, `fvfs_write`, `fvfs_mkdir`, `fvfs_rename`,
     `fvfs_unlink`, `fvfs_rmdir`, `fvfs_symlink`, ENOENT in `fvfs_getattr`
   - expose `fuse_log_clear()`, `fuse_log_set_active(bool)`, `fuse_log_get()`
   - implement `FuseLogObserver` and `FsAccessFeedback` in LibAFL harness
   - compose into `StdFuzzer` via `feedback_or!`
   - wire `MutationGuidance` from log into mutator stages
   - measure guided vs unguided coverage growth

5. **Week 8 — before implementing restore optimisation**: write journal vs CoW
   design comparison in `docs/vfs_design_v2.md`, decide, then implement

Remaining VFS/FUSE work — non-blocking for Week 6/7 but needed before OCI integration (Week 8):

- `chmod` / `mode` field on `vfs_node_t` — needed for permission-sensitive targets
- `release` no-op callback — flush semantics correctness
