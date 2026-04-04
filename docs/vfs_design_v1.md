# VFS v1 Design Note

## Purpose

This document defines the scope, data model, and operation set for the first
version of the in-memory virtual filesystem (VFS). It is written before
implementation begins to prevent design drift and to make the Week 2 scope
concrete. Nothing outside this document should be implemented in the VFS core
until after the first end-to-end milestone is stable.

## Goals

- Build a filesystem state model that lives entirely in memory.
- Keep the VFS core free of any FUSE or kernel dependency so it can be unit
  tested independently.
- Cover exactly the operations needed to support the fuzzing loop: load a
  baseline state, apply mutations between iterations, reset to baseline between
  iterations, and serve reads to a mounted target program.
- Keep the implementation small enough to be finished and fully tested in one
  week.

## Non-goals for v1

The following are explicitly deferred. They must not be added during Week 2
unless the Week 2 scope is already complete with time to spare, and only after
noting the addition here.

- Symlinks
- Hard links
- Extended attributes (xattrs)
- Ownership, uid/gid, precise permission checking
- Timestamps (mtime, atime, ctime) with real values — stat entries will carry
  placeholder timestamps
- File modes beyond a fixed default (regular: 0644, directory: 0755)
- Arbitrary writes from the target program — the target is read-only; mutations
  come through the control path only
- In-memory inode numbers beyond a simple monotone counter

## Node Types

v1 supports exactly two node types:

| Type      | Description                               |
|-----------|-------------------------------------------|
| Directory | Can contain files and other directories   |
| Regular file | Has a byte-string content payload      |

The root node is always a directory. It is created automatically at
initialization and cannot be deleted or replaced.

## Path Model

- Paths are POSIX-style absolute paths starting with `/`.
- Path components are separated by `/`.
- Empty components (double slash) are rejected.
- `.` and `..` components are rejected — the VFS does not resolve them; the
  caller must normalize paths before calling.
- The maximum path depth is not artificially limited in v1, but the test suite
  will cover at least three levels of nesting.
- Node names are arbitrary non-empty byte strings that do not contain `/` or
  null bytes.

## In-Memory Data Structures

This section describes the logical structure. Implementation may choose any
concrete representation (hash map, tree, flat map keyed by absolute path) as
long as the observable behavior matches the spec below.

### Inode

Each node (directory or file) is an inode with:

| Field    | Type               | Notes                                         |
|----------|--------------------|-----------------------------------------------|
| `ino`    | `uint64`           | monotone counter, assigned at creation        |
| `kind`   | `NodeKind`         | `Directory` or `RegularFile`                  |
| `content`| `Vec<u8>` or `[]`  | meaningful only for `RegularFile`; empty for dirs |

### Directory

A directory inode additionally maintains an ordered map from name to child inode
reference. The map must not contain `.` or `..`.

### Filesystem

The filesystem is a container that holds:

- a reference to the root inode
- a snapshot slot (described below)

## Supported v1 Operations

### Read-only operations (called by FUSE layer or tests)

**`lookup(parent_path, name) -> inode | ENOENT`**
Returns the inode at `parent_path/name`. Returns `ENOENT` if the parent does
not exist or does not contain `name`.

**`getattr(path) -> stat | ENOENT`**
Returns a stat-like structure for the node at `path`. For directories:
`st_mode = S_IFDIR | 0755`, `st_nlink = 2`, `st_size = 0`. For regular files:
`st_mode = S_IFREG | 0644`, `st_nlink = 1`, `st_size = len(content)`.
Timestamps are set to a fixed epoch value (0).

**`readdir(path) -> [(name, inode)] | ENOENT | ENOTDIR`**
Returns the list of `(name, inode)` pairs in the directory at `path`. Always
includes `.` (the directory itself) and `..` (the parent, or itself for root).
Returns `ENOTDIR` if `path` exists but is a regular file.

**`read(path, offset, size) -> bytes | ENOENT | EISDIR`**
Returns up to `size` bytes from the file at `path` starting at `offset`. If
`offset >= len(content)`, returns an empty slice (not an error). Returns
`EISDIR` if `path` is a directory.

### Control-path mutating operations (not exposed to the target program)

These operations change the VFS state and are only reachable through the
mutation/control path, not through the FUSE read interface.

**`create_file(path, content) -> () | EEXIST | ENOENT | ENOTDIR`**
Creates a regular file at `path` with the given content. The parent directory
must already exist. Returns `EEXIST` if `path` already exists. Returns `ENOENT`
if the parent directory does not exist. Returns `ENOTDIR` if a component in the
path is a file, not a directory.

**`update_file(path, content) -> () | ENOENT | EISDIR`**
Replaces the content of the existing file at `path`. Returns `ENOENT` if the
path does not exist. Returns `EISDIR` if the path is a directory.

**`delete_file(path) -> () | ENOENT | EISDIR`**
Deletes the regular file at `path`. Returns `ENOENT` if it does not exist.
Returns `EISDIR` if the path is a directory (use `rmdir` for directories).

**`mkdir(path) -> () | EEXIST | ENOENT | ENOTDIR`**
Creates a directory at `path`. The parent must exist. Returns `EEXIST` if
`path` already exists (as either a file or directory).

**`rmdir(path) -> () | ENOENT | ENOTDIR | ENOTEMPTY`**
Deletes the empty directory at `path`. Returns `ENOTEMPTY` if the directory has
any children. The root directory cannot be deleted.

### Snapshot and reset operations

**`save_snapshot() -> ()`**
Saves a deep copy of the current filesystem state as the baseline snapshot.
This overwrites any previously saved snapshot.

**`reset_to_snapshot() -> () | (no snapshot)`**
Replaces the current filesystem state with the saved snapshot. After this call,
reads and mutations see the snapshotted state as if no mutations had occurred
since the snapshot was taken. If no snapshot has been saved, returns an error
(or panics — the caller must not call this without a prior `save_snapshot`).

## Invariants

The following invariants must hold at all times. The implementation should
enforce them by returning errors, not by silently breaking them.

1. The root directory always exists and is always a directory.
2. Every non-root node has exactly one parent directory that contains it.
3. No directory contains two children with the same name.
4. A path component that resolves to a regular file cannot be used as a
   directory prefix (no file can have children).
5. After `reset_to_snapshot()`, the state is identical to the state at the time
   `save_snapshot()` was called. This is a deep equality: same structure, same
   file contents, same inode assignments are NOT required (ino values may
   differ), but the observable tree structure and file content must match.

## Error Model

The VFS returns typed errors (or errno-compatible integer codes when needed by
FUSE). The canonical error codes for v1 are:

| Code       | Meaning                                               |
|------------|-------------------------------------------------------|
| `ENOENT`   | Path or name does not exist                           |
| `EEXIST`   | Path already exists                                   |
| `ENOTDIR`  | Expected a directory, found a file                    |
| `EISDIR`   | Expected a file, found a directory                    |
| `ENOTEMPTY`| Directory is not empty                               |
| `EINVAL`   | Invalid argument (e.g. empty name, path contains `..`) |

## What the FUSE Layer Will Do (preview)

The FUSE layer (implemented in Week 3) will hold a reference to the VFS and
implement the following FUSE callbacks by calling into the VFS:

| FUSE callback | VFS operation            |
|---------------|--------------------------|
| `getattr`     | `getattr`                |
| `readdir`     | `readdir`                |
| `open`        | `lookup` + type check    |
| `read`        | `read`                   |

The FUSE layer will not implement `write`, `unlink`, `mkdir`, `rmdir`, or
`rename` — those operations come through the control path, not through FUSE.
This keeps the mounted view strictly read-only from the target program's
perspective.

## What the Control Path Will Do (preview)

The control path (Week 5) will accept mutation batches from the fuzzer and
apply them to the VFS using `create_file`, `update_file`, `delete_file`,
`mkdir`, and `rmdir`. It will also call `reset_to_snapshot` between fuzzing
iterations.

## Implementation Language

The VFS core will be implemented in C (matching the existing counter_fs.c
baseline). A Makefile or build script will compile it and its tests. No
external runtime dependencies beyond the C standard library are required for
the core. FUSE is only linked in the `fuse_vfs` binary, not in the VFS core
library or its test suite.

## Assumptions Recorded

- The VFS is single-threaded for v1. No locking is required in the core. The
  FUSE layer serializes all callbacks through the single-threaded FUSE dispatch
  loop (no `-o clone_fd` or multithreading enabled).
- File content is stored as a raw byte array. No encoding, compression, or lazy
  loading is needed.
- The baseline snapshot is a full in-memory deep copy. For the file sizes
  expected in fuzzing (kilobytes to low megabytes), this is acceptable.
- inode numbers need not be stable across snapshot/restore cycles. The FUSE
  layer will not use `lookup` caching that depends on stable inodes.
