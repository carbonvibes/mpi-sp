# Week 4: Complete Technical Reference

---

## System Architecture

```
Fuzzer (LibAFL)
    │
    │  serialized fs_delta_t  ← byte buffer; LibAFL mutates this directly
    │
    ▼
control_plane                 ← in-process function call, no IPC
    ├── delta_deserialize()   ← parse + validate byte buffer → fs_delta_t
    ├── cp_apply_delta()      ← apply ops with correctness fixups
    │       ├── cp_ensure_parents()     ← create missing intermediate dirs
    │       └── depth-first RMDIR sort ← children before parents
    ├── cp_vfs_checksum()     ← FNV-1a hash of entire VFS tree
    ├── cp_dump_vfs()         ← print tree for inspection
    └── dry-run mode          ← apply → print → restore snapshot
    │
    ▼
vfs_t                         ← in-memory filesystem
    ├── root       vfs_node_t*
    ├── snapshot   vfs_node_t*  ← deep copy for reset
    └── next_ino   uint64_t
    │
    ▼
FUSE mount → target program (container runtime, OCI tools, etc.)
```

The fuzzer and VFS are in the same process. Applying a delta is a direct C function call — zero IPC overhead on the hot path.

---

## Data Structures

### VFS Node (`vfs_node_t`)

Every node in the in-memory filesystem — file, directory, or symlink — is one of these:

```c
struct vfs_node {
    uint64_t        ino;          /* inode number, monotonically increasing */
    vfs_kind_t      kind;         /* VFS_FILE | VFS_DIR | VFS_SYMLINK */
    uint8_t        *content;      /* VFS_FILE only: heap-allocated bytes */
    size_t          content_len;  /* VFS_FILE only: byte count */
    vfs_dirent_t   *children;     /* VFS_DIR only: linked list of (name, node) */
    char           *link_target;  /* VFS_SYMLINK only: heap-allocated string */
    vfs_node_t     *parent;       /* NULL for root */
    struct timespec mtime;
    struct timespec atime;
};
```

Directory entries form a singly-linked list:

```c
struct vfs_dirent {
    char         *name;    /* heap-allocated entry name */
    vfs_node_t   *node;    /* the node this entry points to */
    vfs_dirent_t *next;
};
```

The top-level filesystem object:

```c
typedef struct {
    vfs_node_t *root;       /* the "/" directory node */
    vfs_node_t *snapshot;   /* NULL if no snapshot saved; deep copy otherwise */
    uint64_t    next_ino;
} vfs_t;
```

---

### Single Operation (`fs_op_t`)

One unit of mutation intent:

```c
typedef struct {
    fs_op_kind_t     kind;         /* which of the 7 ops (enum value 1–7) */
    char            *path;         /* absolute path, heap-allocated, NUL-terminated */
    uint8_t         *content;      /* CREATE_FILE, UPDATE_FILE: heap-allocated bytes */
                                   /* NULL for all other kinds */
    size_t           content_len;  /* CREATE_FILE / UPDATE_FILE: byte count of content */
                                   /* TRUNCATE: new file size (no content bytes written) */
                                   /* all others: 0 */
    struct timespec  mtime;        /* SET_TIMES: desired mtime; zero for others */
    struct timespec  atime;        /* SET_TIMES: desired atime; zero for others */
} fs_op_t;
```

The `content_len` field is dual-purpose by design: for CREATE/UPDATE it is the number of bytes in the content buffer; for TRUNCATE it is the target file size with no bytes in the buffer. This keeps the struct small — no extra `new_size` field needed.

The 7 op kinds:

```c
typedef enum {
    FS_OP_CREATE_FILE = 1,
    FS_OP_UPDATE_FILE = 2,
    FS_OP_DELETE_FILE = 3,
    FS_OP_MKDIR       = 4,
    FS_OP_RMDIR       = 5,
    FS_OP_SET_TIMES   = 6,
    FS_OP_TRUNCATE    = 7,
} fs_op_kind_t;
```

---

### Delta (`fs_delta_t`)

The testcase. One delta = one fuzzing input = one ordered list of ops:

```c
typedef struct {
    fs_op_t *ops;    /* heap-allocated array, doubles in capacity on growth */
    size_t   n_ops;  /* number of valid entries */
    size_t   cap;    /* allocated capacity */
} fs_delta_t;
```

A delta is built with convenience constructors:

```c
delta_add_create_file(d, "/etc/shadow", content, len);
delta_add_update_file(d, "/etc/passwd", content, len);
delta_add_delete_file(d, "/tmp/lockfile");
delta_add_mkdir(d, "/var/run/app");
delta_add_rmdir(d, "/tmp/old");
delta_add_set_times(d, "/var/log/app.log", &mtime, &atime);
delta_add_truncate(d, "/var/log/app.log", 0);  /* truncate to empty */
```

Each constructor deep-copies the path and content so the caller can free its own buffers immediately.

---

### Apply Result (`cp_result_t`)

`cp_apply_delta` returns one of these:

```c
typedef struct {
    int             total_ops;   /* == d->n_ops */
    int             succeeded;
    int             failed;
    cp_op_result_t *results;     /* array[total_ops]; one entry per op */
} cp_result_t;

typedef struct {
    int         op_index;   /* index into the delta's ops array */
    int         error;      /* 0 = success, negative errno = failure */
    const char *message;    /* "ok" | "ensure_parents failed" | "vfs error" */
} cp_op_result_t;
```

---

## Wire Format (Binary Serialization)

The delta serializes to a flat byte buffer. This is what LibAFL stores in the corpus and mutates.

### Layout

```
offset 0:
  [magic    u32 BE]   0x46534400  ("FSD\0") — reject garbage immediately

offset 4:
  [n_ops    u32 BE]   number of operations; 0 is invalid; max 65535

offset 8, per op (repeated n_ops times):
  [kind     u8    ]   1–7; 0 is reserved and will be rejected
  [path_len u16 BE]   byte count of path; 0 is invalid; path must start with '/'
  [path     bytes ]   path_len bytes; NOT NUL-terminated in wire
  [size     u32 BE]   semantic size:
                        CREATE_FILE / UPDATE_FILE → content byte count
                        TRUNCATE                  → new file size
                        all others                → 0
  [data_len u32 BE]   actual bytes that follow:
                        CREATE_FILE / UPDATE_FILE → == size
                        all others                → 0
  [data     bytes ]   data_len bytes of file content
  [mtime_sec  s64 BE] SET_TIMES: desired mtime seconds; 0 for others
  [mtime_nsec s64 BE]
  [atime_sec  s64 BE]
  [atime_nsec s64 BE]
```

**Fixed overhead per op:** `1 + 2 + 4 + 4 + 8 + 8 + 8 + 8 = 43 bytes`, plus path and data.

**All fields always present.** Unused fields are zeroed. This means AFL can flip timestamp bytes or size bytes on any op without breaking the parse — only magic, n_ops, kind, path_len, path, data_len, and data bytes are structurally required.

### TRUNCATE: no data bytes written

A naive TRUNCATE-to-N implementation would write N zero bytes into the wire buffer. Instead:

```
size     = N    ← the new file size (semantic)
data_len = 0    ← zero bytes follow in the buffer
```

The deserializer sees `data_len=0`, reads no bytes, and sets `content_len = size_field = N`. A 1GB truncate op is 43 + path_len bytes in the corpus. No bloat.

### Serialization code (actual implementation)

```c
uint8_t *delta_serialize(const fs_delta_t *d, size_t *out_len)
{
    /* Calculate total buffer size. */
    size_t total = 8;  /* magic(4) + n_ops(4) */
    for (size_t i = 0; i < d->n_ops; i++) {
        const fs_op_t *op = &d->ops[i];
        size_t path_len = op->path ? strlen(op->path) : 0;
        size_t data_len = (op->kind == FS_OP_CREATE_FILE ||
                           op->kind == FS_OP_UPDATE_FILE)
                          ? op->content_len : 0;
        total += DELTA_OP_FIXED + path_len + data_len;
    }

    uint8_t *buf = calloc(1, total);  /* calloc zeros unused fields */

    w32be(buf + 0, DELTA_MAGIC);
    w32be(buf + 4, (uint32_t)d->n_ops);

    size_t pos = 8;
    for (size_t i = 0; i < d->n_ops; i++) {
        const fs_op_t *op = &d->ops[i];
        size_t path_len = strlen(op->path);
        size_t data_len = (op->kind == FS_OP_CREATE_FILE ||
                           op->kind == FS_OP_UPDATE_FILE)
                          ? op->content_len : 0;

        buf[pos++] = (uint8_t)op->kind;
        w16be(buf + pos, (uint16_t)path_len); pos += 2;
        memcpy(buf + pos, op->path, path_len); pos += path_len;
        w32be(buf + pos, (uint32_t)op->content_len); pos += 4;  /* size field */
        w32be(buf + pos, (uint32_t)data_len);         pos += 4;
        if (data_len) { memcpy(buf + pos, op->content, data_len); }
        pos += data_len;
        w64be(buf + pos, op->mtime.tv_sec);  pos += 8;
        w64be(buf + pos, op->mtime.tv_nsec); pos += 8;
        w64be(buf + pos, op->atime.tv_sec);  pos += 8;
        w64be(buf + pos, op->atime.tv_nsec); pos += 8;
    }
    *out_len = total;
    return buf;
}
```

All integers written big-endian with explicit shift/mask helpers (`w32be`, `w16be`, `w64be`) — no `memcpy` of multi-byte integers, no undefined behavior from alignment or endian assumptions.

### Deserialization: the NEED macro

The deserializer is bounds-checked at every read with a single macro:

```c
#define NEED(n) \
    do { \
        if ((size_t)(n) > len - pos) { err = -EINVAL; goto fail_op; } \
    } while (0)
```

Validation rules enforced on each op:
- `kind` must be 1–7; 0 is rejected
- `path_len` must be > 0
- `path[0]` must be `'/'`
- `data_len` bytes must actually exist in the buffer
- For TRUNCATE: `content_len = size_field` (no data bytes); for CREATE/UPDATE: `content_len = data_len`

On any parse error: free all allocations, return NULL, set `*err_out = -EINVAL`.

### Rejection Rate

We ran 10,000 single-byte random overwrites on a valid serialized delta and measured what fraction `delta_deserialize` rejected:

- **Rejection rate: 16.7%**
- **Acceptance rate: 83.3%**

A rejection rate below 70% means AFL byte mutations are productive — most flips produce a parseable testcase. The byte-buffer format is confirmed viable for LibAFL corpus mutation.

---

## Control Plane: Applying Deltas Correctly

### Problem

A fuzzer-generated delta may be internally inconsistent in ways that are not parse errors:

- `CREATE_FILE /a/b/c.txt` before `/a/b/` exists
- `RMDIR /a` before `RMDIR /a/b/c` (parent removed before child — fails)

A naive apply-in-order would fail these ops. The control plane fixes both automatically.

### `cp_ensure_parents` — Intermediate Directory Creation

```c
int cp_ensure_parents(vfs_t *vfs, const char *path)
{
    char *tmp = strdup(path);
    for (size_t i = 1; tmp[i] != '\0'; i++) {
        if (tmp[i] == '/') {
            tmp[i] = '\0';              /* temporarily NUL-terminate at separator */
            int r = vfs_mkdir(vfs, tmp);
            tmp[i] = '/';              /* restore */
            if (r != 0 && r != -EEXIST) { free(tmp); return r; }
        }
    }
    free(tmp); return 0;
}
```

For `path = "/a/b/c.txt"`:
- `i=2`: NUL → `vfs_mkdir("/a")` → restore
- `i=4`: NUL → `vfs_mkdir("/a/b")` → restore
- `i=6`: NUL (`\0`) → loop ends; the file itself is not created

EEXIST is silenced — if a prior op already created the directory, that is fine. Any other error is returned immediately.

Called before every `CREATE_FILE` and `MKDIR` op in Phase 1.

### Two-Phase Apply Algorithm

```
Phase 1 — non-RMDIR ops in original delta order:
  for each op that is not RMDIR:
    if kind == CREATE_FILE or MKDIR:
      cp_ensure_parents(vfs, op->path)
    apply_single_op(vfs, op)
    if result == -EEXIST and kind == MKDIR:
      treat as success  ← ensure_parents may have created it already

Phase 2 — RMDIR ops, deepest first:
  collect all RMDIR ops into array of {index, path_depth}
  sort descending by path_depth (qsort with cmp_rmdir_desc)
  apply in sorted order
```

Path depth is the number of `'/'` characters in the path:
- `/a` → depth 1
- `/a/b` → depth 2
- `/a/b/c` → depth 3

So a delta listing `RMDIR /a`, `RMDIR /a/b`, `RMDIR /a/b/c` gets reordered to `RMDIR /a/b/c`, `RMDIR /a/b`, `RMDIR /a` — deepest first, all succeed.

### TRUNCATE: Read-Modify-Write

TRUNCATE is not a VFS primitive — the VFS has no truncate call. The control plane implements it as:

```c
case FS_OP_TRUNCATE: {
    vfs_stat_t vs;
    vfs_getattr(vfs, op->path, &vs);         /* get current size */
    size_t new_sz = op->content_len;          /* new size from delta */
    uint8_t *tmp = calloc(1, new_sz);         /* zero-filled new buffer */
    if (vs.size > 0) {
        size_t copy = vs.size < new_sz ? vs.size : new_sz;
        size_t got;
        vfs_read(vfs, op->path, 0, copy, tmp, &got);   /* copy existing bytes */
    }
    vfs_update_file(vfs, op->path, tmp, new_sz);        /* replace content */
    free(tmp);
}
```

Shrink: existing bytes beyond `new_sz` are dropped.
Extend: `calloc` zero-fills the extension.

### Dry-Run Mode

`cp_apply_delta(vfs, delta, 1)`:

1. Run Phase 1 + Phase 2 normally
2. Print the resulting VFS tree to stdout via `cp_dump_vfs`
3. Call `vfs_reset_to_snapshot` — requires a snapshot to have been saved first

Used to eyeball whether a delta produces a sensible filesystem before running a real target.

---

## VFS Snapshot and Reset

```c
int vfs_save_snapshot(vfs_t *vfs);
int vfs_reset_to_snapshot(vfs_t *vfs);
```

`vfs_save_snapshot`: deep-copies the entire node tree into `vfs->snapshot`. Any prior snapshot is freed. Cost: O(n) — proportional to the total size of all files.

`vfs_reset_to_snapshot`: deep-copies `vfs->snapshot` back into `vfs->root`. The snapshot is **not consumed** — reset can be called repeatedly from the same baseline. Returns `-EINVAL` if no snapshot was saved.

The VFS struct holds exactly **one snapshot slot**. Currently used for: baseline restore after each fuzzing iteration, and dry-run undo.

---

## Baseline Checksum (`cp_vfs_checksum`)

Every saved testcase must be reproducible. The VFS state the target sees depends on what baseline filesystem was loaded. We compute a 64-bit FNV-1a hash of the entire VFS tree and store it with the testcase corpus entry. Anyone with the same baseline image and the same delta can reproduce the exact crash.

**FNV-1a 64-bit:**
```c
uint64_t h = 0xcbf29ce484222325ULL;
for each byte b:
    h = (h ^ b) * 0x100000001b3ULL;
```

What gets hashed per node (in readdir/insertion order):
- Absolute path string
- mtime seconds + nanoseconds
- atime seconds + nanoseconds
- File content bytes (for files)
- Symlink target string (for symlinks)
- Recurses into directories

**Known limitation:** order is insertion order, not alphabetical. Two VFS instances with the same files created in different orders will produce different checksums. Alphabetical sort is deferred to a later week.

---

## Test Results

15 test suites, 224 checks, 0 failures, 0 ASAN errors.

| Suite | What it verifies |
|---|---|
| `delta_lifecycle` | Create delta, add all 7 op kinds, verify deep copy, free |
| `delta_serialize` | Serialize → deserialize roundtrip for all 7 kinds; verify every field |
| `delta_deser_errors` | Truncated buffer, wrong magic, zero n_ops, invalid kind, non-absolute path |
| `delta_checksum` | Same data → same hash; different data → different hash |
| `ensure_parents` | Basic, deep path, already-exists, root path, bad path |
| `apply_basic` | One op of each kind through `cp_apply_delta` |
| `apply_ensure_parents` | `CREATE_FILE /a/b/c.txt` before `MKDIR /a/b` — both succeed |
| `apply_rmdir_ordering` | RMDIR listed shallowest-first — reordered, all succeed |
| `apply_errors` | ENOENT, EISDIR, ENOTEMPTY returned correctly |
| `apply_set_times` | SET_TIMES reaches VFS; mtime/atime verified post-apply |
| `apply_truncate` | Shrink preserves bytes; extend zero-fills |
| `apply_dry_run` | `dry_run=1`: tree printed; file does NOT exist after apply |
| `apply_mutate_reset` | 10 iterations of apply + `vfs_reset_to_snapshot`; no stale state |
| `vfs_checksum` | Identical tree → same hash; mutation → different hash; timestamp change → different hash |
| `rejection_rate` | 10,000 random byte-flip mutations; **16.7% rejection rate** |

---

## What Comes Next: The Feedback Loop (Week 5)

### The Per-Iteration Flow

```
Startup:
  Load baseline filesystem into VFS
  vfs_save_snapshot()              ← deep copy, done once

Per iteration:
  [1]  pre_delta_gen = vfs->current_gen    ← copy one integer
  [2]  Apply fuzzer's delta
  [3]  pre_run_gen = vfs->current_gen      ← copy one integer
  [4]  Run target through FUSE mount
  [5]  Diff: find what target wrote (gen > pre_run_gen)
  [6]  If diff non-empty → promote as new corpus seed
  [7]  vfs_reset_to_snapshot()             ← restore baseline
  [8]  Repeat
```

Two "snapshots" are needed simultaneously:
- The **baseline** — a full deep copy saved at startup, restored at end of each iteration
- The **pre-run watermark** — a single integer saved just before the target runs, used to identify target writes

These are fundamentally different. The baseline requires a full tree copy. The pre-run watermark is free.

---

### Generation Counter

Add to `vfs_node_t`:
```c
uint64_t gen;   /* generation when this node was last mutated */
```

Add to `vfs_t`:
```c
uint64_t current_gen;
```

Every mutating VFS call increments `current_gen` and stamps the node:
```c
vfs->current_gen++;
node->gen = vfs->current_gen;
```

The pre-run watermark is then:
```c
uint64_t pre_run_gen = vfs->current_gen;   /* O(1) */
```

After the target runs, finding what it wrote:
```c
/* walk tree: collect nodes where node->gen > pre_run_gen */
```

### Concrete Example

Baseline loaded. `current_gen = 3`.

```
/bin/sh          gen=1
/etc/passwd      gen=2
/tmp/            gen=3
```

Fuzzer applies delta (creates `/tmp/input.txt`). `current_gen → 4`.

Save: `pre_run_gen = 4`.

Target runs, modifies `/etc/passwd` and creates `/tmp/output`. `current_gen → 5, 6`.

```
/bin/sh          gen=1
/etc/passwd      gen=5   ← target wrote this
/tmp/            gen=3
/tmp/input.txt   gen=4   ← fuzzer wrote this (before pre_run_gen)
/tmp/output      gen=6   ← target wrote this
```

Diff walk — `node->gen > pre_run_gen` (i.e. `> 4`):

```
/bin/sh        gen=1 → NO
/etc/passwd    gen=5 → YES  ← captured ✓
/tmp/          gen=3 → NO
/tmp/input.txt gen=4 → NO   ← correctly excluded (fuzzer, not target)
/tmp/output    gen=6 → YES  ← captured ✓
```

---

### Subtree Pruning

Add to directory nodes:
```c
uint64_t subtree_max_gen;  /* max gen of self and all descendants */
```

Updated on every mutation — propagate up the path to root. Then:

```c
if (dir->subtree_max_gen <= pre_run_gen) {
    /* nothing inside this directory changed — skip entire subtree */
}
```

For a rootfs with `/bin/` holding 3000 files none of which the target touched:
```
/bin/   subtree_max_gen=1  →  1 > 4?  NO  → skip all 3000 files in one check
/etc/   subtree_max_gen=5  →  5 > 4?  YES → descend
/tmp/   subtree_max_gen=6  →  6 > 4?  YES → descend
```

If the target touches 2 files out of 10,000, the diff walk visits ~20 nodes instead of 10,000.

Complexity: O(changed_files × tree_depth). For typical fuzzing workloads this is effectively constant.

---

### Snapshot Summary

| Mechanism | What it is | Complexity | When |
|---|---|---|---|
| `vfs->snapshot` (deep copy) | Full tree copy for baseline restore | O(n) | Once at startup |
| `pre_delta_gen` (uint64) | Watermark before fuzzer applies delta | O(1) | Each iteration |
| `pre_run_gen` (uint64) | Watermark before target runs | O(1) | Each iteration |
| Diff walk | Find target writes via gen counter + subtree pruning | O(changed × depth) | After each run |
| `vfs_reset_to_snapshot` | Restore baseline | O(n) | After each run |

The reset (`vfs_reset_to_snapshot`) is still a full deep copy and is the known bottleneck for large rootfs. Week 5 measures it. If reset cost exceeds 1ms on the demo tree, the journal/CoW optimisation is pulled forward from Week 8.
