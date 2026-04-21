# Seed Corpus and Baseline Reference

This document is a practical reference for the Week 5 Phase A input setup. It
summarizes the baseline VFS, the path sets derived from it, the seed corpus,
the initial donor corpus, and the vocabulary used by the mutators.

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

## 1. Baseline VFS Setup

The harness starts from a fixed baseline in `mutator/src/bin/fuzz.rs`.

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
/// Dictionary of structurally interesting content values.
///
/// Covers: trigger strings the Week 6 demo target will crash on, magic bytes
/// for common file formats, boundary / overflow markers, and path-shaped
/// strings that sometimes confuse parsers. `ReplaceFileContent` draws from
/// this pool with 40% probability; the other 60% fall back to random bytes so
/// the mutator still explores unstructured space.
static CONTENT_DICTIONARY: &[&[u8]] = &[
    b"foobar",                               // Week 6 demo crash trigger
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
