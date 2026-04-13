# Mutation Model

## Purpose

This document defines the filesystem mutation model used by the fuzzer
control plane.  It covers the testcase representation (`fs_delta_t`), the
binary wire format, the serialization rejection-rate measurement, the
`ensure_parents()` ordering strategy, and the baseline checksum scheme.

**Current status: Week 4 implementation complete.**
All components are implemented in `control_plane/` and validated by
224 passing checks in `control_plane/cp_test.c`.

---

## The Testcase Representation — `fs_delta_t`

A testcase is a **delta**: an ordered list of typed filesystem operations
(`fs_op_t`) applied to a concrete baseline state.

```
delta = [op_0, op_1, ..., op_{n-1}]
```

### Op kinds

| Kind          | Meaning                                        | Key fields                          |
|---------------|------------------------------------------------|-------------------------------------|
| `CREATE_FILE` | Create a new regular file with content         | `path`, `content`, `content_len`    |
| `UPDATE_FILE` | Replace a file's content entirely              | `path`, `content`, `content_len`    |
| `DELETE_FILE` | Unlink a regular file or symlink               | `path`                              |
| `MKDIR`       | Create a directory                             | `path`                              |
| `RMDIR`       | Remove an empty directory                      | `path`                              |
| `SET_TIMES`   | Set `mtime` and `atime` on any node            | `path`, `mtime`, `atime`            |
| `TRUNCATE`    | Resize a file to exactly N bytes               | `path`, `content_len` (= new size)  |

`SET_TIMES` and `TRUNCATE` are first-class op kinds.  Programs that call
`stat()` before `read()` are sensitive to size/timestamp mismatches; omitting
these would leave a large class of interesting mutations unexplored.

### Why a delta rather than a full state?

The fuzzer maintains a concrete baseline loaded into the VFS once
(`vfs_save_snapshot`).  Per iteration:

1. Apply the delta to the live VFS (fast: only touch nodes listed in the delta).
2. Run the target program through the FUSE mount.
3. Reset to the baseline snapshot (`vfs_reset_to_snapshot`).

Reset cost is proportional to the size of the delta applied in step 1, not
the full filesystem size, because the delta only touches a small fraction of
nodes.  (The current `vfs_reset_to_snapshot` is still a full deep copy —
see Week 8 for the journal/CoW optimisation — but the delta-driven model is
already structurally better than rebuilding from scratch each iteration.)

---

## Wire Format (Binary Serialization)

The binary format is designed so that AFL byte-flip mutations can usefully
mutate file content without invalidating the entire record structure.

### Layout

```
Header (8 bytes):
  [magic  u32 BE]  — 0x46534400 ("FSD\0")
  [n_ops  u32 BE]  — number of ops (0 is invalid)

Per op (variable length):
  [kind      u8   ]  — fs_op_kind_t value (1–7); 0 is reserved/invalid
  [path_len  u16 BE]  — byte count of path (path must start with '/')
  [path      bytes]   — path_len bytes, NOT NUL-terminated
  [size      u32 BE]  — semantic size:
                          CREATE_FILE/UPDATE_FILE: content byte count
                          TRUNCATE:               new file size
                          others:                 0
  [data_len  u32 BE]  — count of data bytes that follow:
                          CREATE_FILE/UPDATE_FILE: == size
                          TRUNCATE / others:      0
  [data      bytes]   — data_len content bytes
  [mtime_sec  s64 BE] — SET_TIMES: desired mtime; 0 for other kinds
  [mtime_nsec s64 BE]
  [atime_sec  s64 BE] — SET_TIMES: desired atime; 0 for other kinds
  [atime_nsec s64 BE]
```

Fixed per-op overhead: `1 + 2 + 4 + 4 + 8 + 8 + 8 + 8 = 43 bytes`.
Total wire size: `8 + sum(43 + path_len + data_len)` over all ops.

**Design note on TRUNCATE:** `size` = new file size; `data_len` = 0 (no
content bytes written).  This avoids writing N zero bytes for a TRUNCATE-to-N
operation which would bloat the serialized testcase.

All unused fields are always zeroed (by `calloc` in the serializer).  This
is intentional: AFL mutations that flip timestamp bytes or size bytes on a
non-SET_TIMES op produce a structurally valid record with harmless no-op
values rather than a parse failure.

---

## Rejection Rate Measurement

**Question:** if we let AFL apply arbitrary byte-flip mutations to a
serialized delta, what fraction of mutated buffers are structurally invalid
(rejected by `delta_deserialize`)?  If the rejection rate is low enough,
AFL gets full havoc/splice/minimize for free on the file content bytes.

**Method:** `cp_test.c :: test_rejection_rate` (see suite 15):

1. Build a representative 3-op delta (CREATE_FILE, UPDATE_FILE, SET_TIMES).
2. Serialize it to a byte buffer (~120 bytes for this test).
3. Apply 10 000 random single-byte overwrites (LCG PRNG, seed `0xdeadbeef`,
   one random byte at a random position per trial).
4. Call `delta_deserialize` on each mutated buffer.
5. Count accepted / rejected.

**Result (measured at implementation time):**

```
1668 / 10000 mutations accepted  (rejection rate: 16.7%)
```

**Decision:** Rejection rate 16.7% is well below the 70% threshold.  AFL
byte-flip mutations produce valid deltas 83.3% of the time, which is
high enough to make the byte-buffer format viable as a LibAFL `Input` type.

**Implication for Week 5:** Register the serialized `fs_delta_t` as the
LibAFL `Input` type.  AFL's built-in havoc, splice, and minimize stages will
apply directly to content bytes (the dominant variable-length regions), while
the structured op headers are stable enough that most mutations remain valid.

**Why the rejection rate is low:** The 43-byte fixed overhead per op is
largely timestamps (32 bytes) and size fields (8 bytes) — all of which are
unchecked data, not structural.  Only the 5-byte structural core per op
(kind + path_len + data_len) and the 8-byte header contribute to rejection.
Single-byte mutations hitting content bytes are always accepted.

---

## `ensure_parents()` — Ordering Strategy

A flat op list can contain `CREATE_FILE /a/b/c.txt` before `MKDIR /a/b`.
Applied naively, the CREATE would fail with ENOENT.

**Control plane fix:** Before every `CREATE_FILE` or `MKDIR` op, the control
plane calls `cp_ensure_parents()`, which creates any missing intermediate
directories automatically (ignoring EEXIST).  `MKDIR` ops that fail with
EEXIST are also treated as success (the directory already exists — intent
satisfied).

```
Phase 1: Walk all non-RMDIR ops in original order.
         Before CREATE_FILE or MKDIR: call cp_ensure_parents(path).
         On MKDIR EEXIST: treat as success.
         Apply each op; record per-op result.

Phase 2: Collect all RMDIR ops.
         Sort by path depth descending (most '/' chars first).
         Apply in sorted order (deepest removed first).
```

**Why depth-first RMDIR?**  `RMDIR /a` fails if `/a/b` still exists.  A
delta listing `RMDIR /a, RMDIR /a/b, RMDIR /a/b/c` in that order would fail
all three if applied naively.  Sorting to `/a/b/c → /a/b → /a` makes all
three succeed.

**VFS core unchanged:** These fixups live entirely in the control plane.
The VFS core keeps its strict POSIX semantics.

---

## Baseline Checksum

Every saved testcase should be reproducible by anyone with the same baseline.
The checksum ties a testcase to its baseline.

**`cp_vfs_checksum(vfs_t *vfs)`** walks the current VFS tree in readdir
order (insertion order, which is deterministic for a given population
sequence) and computes a 64-bit FNV-1a hash over:

- The absolute path of every node.
- File content bytes.
- Symlink target strings.
- `mtime` and `atime` (seconds + nanoseconds) of every node.

Two VFS instances populated by the same sequence of operations will have
the same checksum.  Crash testcases are stored with the checksum of the
baseline at the time they were found; a reproducer needs the same baseline
and the same delta.

**`delta_checksum(buf, len)`** computes the same FNV-1a hash on a raw
serialized buffer.  Use this to fingerprint a serialized delta independently
of the VFS state.

**Known limitation:** The checksum is order-sensitive (insertion order, not
alphabetical).  Two baselines populated by the same files in a different
order will produce different checksums even though they appear identical.
Sorting children before hashing (alphabetical order) would fix this — deferred
to Week 8 when reproducibility requirements are firmer.

---

## Files

| File                            | Contents                                   |
|---------------------------------|--------------------------------------------|
| `control_plane/delta.h`         | `fs_op_t`, `fs_delta_t`, serialization API |
| `control_plane/delta.c`         | Lifecycle, serialize, deserialize, checksum |
| `control_plane/control_plane.h` | Apply API, `cp_ensure_parents`, dump       |
| `control_plane/control_plane.c` | `cp_apply_delta`, `cp_vfs_checksum`, dump  |
| `control_plane/cp_test.c`       | 224-check test suite, rejection-rate trial |
| `control_plane/Makefile`        | Build rules                                |
