# Control Plane Design Note

## Purpose

This document describes the Week 4 control plane: the bridge between the
fuzzer and the live VFS.  It covers the transport choice, the wire protocol,
the apply algorithm, and known limitations.

**Current status: Week 4 implementation complete.**
All components implemented in `control_plane/`; 224 tests pass.

---

## Transport: In-Process Shared-Library Call

The fuzzer and the VFS run in **the same process**.  The control plane is a
C library (`libcontrol_plane.a`) linked directly into the fuzzer binary.
Applying a delta is a synchronous function call — no IPC, no sockets, no
serialisation overhead at apply time.

**Why in-process?**

- Lowest possible latency per iteration (no syscall boundary crossing for
  the apply step itself).
- Simple to debug: the full VFS state is inspectable with a debugger.
- Sufficient for the demo target (Week 6) and real-world integration (Week 8).

**Extension path:** A Unix domain socket transport is the obvious upgrade if
process separation is ever required (e.g. the target program must run in a
separate process with a different UID, or the VFS must outlive the fuzzer
crash).  The `cp_apply_delta` API signature is transport-agnostic from the
caller's point of view; only the implementation would change.

---

## Apply Algorithm

```
cp_apply_delta(vfs, delta, dry_run):

  Phase 1 — non-RMDIR ops in original order:
    for each op in delta where op.kind != RMDIR:
      if op.kind == CREATE_FILE or MKDIR:
        cp_ensure_parents(vfs, op.path)   /* create missing intermediate dirs */
      r = apply_single_op(vfs, op)
      if r == EEXIST and op.kind == MKDIR:
        treat as success                  /* dir already exists; intent satisfied */
      record r in results[i]

  Phase 2 — RMDIR ops, deepest first:
    collect indices of all RMDIR ops
    sort by path depth descending (count of '/' chars)
    for each RMDIR op in sorted order:
      r = vfs_rmdir(vfs, op.path)
      record r in results[i]

  if dry_run:
    print resulting VFS tree via cp_dump_vfs()
    vfs_reset_to_snapshot(vfs)    /* requires a snapshot to have been saved */
```

The per-op result (`cp_op_result_t`) records the VFS error code and a static
message string.  The batch result (`cp_result_t`) accumulates succeeded and
failed counts.  Callers inspect individual op errors to decide whether a
partial failure is acceptable.

---

## Dry-Run Mode

`cp_apply_delta(vfs, delta, 1 /* dry_run */)` is a diagnostic tool for
eyeballing what a delta would produce before committing to a campaign.

**Contract:**

- The delta is applied to the live VFS (same as a normal apply).
- The resulting tree is printed to stdout via `cp_dump_vfs()`.
- `vfs_reset_to_snapshot()` is called to undo the apply.
- **A snapshot must have been saved first** (via `vfs_save_snapshot()`).
  If no snapshot exists, the apply is permanent and a warning is printed to
  stderr.

**Usage pattern:**

```c
vfs_t *vfs = vfs_create();
/* ... populate baseline ... */
vfs_save_snapshot(vfs);               /* save baseline */

cp_apply_delta(vfs, delta, 1);        /* dry-run: apply, print, restore */
/* vfs is back to baseline here */

/* Resume normal fuzzing with vfs_save_snapshot still valid. */
```

---

## Snapshot Management

The current `vfs_t` holds a **single snapshot slot** (`vfs->snapshot`).
The baseline is saved once before the fuzzing loop starts.

The feedback loop (Week 5) needs **two snapshots simultaneously**:
1. The baseline (for end-of-iteration reset).
2. A pre-run snapshot (for `vfs_diff_snapshot` to capture target writes).

**Decision deferred to Week 5:** Two options exist:

- **Option A**: Add a second named snapshot slot to `vfs_t` (e.g., `pre_run_snapshot`).
  Minimal change to the VFS API; the control plane manages both slots.
- **Option B**: Manage two separate `vfs_t` instances at the caller level.
  The baseline is a second VFS; the fuzzer resets by deep-copying from it.
  Cleaner API boundary but doubles the memory footprint.

Document the chosen approach in an update to this file at the start of Week 5.

---

## `ensure_parents()` Implementation

```c
int cp_ensure_parents(vfs_t *vfs, const char *path);
```

For a path like `/a/b/c.txt`:

1. Duplicate path as a working buffer.
2. Walk the string character by character.
3. Each time a `/` is found (after position 0), temporarily NUL-terminate
   there, call `vfs_mkdir(vfs, buf)`, restore the `/`, continue.
4. EEXIST from `vfs_mkdir` is silently ignored.

This is O(path_depth) VFS calls, each O(path_depth) for path resolution.
Total cost: O(path_depth²) — negligible for the path depths in a fuzzing
context (typically < 10 levels).

---

## VFS Checksum for Baseline Tagging

```c
uint64_t cp_vfs_checksum(vfs_t *vfs);
```

Walks the VFS tree in readdir order (insertion order) via `vfs_readdir`,
hashing paths, content, symlink targets, and timestamps using FNV-1a 64-bit.

A crash testcase is stored with:
- The serialized delta (`delta_serialize` → file on disk).
- The VFS checksum at snapshot time (`cp_vfs_checksum` on the baseline).

Reproduction: load the same baseline, verify its checksum matches, apply the
delta, confirm crash.

See `docs/mutation_model.md` for full checksum details and known limitations.

---

## Week 5 TODO (Control Plane Extensions)

The following extensions are planned for Week 5:

1. **`vfs_diff_snapshot`** — compare current VFS state against a saved
   snapshot and produce a structured change list (file created, modified,
   deleted; directory created/deleted).  This is how target-side writes are
   captured as new seeds.

2. **Second snapshot slot** — add a pre-run snapshot mechanism so the
   baseline and the pre-run state can coexist.

3. **Iteration harness loop** — wire up the full per-iteration cycle:
   ```
   save pre-run snapshot
   cp_apply_delta(vfs, delta, 0)
   run_target()
   vfs_diff_snapshot(current, pre_run) → new_seed if non-empty
   vfs_reset_to_snapshot(vfs)  /* restore baseline */
   ```

4. **Reset cost measurement** — instrument `vfs_reset_to_snapshot` with a
   wall-clock timer; record per-reset latency in `docs/benchmark_baseline.md`.
