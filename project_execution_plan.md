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

The following phase is treated as completed, but it still needs to be preserved properly as a baseline:

- a simple FUSE filesystem has been built
- a benchmark was written to perform repeated open-read-close cycles
- the measured throughput is about `14k ops/sec`
- the current result comfortably exceeds the minimum target from the spec

This means the next major effort should shift toward building the actual in-memory filesystem backend and then integrating it with the fuzzing side.

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

### Week 3: Expose The VFS Through FUSE

Objectives:

- replace the toy counter backend with the VFS backend
- get a mounted read-only VFS working cleanly
- confirm the benchmark still stays in an acceptable range

Concrete steps:

1. wire FUSE callbacks to the VFS core
2. implement and validate:
   - getattr
   - readdir
   - open
   - read
3. mount a filesystem with one directory and one file
4. verify shell behavior using `ls`, `cat`, repeated opens, and nested directory reads
5. rerun the benchmark against the VFS-backed implementation

Testing and validation:

- manual shell validation
- one small C integration test for read correctness
- negative tests for nonexistent paths
- benchmark comparison against the counter version

Exit criteria:

- mounted read-only VFS works reliably
- benchmark remains practically usable for fuzzing

### Week 4: Add Runtime Mutation And Reset Support

Objectives:

- make the mounted filesystem update without remounting
- make per-iteration reset safe and deterministic
- finish the minimal VFS feature set needed for fuzzing

Concrete steps:

1. implement batch mutation application on the live VFS
2. define atomicity semantics for a testcase update
3. ensure mounted readers never observe half-applied state
4. implement reset from baseline snapshot
5. add multi-file support if the current VFS does not already support it
6. build a tiny local driver that changes content and verifies the mounted filesystem updates

Testing and validation:

- batch update tests
- conflicting operation tests
- repeated mutate-read-reset cycles
- stale-state regression tests
- multi-file visibility tests

Exit criteria:

- the mounted filesystem can be updated and reset safely without remounting

### Week 5: Build The Control Plane And Freeze The Mutation Model

Objectives:

- give the VFS a stable external mutation interface
- finalize what a testcase means for the first LibAFL integration
- avoid overdesigning the search space

Concrete steps:

1. choose the control path:
   - in-process API if feasible
   - Unix domain socket if process separation is needed
2. define the message format for:
   - replace testcase state
   - reset to baseline
   - health/status
3. write `mutation_model.md`
4. freeze the first testcase model as:
   - one fixed-path file with mutable bytes, or
   - a small fixed set of files with mutable bytes
5. explicitly postpone path-generation-heavy mutation unless it is clearly needed for the first milestone

Testing and validation:

- malformed message tests
- valid update plus mounted read verification
- reset after several updates
- review of the mutation model against the demo target

Exit criteria:

- external mutation interface works
- mutation model is stable enough to implement in LibAFL

### Week 6: Build The Minimal End-To-End Demo Harness

Objectives:

- create the smallest possible filesystem-backed fuzzing demo
- keep the scope narrow enough that Week 7 can be used for stabilization, not first-time debugging

Concrete steps:

1. write a tiny target program that reads a mounted file
2. crash when the file contains `foobar`
3. implement the minimal LibAFL harness
4. apply each testcase to the VFS before each execution
5. verify the harness sees seeded crashing and non-crashing inputs correctly

Testing and validation:

- direct manual crash reproduction outside the fuzzer
- harness run with a crashing seed
- harness run with a near-miss input
- multiple consecutive executions with reset between runs

Exit criteria:

- the harness can repeatedly run the target while updating the mounted filesystem

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

### Week 8: Add Snapshotting And Begin Real-World Integration

Objectives:

- finish the MVP feature that realistic targets need
- start integration with the container-runtime setup without waiting for perfect polish

Concrete steps:

1. implement snapshot import from a directory tree or internal serialized state
2. implement restore into the live VFS
3. measure restore speed
4. identify the integration point in Moritz's harness
5. perform smoke tests with an unmutated baseline rootfs
6. mutate one harmless file and verify the target sees it

Testing and validation:

- snapshot-create and restore equivalence checks
- repeated restore cycles
- real-target smoke tests against the mounted filesystem

Exit criteria:

- snapshotting works
- the real target can at least execute against the mounted baseline

### Week 9: Real-World Campaign Bring-Up And Initial Evaluation

Objectives:

- move from integration smoke tests to a usable campaign
- start collecting real data early enough to react if something breaks

Concrete steps:

1. connect the mutation flow to the real target setup
2. run short controlled fuzzing sessions
3. record execution throughput, reset cost, and obvious bottlenecks
4. debug reproducibility issues immediately
5. compare against a naive baseline if possible, such as rebuilding a temp directory each iteration

Testing and validation:

- repeated short campaigns from a clean baseline
- reproducible failures or crashes
- saved scripts for measurement and reruns

Exit criteria:

- the real-world setup runs repeatedly under harness control
- initial evaluation numbers exist

### Week 10: Final Evaluation, Hardening, And Writeup Support

Objectives:

- collect the strongest data possible within the remaining time
- harden the pipeline enough that the results are defensible
- leave the repository in a state another person can run

Concrete steps:

1. run the final benchmark suite:
   - open-read-close baseline
   - VFS-backed read throughput
   - mutation application cost
   - reset cost
   - real-target throughput
2. run longer fuzzing sessions if compute time allows
3. add missing regression tests for discovered bugs
4. improve logging and reproducibility documentation
5. prepare architecture notes, benchmark methodology, and result summaries

Testing and validation:

- multiple repeated performance runs
- confirmation that major demos still work after hardening changes
- final rerun of the first milestone and the real-world integration path

Exit criteria:

- enough evidence exists to support the claims
- the implementation is stable enough for demonstration and writeup

## 7. Bonus Plan If Time Permits

The bonus direction should only begin after the main pipeline is stable, reproducible, and evaluated.

### Bonus Week 1: Observe File Access Behavior

Objectives:

- record which files are accessed by the target during execution
- understand whether this information is useful for mutation guidance

Concrete steps:

1. instrument the FUSE layer to log lookups, opens, reads, and failures
2. record missing-path accesses as well as successful accesses
3. aggregate access counts per execution
4. inspect whether targets repeatedly request specific files that do not exist

Validation:

- logs are correct and do not corrupt normal execution
- overhead introduced by observation is measured

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

Given the current status, the best immediate next sequence is:

1. preserve the benchmark result in a short benchmark note
2. write the VFS v1 design note
3. implement the standalone in-memory VFS core with tests
4. replace the toy counter backend with VFS-backed FUSE reads
5. only then add mutation updates and LibAFL integration

That order gives the best chance of keeping the system correct while complexity rises.
