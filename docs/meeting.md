# Seed Corpus and Baseline Reference

## At a Glance

| Item | Type | Purpose |
| --- | --- | --- |
| `baseline_file_paths` | `Vec<String>` | Existing baseline files used for file-only targeting |
| `baseline_dir_paths` | `Vec<String>` | Existing baseline directories used for directory-only targeting |
| `baseline_all_paths` | `Vec<String>` | Existing baseline files and directories used for general path targeting |
| `generate_seed_corpus(...)` | `Vec<FsDelta>` | Creates the 7 primary seed deltas |
| `initial_corpus_pool()` | `Vec<FsDelta>` | Adds 4 hard-coded donor deltas for early splice diversity |
| `live_corpus` | `Vec<FsDelta>` | Starts with 11 deltas, then grows with promoted novel deltas |

Key distinction:

```text
FsOp       = one filesystem operation
FsDelta    = array/list of FsOp values
Corpus     = array/list of FsDelta values
Path sets  = arrays of path strings, not corpus entries
```

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

```text
build live_corpus = generate_seed_corpus(baseline_file_paths) + initial_corpus_pool()
  -> pick a random seed from the LIVE corpus each iteration
  -> apply 1–3 random mutations from the applicable-mutator subset
     (skip-early filtering via can_apply precondition check)
  -> apply_delta(vfs, &delta)
  -> count per-op VFS successes/failures
  -> compare VFS checksum with the baseline
  -> if the post-apply checksum is novel, promote the delta into live_corpus
     (bounded at MAX_LIVE_CORPUS = 128 with random non-seed eviction)
  -> reset VFS back to the saved snapshot
```

## 1. Baseline VFS Setup

Baseline population:

```text
CreateFile("/input", "seed")
Mkdir("/etc")
CreateFile("/etc/config", "[settings]\nverbose=0\n")
```

Resulting baseline tree:

```text
/input          file
/etc            directory
/etc/config     file
```

After the baseline is populated, the VFS is snapshotted. Every fuzzing
iteration resets back to this snapshot.

## 2. Baseline Path Sets

After baseline setup, the harness enumerates three stable path sets.

| Path set | Source | Meaning | Typical values |
| --- | --- | --- | --- |
| `baseline_file_paths` | `enumerate_vfs_file_paths(vfs)` | Files only | `"/input"`, `"/etc/config"` |
| `baseline_dir_paths` | `enumerate_vfs_dir_paths(vfs)` | Directories only | `"/etc"` |
| `baseline_all_paths` | `enumerate_vfs_all_paths(vfs)` | Files and directories | `"/input"`, `"/etc"`, `"/etc/config"` |

These path sets are calculated from the clean baseline and reused by mutators
for type-correct targeting.

Examples:

```text
UpdateExistingFile -> baseline_file_paths
Rmdir              -> baseline_dir_paths
SetTimes           -> baseline_all_paths
```

## 3. Baseline Contents

`UpdateExistingFile` can perturb real baseline file content. In Phase A, the
baseline content map is:

| Path | Content |
| --- | --- |
| `"/input"` | `"seed"` |
| `"/etc/config"` | `"[settings]\nverbose=0\n"` |

This helps content mutations preserve useful structure while still changing
bytes.

## 4. Seven Seed Deltas

`generate_seed_corpus(baseline_files)` creates exactly 7 seed deltas.

It selects:

```text
primary   = baseline_files.first(), fallback "/input"
secondary = baseline_files.get(1), fallback "/etc/config"
```

The return type is:

```rust
Vec<FsDelta>
```

That means the result is an array of deltas, not one large delta.

### Seed Delta List

| # | Name | Delta ops |
| --- | --- | --- |
| 1 | Update primary | `[UpdateFile(primary, "seed")]` |
| 2 | Update secondary | `[UpdateFile(secondary, "[settings]\nverbose=1\n")]` |
| 3 | Truncate primary | `[Truncate(primary, 2)]` |
| 4 | Set times on primary | `[SetTimes(primary, 1700000000, 0, 1700000000, 0)]` |
| 5 | Multi-op update + metadata | `[UpdateFile(primary, "fuzzed content"), SetTimes(primary, 1000000000, 0, 1000000000, 0)]` |
| 6 | Create fresh file | `[CreateFile("/fuzz_input", "new")]` |
| 7 | Directory plus child file | `[Mkdir("/fuzz_dir"), CreateFile("/fuzz_dir/file", "hello")]` |

Important notes:

- Seed count is always 7.
- Each seed is one `FsDelta`.
- Seed 1 deliberately uses `UpdateFile`, not `CreateFile`, to avoid `EEXIST`
  on the existing `"/input"` baseline file.

## 5. Initial Corpus Donor Deltas

`initial_corpus_pool()` returns 4 hard-coded `FsDelta` values. These are added
to the live corpus at startup and give `SpliceDelta` useful donor material
before the corpus has evolved.

### Donor Delta List

| # | Name | Delta ops |
| --- | --- | --- |
| 1 | Config update donor | `[UpdateFile("/etc/config", "[settings]\nverbose=1\n")]` |
| 2 | Data subtree creation donor | `[Mkdir("/data"), CreateFile("/data/a.bin", [0xde, 0xad, 0xbe, 0xef]), CreateFile("/data/b.txt", "hello\n")]` |
| 3 | Long input overwrite donor | `[UpdateFile("/input", "AAAAAAAAAAAAAAAA")]` |
| 4 | Metadata plus truncate donor | `[SetTimes("/input", 1700000000, 0, 1700000000, 0), Truncate("/input", 2)]` |

At startup:

```text
live_corpus =
  7 seed deltas
  + 4 donor deltas
  = 11 FsDelta entries
```

After fuzzing begins, novel-checksum deltas can also be promoted into
`live_corpus`.

## 6. Path Vocabulary

`PATH_COMPONENTS` is used by random path generation and component-level path
mutation.

```rust
/// A small vocabulary of valid path components.
static PATH_COMPONENTS: &[&str] = &[
    "a", "b", "c", "d",
    "etc", "tmp", "var", "lib", "usr",
    "input", "output", "config", "data", "test", "run",
];
```

## 7. Random Path Construction

`random_path(rand)` creates an absolute path from the path vocabulary.

Algorithm:

```text
1. Choose a depth in [1, 3]
2. Pick one token from PATH_COMPONENTS for each component
3. Join components with a leading slash
```

Examples:

```text
/tmp
/etc/config
/a/run/data
```

Important behavior:

- Random path generation does not check whether the path exists in the
  baseline.
- Some mutators use baseline-biased or guidance-biased selection helpers when
  configured.

## 8. Content Dictionary

`CONTENT_DICTIONARY` is used by `ReplaceFileContent` and by some perturbation
modes. `ReplaceFileContent` takes the dictionary branch 40 percent of the time.

```rust
static CONTENT_DICTIONARY: &[&[u8]] = &[
    b"foobar",                              
    b"FOOBAR",
    b"",                                     // empty content
    b"\x7fELF",                              // ELF magic
    b"#!/bin/sh\n",                          // shell shebang
    b"[settings]\nverbose=1\ndebug=1\n",     // realistic config file
    b"\x00\x00\x00\x00",                     // 4 zero bytes
    b"\xff\xff\xff\xff",                     // all-ones
    b"../../../etc/passwd",                  // path traversal
    b"/dev/null",                            // special path
    b"%s%s%s%s",                             // format string
    b"A",                                    // single byte
    &[0xAA; 64],                             // 64 bytes alternating pattern
    &[0x00; 256],                            // 256 zero bytes (boundary size)
    &[0x41; 4096],                           // 4KB of 'A' (page-size content)
];
```

## 9. Quick Explain Script

Use this flow when explaining the setup:

```text
1. The baseline VFS is created: /input, /etc, /etc/config.
2. Three baseline path sets are collected: files, dirs, and all paths.
3. Seven seed deltas are generated from the primary and secondary baseline files.
4. Four hard-coded donor deltas are added for early splice diversity.
5. The live corpus starts with 11 FsDelta entries.
6. Each iteration picks one FsDelta, mutates it 1-3 times, and applies it.
7. If the resulting VFS checksum is novel, the mutated FsDelta is promoted.
8. The VFS resets to the saved baseline snapshot and the loop repeats.
```

## `guidance.rs` — Mutation Guidance Stub

```rust
pub struct MutationGuidance {
    pub write_paths:    Vec<String>,  // paths target wrote to / created / renamed into
    pub enoent_paths:   Vec<String>,  // paths target tried to open but ENOENT'd
    pub recreate_paths: Vec<String>,  // paths target deleted or renamed away
}
```

All examples start from the primary seed family:

```text
D0 = [ UpdateFile("/input", "seed", size=4) ]
```

#### `ByteFlipFileContent`

Flips bits in file content without changing path or size:

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After:  [ UpdateFile("/input", "semd", size=4) ]
```

The real bytes may be non-printable.  The important invariant is that the
content length stays 4.

#### `ReplaceFileContent`

Replaces the whole content buffer and updates `size`.  With 40% probability
draws from `CONTENT_DICTIONARY` (trigger strings, magic bytes, boundary
sizes); otherwise generates 1–64 random bytes:

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After (random):     [ UpdateFile("/input", [de ad be ef 00], size=5) ]
After (dictionary): [ UpdateFile("/input", "foobar", size=6) ]
After (dictionary): [ UpdateFile("/input", [0xAA; 64], size=64) ]
```

The dictionary carries values that are structurally interesting to parsers
(magic numbers, path-traversal markers, format strings, long fill patterns
sized at 64B / 256B / 4KB for boundary behaviour) as well as the Week 6
demo trigger string `"foobar"`.

#### `AddFileOp`

Appends a new file or directory:

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After:  [
  UpdateFile("/input", "seed", size=4),
  Mkdir("/tmp/run")
]
```

When `MutationGuidance.enoent_paths` is populated (Phase B), 70% of new
paths are drawn from there with a 90% file bias (vs 70% for random paths)
because the target tried to *open* a file at that path, not create a directory.

#### `RemoveOp`

Starting from the raw seed, it skips:

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After:  Skipped, because len <= 1
```

Within a multi-mutation chain, it can shrink a delta grown earlier in the
same iteration:

```text
Mutation 1 (AddFileOp): [ UpdateFile("/input", ...), Mkdir("/tmp/run") ]
Mutation 2 (RemoveOp):  [ Mkdir("/tmp/run") ]
```

This counterbalances `AddFileOp` and `SpliceDelta`.

#### `MutatePath`

Two modes, selected randomly:

- **Whole-path swap** (30% when any target-pool is non-empty): replaces the
  entire path with a known-interesting path.  Preference order:
  `guidance.enoent_paths` → `guidance.write_paths` → `guidance.recreate_paths` → `baseline_paths`.
  In Phase A the guidance lists are empty so this always draws from
  `baseline_paths`; in Phase B the ENOENT paths the target actually tried to
  open take precedence, and the mutator immediately converts failing random
  paths into paths the target is known to care about.
- **Component swap** (otherwise): replaces one segment with a `PATH_COMPONENTS`
  word, exploring the neighbourhood of the current path.

```text
Before: [ UpdateFile("/random/path", ...) ]
After (whole swap, ENOENT):     [ UpdateFile("/wanted/by/target", ...) ] ← highest priority
After (whole swap, write_paths):[ UpdateFile("/written/by/target", ...) ]← target wrote here
After (whole swap, recreate):   [ UpdateFile("/deleted/by/target", ...) ]← target deleted this
After (whole swap, baseline):   [ UpdateFile("/etc/config", ...) ]       ← fallback
After (component swap):         [ UpdateFile("/random/config", ...) ]    ← neighbour
```

#### `SpliceDelta`

Picks a **random start offset** in a donor delta drawn from the **live
corpus** (not a fixed pool — the same `Rc<RefCell<Vec<FsDelta>>>` the
harness uses for seed selection).  Appends a contiguous slice of ops:

```text
Donor: [
  UpdateFile("/etc/config", "verbose=1"),  ← index 0
  SetTimes("/input", 1700000000, 0, ...),  ← index 1
  Truncate("/input", 2),                   ← index 2
]
start = 1  →  slice = [ SetTimes(...), Truncate(...) ]
```

Possible output:

```text
[
  UpdateFile("/input", "seed"),
  SetTimes("/input", 1700000000, 0, ...),
  Truncate("/input", 2),
]
```

The random offset means late-donor ops (metadata ops at the tail of a
sequence) are reachable independently, not only when the entire prefix is
also spliced.`cp_ensure_parents` makes any slice structurally safe.  And
because the donor pool is the *live* corpus, any delta the harness promotes
(novel-checksum yield) becomes available as a splice donor on the next
iteration — the splice distribution shifts as the corpus evolves.

#### `DestructiveMutator`

Appends one destructive or metadata op using **op-type-aware path selection**:

| Op | Path drawn from |
|---|---|
| `DeleteFile` | `baseline_file_paths` (files only) — 70% bias |
| `Rmdir` | `baseline_dir_paths` (dirs only) — 70% bias |
| `Truncate` | `baseline_file_paths` (files only) — 70% bias |
| `SetTimes` | `baseline_all_paths` (any node) — 70% bias |

Drawing file paths for `Rmdir` would always produce ENOTDIR; drawing dir
paths for `Truncate` would always produce EISDIR.  The three separate lists
ensure each op gets semantically correct path candidates.

`SetTimes` timestamps use `pick_timestamp()`, which draws from a set of
interesting edge cases 40% of the time:
- `0` (epoch), `-1` (pre-epoch), `i32::MAX` (2038 boundary),
  `2_000_000_000` (post-2038 far future), `1_700_000_000` (~Nov 2023).

**Guidance bias.** `DeleteFile` and `Rmdir` both check
`guidance.recreate_paths` first and, with 50% probability when populated,
draw from there — the target has already shown it acts on these paths, so
re-deleting them exercises the same code path again.  Phase A runs with an
empty recreate list; Phase B feeds this from the FUSE `UNLINK` /
`RENAME_FROM` log.

```text
[
  UpdateFile("/input", "seed"),
  Truncate("/input", new_size=2),        ← file path used, not /etc
]
```

#### `UpdateExistingFile`

Appends an `UpdateFile` op.  Path is drawn from `guidance.write_paths ∩
baseline_file_paths` (70% bias when the intersection is non-empty — these
are paths the target actively wrote to AND that survive reset) falling back
to `baseline_file_paths`.  Non-baseline `write_paths` entries are handled
by `ReplayWriteFile` instead.  Content selection follows a three-way
strategy:

| Strategy | Probability | Behaviour |
|---|---|---|
| **Real-content perturbation** | 50% when baseline content is available | read the live baseline content for the chosen path; apply one of bit-flip / append / truncate / dictionary-splice |
| **Dictionary draw** | 30% otherwise | pick an entry from `CONTENT_DICTIONARY` |
| **Random bytes** | 70% otherwise | 1–64 uniform random bytes |

This is the highest-value mutation for reaching deep parser state:
targets that read structured content (e.g. `/etc/config`) keep most of the
structure intact under perturbation and reach downstream logic that random
bytes would never reach.  When `write_paths` is active, the path is one
the target confirmed it just wrote — so the mutated content is guaranteed
to be read back.  Constructor chain:

```rust
UpdateExistingFile::new(baseline_file_paths)
    .with_baseline_contents(vec![
        ("/input".into(),      b"seed".to_vec()),
        ("/etc/config".into(), b"[settings]\nverbose=0\n".to_vec()),
    ])
```

Example perturbations from the `/etc/config` baseline
`b"[settings]\nverbose=0\n"` (20 bytes):

```text
bit-flip:           "[settings]\nvdrbose=0\n"         (20 B, 1 bit differs)
append:             "[settings]\nverbose=0\n\x3a\xf1" (22 B)
truncate:           "[settin"                         (7 B)
dictionary-splice:  "[settings]\nverbose=0\nfoobar"   (26 B, "foobar" inserted)
```

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After:  [
  UpdateFile("/input",      "seed", size=4),
  UpdateFile("/etc/config", "[settings]\nverbose=1\n", size=21)   ← perturb
]
```

Skips when `baseline_file_paths` is empty or the delta is at `MAX_OPS`.

#### `ReplayWriteFile`

Covers the complement of `UpdateExistingFile`: `guidance.write_paths ∖
baseline_file_paths` — paths the target *created* mid-run that were wiped
by VFS reset and therefore have no node in the next iteration.  Emits
`CreateFile(path, content)`; `cp_ensure_parents` handles missing parent
directories automatically (it is called for every `CREATE_FILE` op).

Content selection: dictionary (30%) or random (70%).  Real-content
perturbation is unavailable because there is no baseline snapshot of
target-created files.  In Phase B the FUSE write log will capture actual
write bytes, enabling exact content replay.

```text
guidance.write_paths = ["/input", "/tmp/output"]
baseline_file_paths  = ["/input"]

UpdateExistingFile → may pick "/input"   (∩ baseline)
ReplayWriteFile    → always picks "/tmp/output"  (∖ baseline)
```

Skips when `write_paths ∖ baseline_file_paths` is empty (Phase A default —
guidance is unpopulated) or at `MAX_OPS`.

## Dumnb Loop Architecture

```
  VFS baseline  (after populate_baseline + vfs_save_snapshot)
     │
     │  cp_enumerate_paths(vfs, filter=1)  →  baseline_file_paths
     │  cp_enumerate_paths(vfs, filter=2)  →  baseline_dir_paths
     │  cp_enumerate_paths(vfs, filter=0)  →  baseline_all_paths
     │  known at populate time              →  baseline_contents (path, bytes)
     │
     │  live_corpus = generate_seed_corpus(…) + initial_corpus_pool()
     │      wrapped in Rc<RefCell<Vec<FsDelta>>>; shared with SpliceDelta
     │      bounded at MAX_LIVE_CORPUS = 128
     ▼
  per iteration
     │  ► pick a starting delta from live_corpus
     │  ► pick 1–3 mutators from the can_apply subset (skip-early filter)
     │  ► apply each mutation to the delta in sequence
     │
     ▼
  FsDelta  ←── 9 Rust mutator stages (mutators.rs)
     │           MAX_OPS = 20 cap enforced by AddFileOp, SpliceDelta,
     │           DestructiveMutator, UpdateExistingFile, ReplayWriteFile
     │           DestructiveMutator: op-type-aware (files/dirs/all, 70% bias);
     │                               50% recreate_paths bias on DeleteFile/Rmdir
     │           UpdateExistingFile: path from write_paths∩baseline (70%) → baseline;
     │                               content = perturb(baseline) 50% / dict 30% / random 70%
     │           ReplayWriteFile:    path from write_paths∖baseline → CreateFile
     │                               content = dict 30% / random 70%
     │           MutatePath: 30% whole-path swap (enoent → write → recreate → baseline)
     │           SpliceDelta: random start offset; donors drawn from live_corpus
     │           ReplaceFileContent: 40% dictionary / 60% random
     │
     │  ffi::apply_delta(vfs, &delta)
     │    validate_delta()  [debug_assert]
     │    delta_add_*() × n_ops  [all 7 FsOpKind variants]
     ▼
  C fs_delta_t
     │
     │  cp_apply_delta(vfs, delta, dry_run=0)
     ▼
  cp_result_t  { total_ops, succeeded, failed, results[] }
     │
     │  read succeeded/failed → DeltaResult
     │  cp_result_free()
     ▼
  Ok(DeltaResult)  or  Err(errno) on catastrophic failure only
     │
     │  cp_vfs_checksum(vfs)  →  semantic yield check
     │    (alphabetically sorted walk — insertion-order independent)
     │
     │  if checksum is novel: push delta → live_corpus
     │                        (evict random non-seed entry when at cap)
     │
     │  vfs_reset_to_snapshot(vfs)
     ▼
  baseline VFS state  (restored for next iteration)
```