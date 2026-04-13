# VFS v1 Design Note

## Purpose

This document defines the scope, data model, and operation set for the
in-memory virtual filesystem (VFS). It was written before Week 2 implementation
began and is kept up to date as the implementation evolves.

**Current status: Weeks 1–3 and the pre-Week 4 side quest are complete.**
Sections that describe original design intent are annotated where the
implementation diverged or expanded beyond the original scope.

---

## Goals

- Build a filesystem state model that lives entirely in memory — no disk I/O,
  ever.
- Keep the VFS core free of any FUSE or kernel dependency so it can be unit
  tested independently.
- Cover exactly the operations needed to support the fuzzing loop: load a
  baseline state, apply mutations between iterations, reset to baseline between
  iterations, and serve reads (and writes) to a mounted target program.
- Keep the implementation small enough to be finished and fully tested in one
  week per phase.

---

## Originally Deferred — Current Status

The following were listed as non-goals in the original v1 design. Each is now
annotated with its current status.

| Originally deferred | Current status |
|---------------------|----------------|
| Symlinks | **Implemented** — `VFS_SYMLINK` kind, `vfs_symlink`, `vfs_readlink`, `fvfs_symlink`, `fvfs_readlink` |
| Timestamps (mtime, atime) | **Implemented** — real `clock_gettime` values; auto-updated on write; `vfs_set_times` for fuzzer control; `fvfs_utimens` with `UTIME_NOW`/`UTIME_OMIT` |
| Arbitrary writes from target | **Implemented** — FUSE mount is fully read-write from the target's perspective |
| Hard links | Still deferred |
| Extended attributes (xattrs) | Still deferred |
| Ownership, uid/gid, permission checking | Still deferred (fixed defaults: `0644` / `0755` / `0777`) |
| File modes beyond fixed defaults | Still deferred |
| In-memory inode numbers beyond monotone counter | Still deferred (monotone counter is sufficient) |

---

## Node Types

v1 supports three node types:

| Type         | Description                                               |
|--------------|-----------------------------------------------------------|
| `VFS_DIR`    | Can contain files, directories, and symlinks              |
| `VFS_FILE`   | Has a mutable byte-string content payload                 |
| `VFS_SYMLINK`| Stores a target path string; resolved by the kernel, not the VFS |

The root node is always a `VFS_DIR`. It is created automatically at
initialization and cannot be deleted, renamed, or replaced.

---

## Path Model

- Paths are POSIX-style absolute paths starting with `/`.
- Path components are separated by `/`.
- Empty components (double slash) are rejected with `EINVAL`.
- Trailing slashes are rejected with `EINVAL`.
- `.` and `..` components are rejected with `EINVAL` — the VFS does not resolve
  them; callers must pass normalized paths.
- Component names longer than 255 bytes are rejected with `ENAMETOOLONG`.
- Node names are arbitrary non-empty byte strings that do not contain `/` or
  null bytes.
- The maximum path depth is not artificially limited.

**Note on symlink resolution:** The VFS path resolver (`resolve_path` in
`vfs.c`) does NOT follow symlinks. It is the kernel's job to resolve symlinks
before FUSE callbacks are invoked. A symlink node is treated as an opaque leaf
by the resolver.

---

## In-Memory Data Structures

### `vfs_node_t` — the inode

Every node (directory, file, or symlink) is a `vfs_node_t`:

| Field          | Type              | Used by             | Notes                                      |
|----------------|-------------------|---------------------|--------------------------------------------|
| `ino`          | `uint64_t`        | all                 | Monotone counter, assigned at creation     |
| `kind`         | `vfs_kind_t`      | all                 | `VFS_FILE`, `VFS_DIR`, or `VFS_SYMLINK`    |
| `content`      | `uint8_t *`       | `VFS_FILE`          | Heap-allocated; NULL if file is empty      |
| `content_len`  | `size_t`          | `VFS_FILE`          | Number of valid bytes in `content`         |
| `children`     | `vfs_dirent_t *`  | `VFS_DIR`           | Singly-linked list of `(name, node)` pairs |
| `link_target`  | `char *`          | `VFS_SYMLINK`       | Heap-allocated, NUL-terminated target path |
| `parent`       | `vfs_node_t *`    | all                 | NULL only for the root node                |
| `mtime`        | `struct timespec` | all                 | Last content modification time             |
| `atime`        | `struct timespec` | all                 | Last access time (set at creation; not auto-updated on read — noatime behaviour) |

### `vfs_dirent_t` — directory entry

Each directory child is a `vfs_dirent_t` in a singly-linked list:

| Field  | Type             | Notes                              |
|--------|------------------|------------------------------------|
| `name` | `char *`         | Heap-allocated, NUL-terminated     |
| `node` | `vfs_node_t *`   | Pointer to the child node          |
| `next` | `vfs_dirent_t *` | Next sibling in insertion order    |

### `vfs_stat_t` — stat result

Returned by `vfs_getattr` and used by the FUSE layer to fill `struct stat`:

| Field   | Type              | Notes                                              |
|---------|-------------------|----------------------------------------------------|
| `ino`   | `uint64_t`        |                                                    |
| `kind`  | `vfs_kind_t`      |                                                    |
| `size`  | `size_t`          | `content_len` for files; `strlen(link_target)` for symlinks; 0 for dirs |
| `mtime` | `struct timespec` |                                                    |
| `atime` | `struct timespec` |                                                    |

### `vfs_t` — the filesystem container

| Field       | Type           | Notes                                                         |
|-------------|----------------|---------------------------------------------------------------|
| `root`      | `vfs_node_t *` | Always a `VFS_DIR`; never NULL after `vfs_create()`           |
| `snapshot`  | `vfs_node_t *` | Deep-copy root saved by `vfs_save_snapshot`; NULL if no snapshot |
| `next_ino`  | `uint64_t`     | Counter for the next inode number                             |

---

## Public API

### Lifecycle

```c
vfs_t *vfs_create(void);
void   vfs_destroy(vfs_t *vfs);
```

### Read-only operations

```c
int vfs_getattr(vfs_t *vfs, const char *path, vfs_stat_t *out);
int vfs_readdir(vfs_t *vfs, const char *path, vfs_readdir_cb_t cb, void *ctx);
int vfs_read(vfs_t *vfs, const char *path, size_t offset, size_t size,
             uint8_t *buf, size_t *out_len);
int vfs_readlink(vfs_t *vfs, const char *path, char *buf, size_t bufsz);
```

`vfs_readlink` returns the number of bytes written (not NUL-terminated, per
POSIX `readlink` semantics). Returns `-EINVAL` if the path is not a symlink.

### Control-path mutating operations

These are called by the fuzzer control plane and by the FUSE write callbacks.

```c
int vfs_create_file(vfs_t *vfs, const char *path,
                    const uint8_t *content, size_t content_len);
int vfs_update_file(vfs_t *vfs, const char *path,
                    const uint8_t *content, size_t content_len);
int vfs_delete_file(vfs_t *vfs, const char *path);
int vfs_mkdir(vfs_t *vfs, const char *path);
int vfs_rmdir(vfs_t *vfs, const char *path);
int vfs_rename(vfs_t *vfs, const char *oldpath, const char *newpath);
int vfs_symlink(vfs_t *vfs, const char *path, const char *target);
int vfs_set_times(vfs_t *vfs, const char *path,
                  const struct timespec *mtime, const struct timespec *atime);
```

`vfs_update_file` automatically updates `mtime` on the node.

`vfs_set_times` accepts NULL for either pointer to leave that timestamp
unchanged. Used by `fvfs_utimens` and by the fuzzer to control metadata
mutations.

`vfs_rename` supports:
- Same-inode no-op (src == dst → returns 0)
- Atomic overwrite of an existing regular file at the destination
- Replacement of an existing empty directory at the destination
- Moving a directory with its entire subtree intact
- Cycle detection: rejects moving a directory into its own subtree (`-EINVAL`)

`vfs_delete_file` also works on symlinks (symlinks are deleted via `unlink`,
not `rmdir`).

### Snapshot and reset

```c
int vfs_save_snapshot(vfs_t *vfs);
int vfs_reset_to_snapshot(vfs_t *vfs);
```

`vfs_save_snapshot` deep-copies the entire current tree (including timestamps
and symlink targets) into `vfs->snapshot`. Overwrites any prior snapshot.

`vfs_reset_to_snapshot` deep-copies the snapshot back into `vfs->root`.
The snapshot is preserved so reset can be called repeatedly. Returns `-EINVAL`
if no snapshot exists.

**Known scalability note:** deep-copy is O(total tree size). For large rootfs
layouts this will become a bottleneck. A journal/diff approach (O(delta size))
is planned for Week 8 before real-world rootfs integration.

---

## Error Model

| Code           | Meaning                                                        |
|----------------|----------------------------------------------------------------|
| `ENOENT`       | Path or name does not exist                                    |
| `EEXIST`       | Path already exists                                            |
| `ENOTDIR`      | Expected a directory, found a file or symlink                  |
| `EISDIR`       | Expected a file, found a directory                             |
| `ENOTEMPTY`    | Directory is not empty                                         |
| `EINVAL`       | Invalid argument (empty name, `.`/`..` component, trailing slash, rename into own subtree, readlink on non-symlink) |
| `ENAMETOOLONG` | Path component exceeds 255 bytes                               |
| `ENOMEM`       | Allocation failure                                             |

All public functions return 0 on success or a negative errno value on failure.
The FUSE layer passes these values directly to the kernel.

---

## FUSE Layer (`fuse_vfs/fuse_vfs.c`)

The FUSE frontend holds a single global `vfs_t *g_vfs` and implements the
following callbacks:

| FUSE callback | VFS call(s)                        | Notes |
|---------------|------------------------------------|-------|
| `getattr`     | `vfs_getattr`                      | Converts `vfs_stat_t` → `struct stat`; sets `S_IFLNK` for symlinks |
| `readdir`     | `vfs_readdir`                      | Bridges `vfs_readdir_cb_t` → FUSE filler via `readdir_ctx_t` |
| `open`        | `vfs_getattr`                      | Rejects directories; allows any flags on files |
| `read`        | `vfs_read`                         |       |
| `readlink`    | `vfs_readlink`                     |       |
| `create`      | `vfs_create_file`                  | Called by kernel for `O_CREAT` on a new path |
| `write`       | `vfs_read` + `vfs_update_file`     | Read-modify-write; handles partial writes and appends; gaps zero-filled |
| `truncate`    | `vfs_read` + `vfs_update_file`     | Shrink or extend; zeros on extension |
| `mkdir`       | `vfs_mkdir`                        |       |
| `unlink`      | `vfs_delete_file`                  | Works for both files and symlinks |
| `rmdir`       | `vfs_rmdir`                        |       |
| `rename`      | `vfs_rename`                       | `flags` (RENAME_NOREPLACE etc.) ignored |
| `symlink`     | `vfs_symlink`                      | FUSE arg order is `(target, linkpath)`; swapped before calling VFS |
| `utimens`     | `vfs_set_times`                    | Handles `UTIME_NOW` and `UTIME_OMIT` per POSIX |

The FUSE process is single-threaded (no `-o clone_fd`). No locking is required
in the VFS core.

---

## Test Coverage

**`vfs/vfs_test.c`** — 439 checks across 16 test suites:

| Suite | What it covers |
|-------|---------------|
| `path_parsing` | Root, missing slash, `.`/`..`, double slash, trailing slash, `ENAMETOOLONG`, ENOENT, NULL |
| `create_file` | Success, content, nested, EEXIST, ENOENT parent, trailing slash, `.`/`..` as name |
| `mkdir` | Success, EEXIST, ENOENT parent, `.`/`..` as name, trailing slash |
| `readdir` | `.` and `..` entries, all children listed, ENOENT, ENOTDIR |
| `read` | Full read, partial, offset, offset past end, EISDIR, ENOENT, empty file |
| `update_file` | New content, empty content, ENOENT, EISDIR |
| `delete_file` | Success, ENOENT, EISDIR |
| `rmdir` | Success, ENOENT, ENOTDIR, ENOTEMPTY, root rejected |
| `nested` | Three-level tree, reads and deletes at depth |
| `mutation_sequence` | Interleaved create/update/delete/mkdir/rmdir sequences |
| `snapshot_reset` | Save, mutate, reset, verify; repeated resets |
| `snapshot_nested` | Multi-level tree preserved across reset |
| `invariants` | Root always exists, duplicate rejection, ENOTEMPTY enforcement |
| `random_sequence` | 200-step randomized create/update/delete/mkdir/rmdir + snapshot/reset |
| `rename` | Same-dir, cross-dir, overwrite, directory move with children, same-path no-op, ENOENT/ENOTEMPTY/EISDIR/ENOTDIR/EINVAL, cycle detection |
| `symlink` | Create/readlink roundtrip, getattr kind+size, error cases, delete, snapshot/restore, bufsz truncation |

**`fuse_vfs/test_mount.sh`** — 40 integration checks on a live FUSE mount:
root listing, nested dirs, file content and sizes, repeated opens, negative
cases, write (overwrite, create, append), mkdir/rmdir, unlink, touch (utimens).

---

## Invariants

1. The root directory always exists and is always a `VFS_DIR`.
2. Every non-root node has exactly one parent directory that contains it.
3. No directory contains two children with the same name.
4. A path component that resolves to a file or symlink cannot be used as a
   directory prefix.
5. After `vfs_reset_to_snapshot()`, the observable tree structure, file
   contents, symlink targets, and timestamps match those at the time
   `vfs_save_snapshot()` was called. Inode numbers need not be stable.

---

## Assumptions

- Single-threaded for v1. No locking in the VFS core.
- File content is stored as a raw byte array. No encoding or lazy loading.
- The baseline snapshot is a full in-memory deep copy. Acceptable for
  kilobyte-to-low-megabyte filesystems; a journal approach is planned for
  Week 8 when scaling to real rootfs sizes.
- Inode numbers need not be stable across snapshot/restore. The FUSE layer
  does not use lookup caching that depends on stable inodes.
- Symlink resolution is the kernel's responsibility. The VFS resolver treats
  symlink nodes as opaque leaves and never follows them.

---

## What Comes Next

- **Week 4**: Define the `fs_delta_t` mutation model and build the control
  plane interface so the fuzzer can push delta operations to the live VFS.
- **Week 5**: Implement LibAFL mutator stages and `vfs_diff_snapshot` to close
  the feedback loop (capture target-side writes as new seeds).
- **Week 8**: Replace deep-copy snapshot restore with a journal-based approach
  before importing a real container rootfs.
- **Still deferred**: hard links, xattrs, uid/gid, `chmod`, `release` callback.
