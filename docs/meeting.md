# Seed Corpus and Baseline Reference

## At a Glance

| Item | Type | Purpose |
| --- | --- | --- |
| `baseline_file_paths` | `Vec<String>` | Existing baseline files used for file-only targeting |
| `baseline_dir_paths` | `Vec<String>` | Existing baseline directories used for directory-only targeting |
| `baseline_all_paths` | `Vec<String>` | Existing baseline files and directories used for general path targeting |
| `rootfs_seeds(bin_true)` | `Vec<FsDelta>` | ~35 rootfs seed deltas (ELF corruption, dir/file removals, rich trees) |
| `crun_symlink_seeds(index)` | `Vec<FsDelta>` | ~32 symlink-replacement seed deltas (mount / exec / escape / loop) |
| `live_corpus` | `Vec<FsDelta>` | Starts with `rootfs_seeds + crun_symlink_seeds` (~67), grows with promoted novel deltas; also the `SpliceDelta` donor pool |

Key distinction:

```text
FsOp       = one filesystem operation  (8 kinds: CreateFile, UpdateFile, DeleteFile,
              Mkdir, Rmdir, SetTimes, Truncate, CreateSymlink)
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
    fs_op_kind_t     kind;         /* which of the 8 ops (enum value 1–8) */
    char            *path;         /* absolute path, heap-allocated, NUL-terminated */
    uint8_t         *content;      /* CREATE_FILE, UPDATE_FILE: heap-allocated bytes */
                                   /* SYMLINK: heap-allocated target string (NUL-terminated) */
                                   /* NULL for RMDIR, MKDIR, DELETE_FILE, SET_TIMES */
    size_t           content_len;  /* CREATE_FILE / UPDATE_FILE: byte count of content */
                                   /* TRUNCATE: new file size (no content bytes written) */
                                   /* SYMLINK: byte length of the target string */
                                   /* all others: 0 */
    struct timespec  mtime;        /* SET_TIMES: desired mtime; zero for others */
    struct timespec  atime;        /* SET_TIMES: desired atime; zero for others */
} fs_op_t;
```

The `content_len` field is triple-purpose by design: for CREATE/UPDATE it is the number of bytes in the content buffer; for TRUNCATE it is the target file size with no bytes in the buffer; for SYMLINK it is the byte length of the target string (stored in `content`, NUL-terminated by `strndup` before the VFS call). This keeps the struct small — no extra `new_size` or `target` field needed.

The 8 op kinds:

```c
typedef enum {
    FS_OP_CREATE_FILE = 1,
    FS_OP_UPDATE_FILE = 2,
    FS_OP_DELETE_FILE = 3,
    FS_OP_MKDIR       = 4,
    FS_OP_RMDIR       = 5,
    FS_OP_SET_TIMES   = 6,
    FS_OP_TRUNCATE    = 7,
    FS_OP_SYMLINK     = 8,
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
delta_add_symlink(d, "/proc", "../../proc");    /* create symlink */
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
build live_corpus = rootfs_seeds(BIN_TRUE) + crun_symlink_seeds(baseline_index)
  -> LibAFL scheduler picks a corpus entry (seed)
  -> havoc stage applies a stacked batch of mutations to a clone of the seed
     (guidance-biased; see the mutation-scheduling notes)
  -> apply_delta(vfs, &delta) replays the ops into the FUSE VFS
  -> crun executes the container against the FUSE rootfs
  -> the FUSE access log → MutationGuidance for the next mutate()
  -> new AFL edge coverage (MaxMapFeedback) promotes the input into the corpus
     (live_corpus, shared with SpliceDelta, bounded at MAX_LIVE_CORPUS = 128)
  -> reset VFS back to the saved snapshot
```

## 1. Baseline VFS Setup

`init_vfs(vfs, BIN_TRUE)` builds a minimal but realistic crun container rootfs
in the in-memory VFS, then snapshots it. Every fuzzing iteration resets back to
this snapshot.

Directories:

```text
/bin /proc /dev /sys /tmp /etc /var /run /usr /usr/bin /app /home /home/user
```

Binaries — every executable the grammar can reference is the same static
`exit(0)` ELF blob `BIN_TRUE`, so crun's `find_executable` succeeds whichever
process path the config names:

```text
/bin/true   /bin/sh   /bin/bash   /app/app   /usr/bin/nginx
```

`/etc` files crun actually reads from the rootfs:

```text
/etc/passwd       root + nobody entries
/etc/group        root / daemon / bin / nobody
/etc/hosts        127.0.0.1 localhost / ::1 localhost
/etc/hostname     "fuzz"
/etc/resolv.conf  nameserver 8.8.8.8
```

`BIN_TRUE` is `include_bytes!("../../static/true")` — a raw `exit(0)` syscall
built with `gcc -static -nostartfiles -nostdlib -Os`.

## 2. Baseline Path Sets

After baseline setup, the harness enumerates three stable path sets.

| Path set | Source | Meaning | Typical values |
| --- | --- | --- | --- |
| `baseline_file_paths` | `enumerate_vfs_file_paths(vfs)` | Files only | `/bin/true`, `/etc/passwd`, `/etc/hostname`, … |
| `baseline_dir_paths` | `enumerate_vfs_dir_paths(vfs)` | Directories only | `/bin`, `/proc`, `/etc`, `/usr/bin`, … |
| `baseline_all_paths` | `enumerate_vfs_all_paths(vfs)` | Files and directories | all of the above |

These path sets are calculated from the clean baseline and reused by mutators
for type-correct targeting.

Examples:

```text
UpdateExistingFile -> baseline_file_paths
Rmdir              -> baseline_dir_paths
SetTimes           -> baseline_all_paths
```

## 3. Baseline Contents

`UpdateExistingFile` can perturb real baseline file content. The baseline
content map is:

| Path | Content |
| --- | --- |
| `"/etc/passwd"` | `"root:x:0:0:root:/root:/bin/sh\n"` |
| `"/bin/true"` | `BIN_TRUE` (static `exit(0)` ELF blob) |

This helps content mutations preserve useful structure while still changing
bytes.

## 4. Rootfs Seed Deltas (`rootfs_seeds`)

`rootfs_seeds(BIN_TRUE)` returns ~35 `FsDelta` seeds (31 static + 4 derived from
the real ELF when `BIN_TRUE.len() > 8`). Each seed is one delta applied on top
of the baseline rootfs, grouped by what it breaks in crun's startup path:

| Group | Examples |
| --- | --- |
| Empty / mount-point removal | `[]` (pristine baseline), `rmdir("/proc")`, `rmdir("/dev")`, …, and all four of `/proc /dev /sys /tmp` together |
| Bad container binary | `truncate("/bin/true", 0/4/16/64)`, `update("/bin/true", "not an elf")`, 32-bit ELF magic, 64-bit magic + zeroed header, shebang script, `delete("/bin/true")` (+ `rmdir("/bin")`) |
| Rich valid rootfs | full Linux dir tree (`/usr /usr/bin /usr/lib /lib /lib64 /sbin /opt /home /root /var/log /var/tmp /var/run /run/lock`); populated `/dev` nodes (`null zero full random urandom tty console ptmx`, `pts/`, `shm/`); extra `/etc` files (`group shadow subuid subgid nsswitch ld.so.cache ld.so.conf`, `.dockerenv`); extra `/bin` binaries (`sh bash ls`) |
| Rich rootfs + one defect | full tree but missing `/proc`; full tree + corrupted binary; full tree + missing binary |
| `/etc` removals | delete `passwd` / `hosts`; delete `passwd hosts hostname resolv.conf` together |
| OCI-specific | `.dockerenv`, `ld.so.cache`, pre-populated `/proc/mounts` + `/proc/self` |
| ELF-derived (from `BIN_TRUE`) | flip ELF class byte (offset 4), flip endianness byte (offset 5), truncate the real blob to 128 B / 64 B |

Seed 1 is the **empty delta** — the unmodified baseline rootfs — so the pristine
rootfs is always present in the corpus.

## 5. Symlink Seed Deltas (`crun_symlink_seeds`)

`crun_symlink_seeds(&baseline_index)` returns ~32 seeds that pre-plant symlink
structures (built via `replace_with_symlink`, so each correctly tears down the
existing path first). They mirror the themes the symlink mutators explore:

| Theme | Examples |
| --- | --- |
| Mount-point → relative escape | `/proc → ../../proc`, `/dev → ../../dev`, `/sys`, `/tmp` (+ a combined proc/dev/sys delta) |
| Mount-point → special / wrong | `/dev → /proc/self/fd`, `/proc → /proc/self/exe`, `/proc → /etc/passwd`, `/proc → /nonexistent`, `/dev → /missing` |
| Parent-component escape | `/etc → ../../etc`, `/bin → ../../bin`, `/lib`, `/usr` |
| Exec hijack | `/bin/true →` `/proc/self/exe` `/proc/self/mem` `/proc/self/fd/0` `/dev/zero` `/dev/null` `../../usr/bin/python3`; `/bin/sh → ../../../proc/sysrq-trigger` |
| `/etc/passwd` redirection | `→ ../../etc/passwd`, `→ /etc/passwd`, `→ ../../../etc/shadow`; `/etc/group → ../../etc/group` |
| Loops / cycles | `/loop → /loop` (self-loop), `/a→/b→/c→/a` (3-cycle) |

### Live Corpus

```text
live_corpus = rootfs_seeds(BIN_TRUE) + crun_symlink_seeds(baseline_index)
            ≈ 35 + 32 ≈ 67 FsDelta entries
```

The live corpus is wrapped in `Rc<RefCell<Vec<FsDelta>>>` and shared with
`SpliceDelta` as its donor pool — there is no separate hard-coded donor pool
(the old `initial_corpus_pool` is dumb-loop-only). Inputs the harness promotes
on new edge coverage are pushed here and become splice donors on the next
iteration; bounded at `MAX_LIVE_CORPUS = 128` with random non-seed eviction.

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
    b"random_shit",
    b"cone_ice",
    b"paal_ice",
    b"chocobar",
    b"fahhhhhhh",
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
1. init_vfs builds the baseline crun rootfs (dirs + static binaries + /etc files) and snapshots it.
2. Three baseline path sets are collected: files, dirs, and all paths.
3. rootfs_seeds builds ~35 deltas (ELF/dir/file defects); crun_symlink_seeds adds ~32 symlink deltas.
4. The live corpus starts with ~67 FsDelta entries (also the SpliceDelta donor pool).
5. The LibAFL scheduler picks a corpus entry; the havoc stage applies a stacked batch of mutations to a clone.
6. FUSE access during the crun run populates MutationGuidance, biasing the next mutate().
7. apply_delta replays the FsDelta into the FUSE VFS; crun executes the container against it.
8. New AFL edge coverage promotes the input; the VFS resets to the snapshot and the loop repeats.
```

## `guidance.rs` — Mutation Guidance (live, FUSE-driven)

Guidance is **live**, not a stub.  In the combined LibAFL fuzzer
(`fuzz_combined_afl.rs`) the `FuseLogObserver` populates it every iteration from
the FUSE access log, and the mutators read it at `mutate()` time.

```rust
pub struct MutationGuidance {
    pub write_paths:    Vec<String>,  // paths target wrote to / created / renamed into
    pub enoent_paths:   Vec<String>,  // paths target tried to open but ENOENT'd
    pub recreate_paths: Vec<String>,  // paths target deleted or renamed away
}
```

**How it is populated (per iteration):**

```text
FuseLogObserver.pre_exec   → fuse_log_clear() + activate logging
   target runs             → every FUSE callback appends (path, kind) to the log
FuseLogObserver.post_exec  → drain log into the 3 buckets → guidance::update_live()
mutators (in mutate())     → guidance::peek_live() reads the latest snapshot
```

**FUSE event → bucket mapping** (9 logged kinds folded into 3 buckets):

| Bucket | Fed by FUSE events |
|---|---|
| `enoent_paths`   | `ENOENT` (getattr miss — the most coverage-expanding signal) |
| `write_paths`    | `WRITE`, `CREATE`, `RENAME_TO`, `MKDIR`, `SYMLINK` |
| `recreate_paths` | `UNLINK`, `RENAME_FROM` |

> `RMDIR` is logged by the FUSE layer but currently **not** mapped into any
> bucket — `recreate_paths` only ever sees file unlinks and rename-froms.

**Fallback.** When the buckets are empty — the standalone dumb loop
(`fuzz.rs`, which never populates guidance) or the first iterations before the
target touches the filesystem — every guided branch below falls back to
baseline/random selection.

The per-mutator examples below use an **illustrative** delta `D0`. The path
`/input` is a readability stand-in — it is not part of the current crun
baseline rootfs (use e.g. `/etc/passwd` or `/bin/true` for a real baseline
path):

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
After (dictionary): [ UpdateFile("/input", "cone_ice", size=8) ]
After (dictionary): [ UpdateFile("/input", [0xAA; 64], size=64) ]
```

The dictionary carries values that are structurally interesting to parsers
(magic numbers, path-traversal markers, format strings, long fill patterns
sized at 64 B / 256 B / 4 KB for boundary behaviour).  The first five entries
(`random_shit`, `cone_ice`, `paal_ice`, `chocobar`, `fahhhhhhh`) are ad-hoc
marker strings, handy for spotting dictionary-sourced content in logs.

#### `AddFileOp`

Appends a new file or directory:

```text
Before: [ UpdateFile("/input", "seed", size=4) ]
After:  [
  UpdateFile("/input", "seed", size=4),
  Mkdir("/tmp/run")
]
```

When `guidance.enoent_paths` is populated (live, from the FUSE `ENOENT` log),
70% of new paths are drawn from there with a 90% file bias (vs 70% for the
empty-guidance fallback) because the target tried to *open* a file at that
path, not create a directory.

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
  With live FUSE guidance the ENOENT paths the target actually tried to open
  take precedence, so the mutator immediately converts failing random paths
  into paths the target is known to care about; when guidance is empty
  (standalone dumb loop / cold start) it falls back to `baseline_paths`.
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
re-deleting them exercises the same code path again.  `recreate_paths` is fed
live from the FUSE `UNLINK` / `RENAME_FROM` log; with an empty list (standalone
dumb loop / cold start) it uses the baseline file/dir lists only.

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
        ("/etc/passwd".into(), b"root:x:0:0:root:/root:/bin/sh\n".to_vec()),
        ("/bin/true".into(),   BIN_TRUE.to_vec()),
    ])
```

Example perturbations on an illustrative 20-byte config buffer
`b"[settings]\nverbose=0\n"`:

```text
bit-flip:           "[settings]\nvdrbose=0\n"         (20 B, 1 bit differs)
append:             "[settings]\nverbose=0\n\x3a\xf1" (22 B)
truncate:           "[settin"                         (7 B)
dictionary-splice:  "[settings]\nverbose=0\ncone_ice" (28 B, "cone_ice" inserted)
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
target-created files.  (The FUSE log currently records only `(path, kind)`,
not write bytes; capturing the bytes is a future extension that would enable
exact content replay.)

```text
guidance.write_paths = ["/input", "/tmp/output"]
baseline_file_paths  = ["/input"]

UpdateExistingFile → may pick "/input"   (∩ baseline)
ReplayWriteFile    → always picks "/tmp/output"  (∖ baseline)
```

Skips when `write_paths ∖ baseline_file_paths` is empty (the standalone
dumb-loop / cold-start default — guidance unpopulated) or at `MAX_OPS`.

## Symlink Mutators (`symlink_mutators.rs`)

Six structure-aware mutators that turn paths into symlinks to probe crun's
path-, mount-, and exec-resolution. They are registered in the combined and
rootfs AFL fuzzers (`fuzz_combined_afl.rs`, `fuzz_rootfs_afl.rs`) — **not** the
standalone dumb loop. `MountDestinationSymlinkMutator` is registered twice, so
it draws double weight. They **append** to the delta and never read or modify
existing ops. Five of them route through one helper.

### `replace_with_symlink(path, target, index)`

You cannot create a symlink where a file/dir already exists (`EEXIST`), so this
helper consults the **`BaselineIndex`** (a one-time snapshot of the seed rootfs,
built at startup) and emits the cleanup ops needed to free the name first:

| `path` in baseline | Ops emitted |
|---|---|
| absent | `[CreateSymlink(path, target)]` |
| file / symlink | `[DeleteFile(path), CreateSymlink(path, target)]` |
| empty dir | `[Rmdir(path), CreateSymlink(path, target)]` |
| non-empty dir | `DeleteFile`/`Rmdir` each child (deepest first), `Rmdir(path)`, then `CreateSymlink(path, target)` |

Targets are stored as **opaque strings** — creating a symlink never resolves or
checks the target, so dangling links and cycles are created successfully and
only fail (`ENOENT` / `ELOOP`) when something later *follows* them. The
`BaselineIndex` reflects the seed rootfs, not the post-delta state, so a cleanup
op may no-op at apply time; that is tolerated.

### The six mutators

All examples assume a baseline where `/proc /dev /sys /tmp` are empty dirs,
`/etc` holds `/etc/passwd`, and `/bin` holds `/bin/true`.

**1. `MountDestinationSymlinkMutator`** — replaces a mount destination
(`/proc /dev /sys /tmp`) with a symlink. Target: 35% relative escape
(`../`×2–5), 25% absolute (`/proc/self/exe`, …), 20% wrong-type
(`/etc/passwd` for `/proc`, else `/bin/true`), 15% `/nonexistent`, 5%
`/dev/null`|`/proc/self/fd`. Targets the FUSE-shadowing / mount-redirect class.

```text
Append (picks /tmp, relative): [ Rmdir("/tmp"), CreateSymlink("/tmp", "../../../tmp") ]
```

**2. `MountOptionSymlinkMutator`** — same destinations, fixed target set
(`/proc/self/fd`, `/proc/self/exe`, `/dev/null`, `../../tmp`, `/nonexistent`).

```text
Append (picks /sys): [ Rmdir("/sys"), CreateSymlink("/sys", "/proc/self/exe") ]
```

**3. `ExecutableSymlinkMutator`** — points a container binary path
(`/bin/target`, `/bin/exploit`, `/sbin/target`, `/usr/local/bin/target`) at
`/proc/self/exe`, `/proc/self/mem`, `/dev/zero`, `../../bin/sh`, … The
CVE-2019-5736 shape (exec resolution).

```text
Append (path absent → single op): [ CreateSymlink("/bin/target", "/proc/self/exe") ]
```

**4. `ParentComponentSymlinkMutator`** — poisons an intermediate dir component
(`/etc /bin /lib /usr /dev /var /run`); 50% relative escape, 50% absolute. One
mutation redirects every lookup that passes through that component.

```text
Append (picks /etc, a non-empty dir):
  [ DeleteFile("/etc/passwd"), Rmdir("/etc"), CreateSymlink("/etc", "../../etc") ]
```

**5. `SymlinkEscapeMutator`** — generic traversal. Target: 40% absolute, 60%
deep relative (`../`×2–8 + `etc/passwd`|`proc/self/exe`|…). Path: 60% a fresh
`<base>/escape`, 40% hijacks an existing baseline entry.

```text
Append (fresh path):  [ CreateSymlink("/etc/escape", "../../../../proc/self/exe") ]
Append (hijack file): [ DeleteFile("/etc/passwd"), CreateSymlink("/etc/passwd", "/proc/self/exe") ]
```

**6. `LoopAndDepthMutator`** — ignores the baseline; fabricates pathological
structures to break the resolver. 20% self-loop, 20% two-cycle, 30% chain of N
(`N ∈ {1,5,10,39,40,41}`, straddling the kernel's 40-link `ELOOP` limit), 15%
near-`PATH_MAX` target, 15% repeated slashes. All fresh paths → no cleanup ops.

```text
Append (self-loop): [ CreateSymlink("/fuzz_loop", "/fuzz_loop") ]
Append (two-cycle): [ CreateSymlink("/fuzz_a", "/fuzz_b"), CreateSymlink("/fuzz_b", "/fuzz_a") ]
Append (chain N=5): [ CreateSymlink("/s4","/s5"), CreateSymlink("/s3","/s4"),
                      CreateSymlink("/s2","/s3"), CreateSymlink("/s1","/s2"),
                      CreateSymlink("/s0","/proc") ]
```

## Loop Architecture (`fuzz_combined_afl.rs`)

The real campaign is a LibAFL fuzzer driving **crun** through an AFL++
forkserver, against a rootfs served live by an in-process FUSE daemon. An input
is a `CombinedInput { config: Nautilus OCI-config tree, rootfs: FsDelta }` —
both halves are mutated together.

```
  ONE-TIME SETUP
     │  init_vfs(vfs, BIN_TRUE)  →  baseline crun rootfs  →  vfs_save_snapshot
     │  BaselineIndex::build(vfs);  baseline_{file,dir,all}_paths
     │  start_fuse(vfs, mountpoint)  →  FUSE serves the VFS from a background thread
     │  shmem (coverage map) + __AFL_SHM_ID env var
     │  observers  = [ FuseLogObserver , TimeObserver , HitcountsMapObserver(edges) ]
     │  feedback   = MaxMapFeedback(edges)  OR  TimeFeedback     ← corpus admission
     │  objective  = CrashFeedback                               ← saved to crashes/
     │  scheduler  = IndexesLenTimeMinimizerScheduler( QueueScheduler )
     │  mutators   = ConfigMutator(Nautilus) + 9 RootfsMutators + 6 SymlinkMutators
     │  stages     = StdMutationalStage( HavocScheduledMutator ) , AflStatsStage
     │  executor   = CalibratedExecutor( ForkserverExecutor(crun, config.json) )
     │  seeds = Nautilus configs ⊕ (baseline_config × rootfs_seeds) ⊕ crun_symlink_seeds
     │  live_corpus = rootfs deltas  (Rc<RefCell<…>>, shared with SpliceDelta)
     ▼
  fuzzer.fuzz_one()  ── repeated forever ───────────────────────────────────────
     │
     │  1. scheduler picks a corpus entry:
     │        CombinedInput { config: Nautilus tree , rootfs: FsDelta }
     │
     │  2. havoc stage — C = 1..128 rounds on a CLONE of the seed; each round
     │     stacks N ∈ {2,4,…,128} mutations, each a uniform pick from the tuple:
     │        ConfigMutator    → mutates the Nautilus OCI-config tree
     │        Rootfs/Symlink   → mutate the FsDelta (guidance-biased via peek_live())
     │
     │  3. TargetBytesConverter.to_target_bytes(mutated input):
     │        vfs_reset_to_snapshot(vfs)        → baseline rootfs
     │        apply_delta(vfs, input.rootfs)    → mutated rootfs in the live FUSE VFS
     │        render config tree → JSON
     │        override_rootfs_path()            → inject {"type":"mount"} ns +
     │                                            set root.path = FUSE mountpoint
     │        write config.json to disk
     │
     │  4. CalibratedExecutor.run_target → ForkserverExecutor:
     │        FuseLogObserver.pre_exec   → fuse_log_clear() + activate logging
     │        forkserver forks a crun child → reads config.json, builds the container
     │                                         against the FUSE rootfs, execs the process
     │        FUSE callbacks append (path, kind) as crun walks the rootfs
     │        child exits → wait status; edge hits land in SHM
     │        observers.post_exec:
     │            edges / time            → coverage + timing
     │            FuseLogObserver         → drain log → 3 buckets → guidance::update_live()
     │        if ExitKind::Crash → re-run `reruns`× ; keep only if it reproduces
     │            ≥ threshold AND the config did NOT request the kill
     │            (RLIMIT_CPU / RLIMIT_FSIZE / seccomp-KILL signal filter)
     │
     │  5. feedback decides admission:
     │        MaxMapFeedback new edge → add CombinedInput to corpus
     │            → write sidecar, push input.rootfs into live_corpus,
     │              add Nautilus tree to NautilusChunksMetadata
     │        CrashFeedback (objective) → save to crashes/
     │        AflStatsStage → fuzzer_stats / plot_data
     │
     │  6. export each new corpus entry to --sync-dir;
     │     every 256 iters import peer instances' entries (distributed fuzzing)
     │
     ▼
  ON "Unable to communicate with fork server" (forkserver death):
     │  kill_stray_crun_in_cwd();  drop + rebuild ForkserverExecutor with the SAME
     │  shmem_ptr / __AFL_SHM_ID (virgin map lives on the fuzzer heap → no coverage
     │  loss); re-wrap in CalibratedExecutor; append to restarts.log; continue
```

Notes:
- **Two mutated halves.** `ConfigMutator` wraps the Nautilus grammar mutator for
  the OCI config; the 9 `mutators.rs` + 6 `symlink_mutators.rs` mutators (each
  wrapped in `RootfsMutator`) mutate the `FsDelta`. The havoc scheduler picks
  uniformly across all of them every stack step.
- **The FUSE VFS is the rootfs.** `apply_delta` mutates the *same* in-process VFS
  that the FUSE daemon serves to crun — there is no on-disk rootfs image. The VFS
  is reset to the snapshot before every input via the target-bytes converter.
- **Coverage, not checksum.** Corpus admission is AFL edge coverage
  (`MaxMapFeedback`); the old dumb-loop "novel VFS checksum" promotion is gone.
- **`MAX_OPS = 48`**, `MAX_LIVE_CORPUS = 128`. The standalone `fuzz.rs` dumb loop
  (checksum-novelty, `/input` baseline, `generate_seed_corpus` +
  `initial_corpus_pool`, no FUSE/guidance) still exists but is legacy.