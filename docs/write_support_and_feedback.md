# Write Support and Feedback-Guided Mutations

## Context

During a discussion with Moritz, the following direction was proposed for the
filesystem component. This document captures the idea and breaks it down into
concrete implementation tasks to be addressed in a later phase.

---

## What was proposed

The FUSE mount should be writable from the target's perspective, not just from
the fuzzer's control path. The motivation is twofold:

1. Some target programs need to write to the filesystem during execution (e.g.
   an OCI runtime setting up a container environment by creating device nodes,
   writing `/etc/hostname`, `/etc/resolv.conf`, etc.). A strictly read-only
   mount would cause these targets to fail before reaching any interesting code
   paths.

2. More importantly, when a target writes something to the filesystem — for
   example a default config file it generates on first run — that write is
   valuable information. It tells us what the target expects the filesystem to
   look like, and is therefore a great seed for future mutations.

The proposed model is:

```
1. Save a snapshot of the initial filesystem state (pre-execution)
2. Run the target — it reads and potentially writes through the FUSE mount
3. After the run, compute the diff between the current VFS state and the snapshot
4. Collect that diff (the files the target created or modified) as a new snapshot candidate
5. Feed this post-write state back to the fuzzer as a base for future mutations
6. Now the fuzzer can mutate from both the pre-write and post-write states
```

This closes a feedback loop: the fuzzer not only provides inputs to the target,
but also learns from what the target itself produces.

---

## Current state of the codebase

### Already implemented

**VFS core (`vfs/vfs.c`, `vfs/vfs.h`)**
- `vfs_save_snapshot` / `vfs_reset_to_snapshot` — deep-copy snapshot and restore
- `vfs_create_file`, `vfs_update_file`, `vfs_delete_file`, `vfs_mkdir`, `vfs_rmdir` — full control-path mutation API
- `vfs_set_times` — set `mtime` and `atime` on any node
- `mtime` and `atime` fields on `vfs_node_t` and `vfs_stat_t` — timestamps tracked, auto-updated on write, preserved across snapshot/restore

**FUSE layer (`fuse_vfs/fuse_vfs.c`)**
- `create`, `write`, `truncate`, `mkdir`, `unlink`, `rmdir`, `utimens` — all implemented and tested
- `write` handles partial writes and appends via read-modify-write on the VFS buffer
- `utimens` handles `UTIME_NOW` and `UTIME_OMIT` correctly per POSIX

What is still missing is described below.

---

## What still needs to be implemented

### 1. Remaining FUSE callbacks

| Callback   | Status | Notes                                              |
|------------|--------|----------------------------------------------------|
| `create`   | ✓ done |                                                    |
| `write`    | ✓ done | partial writes and appends handled                 |
| `truncate` | ✓ done |                                                    |
| `mkdir`    | ✓ done |                                                    |
| `unlink`   | ✓ done |                                                    |
| `rmdir`    | ✓ done |                                                    |
| `utimens`  | ✓ done | real implementation, not a no-op                   |
| `rename`   | TODO   | needs `vfs_rename` in VFS core first               |
| `chmod`    | TODO   | needs permission field on `vfs_node_t`             |
| `release`  | TODO   | can be a no-op for now; needed for flush semantics |

### 2. Remaining VFS operations

- `vfs_rename` — move a node from one path to another within the tree; needed
  for OCI runtimes that rename temp files into place
- Permission (`mode`) field on `vfs_node_t` — needed if `chmod` is to be
  supported meaningfully; currently all files get a fixed `0644` / `0755`

### 3. Delta-driven mutation model

Rather than rebuilding the filesystem tree from scratch each iteration or
deep-copying a snapshot on every reset, the fuzzer should operate on a
concrete baseline tree and send only the delta — the set of changes — for
each iteration.

The model works as follows:

```
1. Load a concrete baseline filesystem into the VFS once (e.g. a container rootfs)
2. Save a snapshot of that baseline
3. Per fuzzing iteration:
   a. Fuzzer sends a delta: a list of create / update / delete / mkdir / rmdir ops
   b. Apply the delta to the live VFS via the existing control-path API
   c. Run the target — it reads (and possibly writes) through the FUSE mount
   d. Collect any target-side writes as a diff against the pre-run snapshot
   e. Reset to the baseline snapshot for the next iteration
```

This is more efficient than rebuilding the tree from scratch because:
- Reset cost is proportional to the size of the delta, not the full tree
- Most of the baseline (unchanged files) is never touched
- The fuzzer only needs to describe what changed, not the entire state

The existing VFS API already supports steps 3a and 3b directly
(`vfs_create_file`, `vfs_update_file`, etc.). What still needs to be built
is the control plane (Week 5) that receives the delta from the fuzzer and
applies it, and the diff mechanism (below) that captures target-side writes.

**Note on snapshot scalability:** The current `vfs_reset_to_snapshot`
deep-copies the entire tree, which is O(total filesystem size). For large
layouts like a full container rootfs, this should be replaced with a
diff/journal approach where each mutating operation records what it changed
and reset replays those changes in reverse — making reset cost O(delta size)
instead. This is the right optimization to make before scaling to real-world
rootfs sizes, but is not needed until Week 8.

### 5. VFS diff in `vfs/vfs.c` and `vfs/vfs.h`

A new function `vfs_diff_snapshot` needs to be implemented. It should walk the
current VFS tree and the saved snapshot in parallel and produce a list of
changes of the form:

- file created (path, content)
- file modified (path, old content, new content)
- file deleted (path)
- directory created (path)
- directory deleted (path)

The exact representation of this diff (a linked list of change records, a
callback-based enumeration, etc.) needs to be decided. The output needs to be
consumable by the fuzzer so it can decide which post-write states to keep as
new mutation seeds.

### 6. Snapshot management for pre/post states

The current `vfs_t` struct holds a single `snapshot` pointer. To support both
pre-write and post-write states simultaneously, this may need to be extended,
or the caller (the fuzzer integration layer) may need to manage multiple `vfs_t`
instances. This design decision should be made when the fuzzer integration
(Week 5 / control path) is being implemented.

---

## Open questions

- Should writes from the target be reflected back into the fuzzer's mutation
  pool automatically, or should there be an explicit API call to "promote" a
  post-write state to a snapshot?
- How should conflicts be handled if the target deletes a file the fuzzer
  intended to mutate in the next round?
- For the OCI runtime case specifically: does the runtime write to the rootfs
  in a way that is part of the interesting input surface, or are those writes
  just setup noise that should be discarded before the next run?

---

## Related files

- [vfs/vfs.c](../vfs/vfs.c) — VFS core, snapshot save/restore already here
- [vfs/vfs.h](../vfs/vfs.h) — public API, needs new function signatures
- [fuse_vfs/fuse_vfs.c](../fuse_vfs/fuse_vfs.c) — FUSE frontend, needs write callbacks
- [docs/vfs_design_v1.md](vfs_design_v1.md) — original VFS design notes
