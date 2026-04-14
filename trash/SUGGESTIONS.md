# Suggestions and Improvements for VFS-Fuzz

This document captures architectural suggestions, design alternatives, and
concrete improvement tasks identified during a deep review of the project
specification, VFS design, execution plan, and write-support/feedback proposal.

It is organized by topic area, with each suggestion marked by priority and
roughly mapped to the week it should be addressed.

**Current status assumed:** Weeks 1–3 complete. Symlinks and `vfs_rename`
have been incorporated. The next work is Week 4 (mutation model + control
plane).

---

## 1. Rethink the LibAFL Testcase Representation

**Priority:** High — affects Week 4 design decisions and everything downstream.

### Problem

The current plan defines `fs_delta_t` as a structured list of `fs_op_t`
entries and proposes 6+ custom LibAFL mutator stages that operate on this
structure (`ByteFlipFileContent`, `ReplaceFileContent`, `AddFileOp`,
`RemoveOp`, `MutatePath`, `SpliceDelta`). This approach throws away all of
LibAFL's built-in mutation power — havoc, splice, minimization, scheduling —
because none of those can operate on an opaque struct they don't understand.

You're committing to building and maintaining a parallel mutation
infrastructure for a novel input type. In a 10-week project, that's a lot of
surface area for bugs.

### Suggested alternative: hybrid byte-buffer + structure-aware mutators

Define a compact binary serialization of the delta and register the
*serialized byte buffer* as the LibAFL `Input` type. The iteration loop
becomes:

```
1. LibAFL hands you a byte buffer (the testcase)
2. Deserialize it into fs_delta_t
3. If deserialization fails, skip this iteration (the mutation produced garbage)
4. Apply the delta to the VFS
5. Run the target
6. Reset
```

The key insight: design the serialization format so that **file content lives
in contiguous byte regions** within the buffer. A standard AFL byte flip that
lands inside a file's content region does something useful — it mutates the
content. A flip that lands on a path byte or op-kind byte produces garbage
that the deserializer rejects. This is the standard tradeoff in grammar-aware
fuzzing and it works well in practice because AFL runs at enormous speed.

On top of this, add 3–4 structure-aware custom mutators for the high-value
operations:

- `AddFileOp` — append a well-formed create/mkdir op
- `RemoveOp` — drop a random op (shrink the testcase)
- `SpliceDelta` — combine ops from two deltas

This gives you LibAFL's full byte-level mutation power on file contents for
free, plus targeted structural mutations where they matter. Much less code to
write and maintain.

### Serialization format sketch

```
[header: num_ops (u32)]
[op 0: kind (u8) | path_len (u16) | path_bytes | content_len (u32) | content_bytes]
[op 1: kind (u8) | path_len (u16) | path_bytes | content_len (u32) | content_bytes]
...
```

File content is byte-aligned and contiguous, so AFL's standard mutations
naturally target it. The format is simple enough that deserialization
validation is cheap.

### Action items

- [ ] Before implementing `fs_delta_t`, evaluate the hybrid approach
- [ ] Design the serialization format and document it in `docs/mutation_model.md`
- [ ] Prototype deserialization + validation to measure rejection rate on
      random byte mutations (aim for <80% rejection — if too high, the format
      needs adjustment)
- [ ] Reduce the custom mutator count from 6+ to 3–4

---

## 2. Replace Write-Diffing Feedback with Access-Pattern Tracing

**Priority:** High — simplifies Week 5 significantly and produces better
signal.

### Problem with the current feedback model

The `write_support_and_feedback.md` proposes:

1. Target writes to the FUSE mount during execution
2. Diff the post-execution VFS against a pre-execution snapshot
3. Promote the post-write state as a new mutation seed

The open questions section already flags the core issue: in the OCI runtime
case, target writes are mostly **setup noise** — `/dev/null`, `/dev/zero`,
`/etc/hostname`, `/etc/resolv.conf`, cgroup control files. Promoting all of
that into the corpus floods the mutation pool with seeds that don't help find
bugs.

The diff-based approach also requires significant machinery:
- `vfs_diff_snapshot` implementation (parallel tree walk, structured change list)
- Dual-snapshot management (baseline + pre-run, simultaneously)
- Promotion logic deciding which post-write states to keep
- Filtering heuristics to avoid corpus pollution

### Suggested alternative: access-pattern tracing

Instead of capturing what the target **wrote**, capture what it **tried to
read**. The FUSE layer already intercepts every filesystem call. Instrument
it to record:

| Event | What it tells the fuzzer |
|---|---|
| Successful open/read | Which files are "hot" — mutations to these are more likely to trigger new behavior |
| Failed lookup (ENOENT) | Which paths the target expects but can't find — creating these files likely unlocks new code paths |
| Failed permission check (EACCES) | The target tried to access something it couldn't — try chmod mutations |
| Directory listing (readdir) | Where the target looks for files — good place to create new ones |

This feedback is strictly more useful because it captures the target's
**intent**, not its **side effects**.

### Concrete implementation

```
Per iteration:
1. Clear the access log (a simple array of {path, event_type, count} entries)
2. Apply the fuzzer's delta
3. Run the target (FUSE callbacks log accesses into the array)
4. After execution, extract:
   - hot_files: files that were read (ranked by access count)
   - missing_paths: paths that got ENOENT
   - listed_dirs: directories that were enumerated
5. Feed this metadata back to the mutator as guidance:
   - bias content mutations toward hot_files
   - bias file-creation mutations toward missing_paths
   - bias new-file placement toward listed_dirs
6. Reset
```

### What this replaces

- `vfs_diff_snapshot` — **not needed**
- Dual-snapshot management — **not needed** (single baseline snapshot is sufficient)
- Post-write state promotion logic — **not needed**
- Corpus pollution filtering — **not needed**

Target writes should still be **enabled** (the target needs to write to not
crash), but treated as **ephemeral** — they exist during the iteration and
are wiped on reset. No diffing, no promotion.

### What this adds

- A per-callback logging hook in the FUSE layer (lightweight — a few lines per
  callback)
- A structured access log that gets cleared each iteration
- Mutator guidance logic that biases mutation selection based on access data

This is less code, less complexity, and more actionable signal than the
write-diffing approach.

### Action items

- [ ] Add an access-log structure to the FUSE context (array of path + event
      type + count, cleared per iteration)
- [ ] Instrument `getattr`, `open`, `read`, `readdir` callbacks to log
      accesses
- [ ] Instrument failed lookups to separately log ENOENT paths
- [ ] Expose the access log to the control plane so the fuzzer can read it
      after each iteration
- [ ] Design the mutator guidance interface (which fields the mutator reads,
      how they influence mutation selection)
- [ ] Drop `vfs_diff_snapshot` from the Week 5 plan
- [ ] Drop dual-snapshot management from the plan

---

## 3. Fix the Delta Ordering Problem

**Priority:** High — must be decided before implementing the control plane in
Week 4.

### Problem

A flat list of ops has implicit ordering dependencies. If the fuzzer generates
a delta with `CREATE_FILE /a/b/c` before `MKDIR /a/b`, application will fail
because the parent doesn't exist. Similarly, `RMDIR /a` before
`DELETE_FILE /a/foo` will fail because the directory isn't empty.

This problem gets worse with the hybrid byte-buffer approach (suggestion 1),
because AFL's mutations can reorder ops by splicing bytes around.

### Two options

**Option A: Enforce topological ordering in the delta.**
The control plane sorts ops before applying: mkdirs in depth-first order,
creates after their parent dirs exist, deletes in reverse depth order, rmdirs
after their children are removed. This keeps the delta format simple but
requires a sort pass on every application.

**Option B: Auto-create intermediate directories on the VFS side.**
When applying a `CREATE_FILE /a/b/c`, if `/a/b` doesn't exist, create it
implicitly. When applying `RMDIR /a`, recursively remove children first. This
is simpler for the mutator (no ordering constraints) but changes VFS
semantics.

### Recommendation

Go with **Option B** in the control plane layer, not in the VFS core. The VFS
core should keep its strict semantics (parent must exist, rmdir requires
empty). The control plane receiver should handle the fixup:

```c
// In the control plane, not in vfs.c:
int apply_create_file(vfs_t *vfs, const char *path, const uint8_t *content, size_t len) {
    // Ensure all parent directories exist
    ensure_parents(vfs, path);
    // Try create, fall back to update if it already exists
    int rc = vfs_create_file(vfs, path, content, len);
    if (rc == -EEXIST)
        rc = vfs_update_file(vfs, path, content, len);
    return rc;
}
```

This keeps the VFS core clean and testable while making the fuzzer's life
easy.

### Action items

- [ ] Document the ordering strategy in `docs/mutation_model.md`
- [ ] Implement `ensure_parents()` helper in the control plane
- [ ] Implement `apply_delta()` with auto-fixup in the control plane
- [ ] Add tests for out-of-order deltas (create before parent mkdir, etc.)

---

## 4. Consider Copy-on-Write Tree Nodes Instead of Journal-Based Restore

**Priority:** Medium — affects Week 8 but should be evaluated during Week 4
design.

### Current plan

Deep-copy snapshots (Weeks 2–7) → journal-based restore (Week 8). The journal
approach records a reverse entry for every VFS mutation and replays the
journal in reverse on restore.

### Problem with journals

Journals add complexity to every mutation path. Every `vfs_create_file`,
`vfs_update_file`, `vfs_delete_file`, `vfs_mkdir`, `vfs_rmdir` must push a
correct reverse entry. If any reverse entry is wrong, restore produces a
silently corrupted state that's incredibly hard to debug. Journal bugs are
the kind of thing that causes "the fuzzer ran for 48 hours and found nothing
because the filesystem was wrong after the first reset."

### Alternative: persistent (copy-on-write) tree nodes

Each VFS node is reference-counted. When you mutate a node, you create a new
node with the change and new parent nodes up the path to the root (path
copying). Unchanged subtrees are shared.

- **Save snapshot** = keep a reference to the current root. O(1).
- **Restore** = switch the active root pointer back. O(1).
- **Mutation cost** = O(depth of changed path), typically bounded by
  filesystem depth (rarely >15 levels).
- **Memory** = shared unchanged subtrees, so overhead is proportional to
  delta size, not total tree size.

For a container rootfs with 10,000 files where the fuzzer changes 3 files per
iteration, this creates ~45 new nodes per mutation (3 paths × ~15 levels)
instead of deep-copying 10,000 nodes on restore. And there's no journal to
get wrong.

### Tradeoffs

The catch is implementation complexity in C. You need reference counting,
careful memory management, and immutable node semantics. In Rust this would
be natural with `Arc<Node>`. In C it's doable but requires discipline.

### Recommendation

At minimum, evaluate this as a design option during Week 4 and document the
tradeoff in `docs/vfs_design_v2.md`. If the journal approach is chosen,
document why. If CoW is chosen, plan for a VFS core refactor in Week 8.

### Action items

- [ ] Write a short design comparison: journal vs. CoW, with pros/cons for
      this project
- [ ] If journal is chosen, add comprehensive journal-correctness tests
      (fuzz the journal itself: random mutation sequences, verify
      post-restore state matches a known-good deep copy)
- [ ] If CoW is chosen, prototype the refcounted node structure and
      path-copying logic

---

## 5. Add Metadata-Only Mutator Stages

**Priority:** Medium — should be included in Week 5 mutator implementation.

### Problem

The proposed mutator stages are all content-oriented. But the spec explicitly
calls out mtime, atime, and `vfs_set_times` is already implemented. Real
programs check timestamps and permissions in ways that trigger interesting
bugs.

### Missing mutators

**`MutateTimestamp`** — randomize mtime/atime on an existing file. OCI
runtimes and build systems use make-style freshness checks. Cache validation
logic often branches on whether mtime is before or after some threshold.
Setting mtime to 0, to a future date, or to match another file's mtime can
trigger unexpected behavior.

**`MutatePermissions`** (once the mode field is added) — change file
permissions. Targets that check access bits before reading will hit different
error paths. Setting a file to 0000, or a directory to 0644 (not executable,
so not traversable), exercises permission-checking code.

**`MutateFileSize`** — truncate or extend a file without changing (or
zeroing) its content. Programs that `stat()` before `read()` and allocate
based on `st_size` can be tripped up by size/content mismatches.

### Action items

- [ ] Add `SET_TIMES` as an op kind in `fs_op_t` with `mtime` and `atime`
      fields
- [ ] Add `CHMOD` as an op kind once the mode field exists
- [ ] Add `TRUNCATE` as an op kind (set file size without full content
      replacement)
- [ ] Implement corresponding mutator stages

---

## 6. Measure FUSE Overhead in Isolation

**Priority:** Medium — useful for the paper and for guiding optimization.

### Problem

The benchmark shows 13.8k ops/sec through FUSE, but there's no measurement
of how much of that time is kernel FUSE overhead vs. VFS work. Without this
breakdown, you can't know where to spend optimization effort.

### What to measure

Run the same read workload directly against the VFS API (no FUSE, just C
function calls). Compare:

| Path | Expected ops/sec | What it tells you |
|---|---|---|
| Direct VFS API calls | Likely 200k–500k+ | Pure VFS cost |
| FUSE-mounted reads | ~13.8k | VFS + kernel FUSE + context switches |

If direct VFS is 500k and FUSE is 14k, then ~97% of time is in the FUSE
kernel path and optimizing VFS internals won't help. If direct VFS is 20k,
then VFS is the bottleneck and worth optimizing.

This number is also valuable for the paper — it quantifies the FUSE overhead
tax and helps justify future work on kernel bypass if needed.

### Action items

- [ ] Write a small benchmark that calls `vfs_read` in a tight loop (no
      FUSE, no mount, just the C API)
- [ ] Run it alongside the FUSE benchmark on the same machine
- [ ] Record the ratio in `docs/benchmark_baseline.md`

---

## 7. Add a NyxFuzz Comparison to the Evaluation

**Priority:** Medium — important for the paper, should be planned during
Week 9.

### Problem

The spec explicitly mentions "Comparison with other snapshotting systems
(NyxFuzz first)." The execution plan's Week 9 only mentions comparing against
"rebuilding a temp directory each iteration." That's too weak a baseline for
a conference paper.

### What to compare

At minimum, compare published NyxFuzz throughput numbers against yours on
similar workloads. If you can run NyxFuzz yourself, a direct comparison on
the same target is much stronger.

Also compare against **tmpfs + rsync** — mount a tmpfs, rsync the rootfs
into it each iteration, run the target, unmount. This is what a practitioner
would actually do without your tool. If your FUSE-backed approach isn't
significantly faster than tmpfs + rsync, the contribution story gets harder.

### Comparison matrix

| Baseline | What it shows |
|---|---|
| tmpfs + rsync per iteration | Naive practitioner approach |
| tmpfs + cp -a per iteration | Slightly faster naive approach |
| NyxFuzz (published numbers) | State-of-the-art snapshot fuzzing |
| Your system (deep-copy restore) | Before optimization |
| Your system (journal/CoW restore) | After optimization |

### Action items

- [ ] Add tmpfs + rsync and tmpfs + cp baselines to the benchmark suite
- [ ] Collect NyxFuzz published throughput numbers for comparable workloads
- [ ] If feasible, set up NyxFuzz on the same machine for direct comparison
- [ ] Document comparison methodology in `docs/evaluation_plan.md`

---

## 8. Handle Large Files Efficiently

**Priority:** Medium — matters for Week 8 real-world integration.

### Problem

The VFS stores file content as raw byte arrays in memory. A real container
rootfs contains binaries that are megabytes each (`/usr/bin/python3` can be
50MB+). Even with journal-based restore, if the journal records "this is
what the file looked like before the mutation" for every touched file, you're
copying multi-megabyte buffers.

### Recommendation

The journal (or CoW approach) must distinguish between "this file was
modified" and "this file was not touched." Only modified files should have
their original content stored in the journal. Files the fuzzer never mutates
should be zero-cost to restore.

For the CoW approach this is automatic — unchanged subtrees are shared. For
the journal approach, the key rule is:

```
Only push a journal entry when a VFS mutation function is actually called.
Never proactively snapshot unchanged files.
```

This sounds obvious, but it's easy to accidentally deep-copy unchanged
content during tree walks or snapshot initialization.

Additionally, consider **content deduplication** for the baseline import.
Many files in a rootfs are identical across packages (e.g., license files,
empty `__init__.py`). Storing content by hash and sharing identical buffers
reduces baseline memory.

### Action items

- [ ] Verify that the journal only records entries for actually-mutated files
- [ ] Measure peak memory usage when loading a real rootfs baseline
- [ ] If memory is a problem, implement content-addressed storage (store
      content by SHA-256, share identical buffers)
- [ ] Add a large-file stress test: create a 10MB file in the VFS, snapshot,
      mutate a different file, restore, verify the 10MB file was not copied

---

## 9. Make Testcases Deterministically Replayable

**Priority:** Medium — important for bug reporting and paper credibility.

### Problem

When the fuzzer finds a bug in the OCI runtime, a reviewer will want to see
"here is the exact filesystem state that triggered the crash." If the
testcase is a delta and the delta depends on a baseline imported from a
directory tree, reproducing the bug requires both the delta AND the exact
baseline snapshot.

### Recommendation

- Version or checksum the baseline snapshot at import time (e.g., SHA-256 of
  the directory tree's serialized representation)
- Store the baseline checksum alongside every saved testcase
- Implement a replay tool: `vfs_replay <baseline_dir> <delta_file>` that
  imports the baseline, applies the delta, mounts the result, and optionally
  runs the target
- Make sure the serialized delta format is self-contained (no references to
  external state beyond the baseline)

### Dry-run mode

Also add a **dry-run mode** to the control plane where you apply a delta and
dump the resulting filesystem tree to stdout or a log without running a
target. This is invaluable for debugging mutation quality — you can eyeball
whether the mutator is producing reasonable filesystems or just noise.

### Action items

- [ ] Add a baseline checksum to the snapshot metadata
- [ ] Store baseline checksum in saved testcases
- [ ] Build a minimal replay tool (`apply_and_dump` or similar)
- [ ] Add a `--dry-run` flag to the control plane that prints the VFS tree
      after delta application

---

## 10. Plan for Concurrency Before the OCI Integration

**Priority:** Low-medium — not needed until Week 8 but should be evaluated
earlier.

### Problem

The VFS design says "single-threaded for v1, no locking required." The FUSE
layer uses single-threaded dispatch. This is fine for the Week 6–7 toy demo.

But OCI runtimes are multi-process. `runc` forks, the container init process
runs, and both may hit the FUSE mount from different processes simultaneously.
Single-threaded FUSE will serialize all requests through its dispatch loop,
which means the target processes will block on each other's filesystem
operations.

### Recommendation

Don't add locking to the VFS core now. But during Week 8 integration:

1. Measure whether single-threaded FUSE is a bottleneck for the real target
   (if the target only does a few filesystem calls, it won't matter)
2. If it is, enable FUSE multithreading (`-o clone_fd`) and add a
   read-write lock around VFS access (readers for FUSE callbacks, writer for
   control-plane mutations)
3. Measure the overhead of locking vs. serialization

The read-write lock is cheap if reads dominate (they will — the target mostly
reads, the fuzzer mutates once per iteration). The main risk is ensuring
the control-plane mutation path is not concurrent with target reads, which
the fuzzing loop structure already guarantees (mutate → run → reset is
sequential).

### Action items

- [ ] During Week 8 integration, measure target filesystem call patterns
      (how many concurrent processes, how many FS calls per run)
- [ ] If serialization is a bottleneck, add a pthread rwlock to `vfs_t`
- [ ] Benchmark with and without multithreaded FUSE

---

## 11. Pull the Journal/CoW Optimization Forward

**Priority:** Low-medium — the plan says Week 8, but earlier is better.

### Problem

The plan defers journal-based (or CoW-based) restore to Week 8. But
Weeks 5–7 involve running the full mutation loop repeatedly. Deep-copying
the entire tree on every reset during these weeks will silently eat
throughput and give misleading performance numbers.

For the toy demo with a few files, deep copy is fine. But if you test with
anything larger (even a synthetic tree of 100 files), the numbers will be
wrong.

### Recommendation

You don't need the final optimized implementation early. But at minimum,
**measure deep-copy restore cost** during Week 5 testing and record it. If
it's >1ms per reset, consider stubbing out the journal infrastructure early
so Week 6–7 numbers are representative.

### Action items

- [ ] Add a timer around `vfs_reset_to_snapshot` in the iteration loop
- [ ] Record per-reset cost in benchmark results
- [ ] If reset cost exceeds 1ms for the demo tree size, evaluate pulling
      the optimization forward

---

## 12. Week 5 Scope Reduction

**Priority:** High — schedule risk mitigation.

### Problem

Week 5 as currently planned is overloaded:

- Implement `vfs_diff_snapshot` (parallel tree walk, structured change list)
- Design dual-snapshot management
- Implement 6 mutator stages
- Wire the full per-iteration feedback loop
- Test everything

If suggestions 1 and 2 above are adopted, Week 5 becomes much more
manageable:

- No `vfs_diff_snapshot` (replaced by access-pattern tracing, which is a few
  logging hooks)
- No dual-snapshot management (single baseline snapshot is sufficient)
- 3–4 mutator stages instead of 6+ (byte-level mutations handled by LibAFL)
- The "feedback loop" is just logging, not diffing and promoting

### Revised Week 5 scope

1. Implement 3–4 structure-aware mutator stages (`AddFileOp`, `RemoveOp`,
   `SpliceDelta`, `MutateTimestamp`)
2. Add access-pattern logging hooks to FUSE callbacks
3. Expose the access log to the control plane
4. Wire the basic per-iteration loop: apply delta → run target → read access
   log → reset
5. Test each mutator stage in isolation

This is a realistic one-week scope. The access-log-based guidance can be
wired into the mutator selection logic in a later week if time permits.

---

## Summary: Priority-Ordered Task List

### Must-do before or during Week 4

1. Evaluate hybrid byte-buffer testcase representation (Section 1)
2. Decide and document delta ordering strategy (Section 3)
3. Write design comparison: journal vs. CoW snapshots (Section 4)

### Must-do during Week 5

4. Implement access-pattern logging in FUSE layer (Section 2)
5. Implement 3–4 structure-aware mutator stages (Section 12)
6. Add metadata-only mutator stages — at least `MutateTimestamp` (Section 5)

### Must-do before Week 8

7. Measure FUSE overhead in isolation (Section 6)
8. Measure deep-copy restore cost and decide whether to pull optimization
   forward (Section 11)
9. Add large-file handling tests (Section 8)

### Must-do during Weeks 8–9

10. Add baseline checksumming and replay tool (Section 9)
11. Add tmpfs + rsync comparison baseline (Section 7)
12. Evaluate concurrency needs for OCI target (Section 10)

### Must-do for the paper

13. Collect NyxFuzz published numbers for comparison (Section 7)
14. Record FUSE overhead ratio (direct VFS vs. mounted) (Section 6)
