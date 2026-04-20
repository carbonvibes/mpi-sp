# Mutator README (Week 5 Phase A)

This document explains, in order:

1. how the generator creates a valid seed delta from scratch,
2. how that seed is handed to the mutator loop,
3. how each mutator stage transforms the delta,
4. concrete examples for all 7 stages starting from the generator output.

## 1) Core Data Model

The mutator works on a Rust-native input type:

- `FsDelta`: a list of operations (`ops: Vec<FsOp>`)
- `FsOp`: one filesystem operation
- `FsOpKind`: operation kind

Supported `FsOpKind` values:

- `CreateFile`
- `UpdateFile`
- `DeleteFile`
- `Mkdir`
- `Rmdir`
- `SetTimes`
- `Truncate`

A single `FsOp` contains:

- `kind`
- `path` (absolute path, should start with `/`)
- `content` (used by `CreateFile` and `UpdateFile`)
- `size` (content length for file ops, truncate target size for `Truncate`)
- timestamp fields (used by `SetTimes`)

## 2) Generator: Creating the First `FsDelta`

The generator is very small in Phase A.  It is not a smart random generator
yet.  Its job is simply:

1. create one valid `FsDelta`,
2. make sure the delta has at least one operation,
3. make sure the operation is internally consistent,
4. give the mutator loop a safe starting point.

The generator function is in `src/delta.rs`:

```rust
pub fn generate_seed() -> FsDelta {
    FsDelta::new(vec![FsOp::create_file("/input", b"seed".to_vec())])
}
```

Read that from inside to outside:

```text
b"seed".to_vec()
```

creates the file content bytes:

```text
[0x73, 0x65, 0x65, 0x64]
```

Those bytes are the ASCII string:

```text
"seed"
```

Then:

```rust
FsOp::create_file("/input", b"seed".to_vec())
```

creates one filesystem operation:

```text
Create a file at path "/input" with content "seed".
```

Then:

```rust
FsDelta::new(vec![ ... ])
```

wraps that one operation in a delta:

```text
FsDelta = a list of filesystem operations
```

So the generated seed is exactly:

```text
FsDelta {
  ops: [
    FsOp {
      kind: CreateFile,
      path: "/input",
      content: [73 65 65 64],   // "seed"
      size: 4,
      mtime_sec: 0,
      mtime_nsec: 0,
      atime_sec: 0,
      atime_nsec: 0,
    }
  ]
}
```

### What `FsOp::create_file()` Actually Fills In

The generator does not manually fill every field.  It calls the constructor:

```rust
pub fn create_file(path: impl Into<String>, content: Vec<u8>) -> Self {
    let size = content.len();
    Self {
        kind: FsOpKind::CreateFile,
        path: path.into(),
        content,
        size,
        mtime_sec: 0,
        mtime_nsec: 0,
        atime_sec: 0,
        atime_nsec: 0,
    }
}
```

For the generator call:

```rust
FsOp::create_file("/input", b"seed".to_vec())
```

the constructor produces:

| Field | Value | Why |
|---|---|---|
| `kind` | `CreateFile` | We are creating a file |
| `path` | `"/input"` | This is the initial file path |
| `content` | bytes for `"seed"` | The initial file content |
| `size` | `4` | `"seed"` is 4 bytes |
| `mtime_sec` | `0` | Not used for `CreateFile` |
| `mtime_nsec` | `0` | Not used for `CreateFile` |
| `atime_sec` | `0` | Not used for `CreateFile` |
| `atime_nsec` | `0` | Not used for `CreateFile` |

This is why using constructors matters.  The constructor automatically keeps:

```text
size == content.len()
```

For the seed:

```text
content = "seed"
content.len() = 4
size = 4
```

### Why This Seed Is Valid

- non-empty op list (`ops.len() == 1`)
- absolute path (`/input`)
- `size == content.len()` for `CreateFile`
- timestamps are zero because `CreateFile` does not use them

The seed is intentionally boring.  It is not supposed to be the clever part of
the fuzzer.  It is a minimal valid input that proves the rest of the pipeline
can mutate, apply, measure, and reset.

### What the Generator Does Not Do Yet

In Phase A, the generator does **not**:

- inspect the target program,
- inspect the current VFS tree,
- learn paths from FUSE,
- generate many seed families,
- generate realistic config files,
- choose operation sequences,
- ensure parent directories for deep paths.

It only creates:

```text
[ CreateFile("/input", "seed") ]
```

That means the current generator is a scaffold.  The more interesting behavior
is expected to come from future feedback-guided generation and smarter
operation-aware mutators.

## 3) How Seed Is Given To Mutators

In the dumb loop harness (`src/bin/fuzz.rs`), the seed is built once:

```rust
let seed = generate_seed();
```

Then every iteration starts by cloning that seed:

```rust
for i in 0..n_iters {
    let mut delta = seed.clone();
    ...
}
```

So iteration 0 starts with:

```text
[ CreateFile("/input", "seed") ]
```

Iteration 1 also starts with:

```text
[ CreateFile("/input", "seed") ]
```

And so on.

The loop does **not** currently say:

```text
use the previous iteration's mutated delta as the next seed
```

It says:

```text
always go back to the original generated seed
```

That is why Phase A is called a dumb loop.  It is designed to test the mechanics
of mutation, application, and reset, not to evolve a sophisticated corpus yet.

The actual flow is:

1. Build seed once: `let seed = generate_seed();`
2. For each iteration: `let mut delta = seed.clone();`
3. Randomly pick one mutator stage.
4. Mutate `delta` in place.
5. Apply mutated delta to VFS through FFI (`apply_delta`).
6. Reset VFS snapshot.

Important: each iteration starts from the same seed clone, so behavior is
reproducible and mutation effects are isolated.

### Generator vs Baseline VFS

One confusing detail: the generated seed and the baseline VFS are related, but
they are not the same thing.

The generator creates a delta:

```text
[ CreateFile("/input", "seed") ]
```

Separately, `populate_baseline()` creates the initial VFS state:

```text
/input        file with content "seed"
/etc          directory
/etc/config   file with content "[settings]\nverbose=0\n"
```

That means applying the unmodified seed to the baseline may fail at the VFS
level, because `/input` already exists:

```text
CreateFile("/input", "seed") -> EEXIST-style failure
```

That is okay.  Phase A tracks this as a per-op failure inside `DeltaResult`,
not as a harness crash.  A mutator like `AddFileOp` or `SpliceDelta` may append
extra operations that do change the VFS and produce semantic yield.

## 4) Generator-to-Mutator Examples (All 7 Stages)

Base seed used below:

```text
D0 = [ CreateFile("/input", "seed") ]
```

### Stage 1: ByteFlipFileContent

What it does:

- picks a `CreateFile` or `UpdateFile` op with non-empty content
- flips 1 to 4 random bits in content bytes

Example:

```text
Input  D0: [ CreateFile("/input", "seed") ]
Output D1: [ CreateFile("/input", "seEd") ]
```

(`e` changed because one bit flip can change one ASCII character.)

Skip case:

- if no eligible file-content op exists.

### Stage 2: ReplaceFileContent

What it does:

- picks a `CreateFile` or `UpdateFile` op
- replaces full content with random bytes (length 1..64)
- updates `size` to match new content length

Example:

```text
Input  D0: [ CreateFile("/input", "seed") ]
Output D2: [ CreateFile("/input", "A9\\x01z", size=4) ]
```

Skip case:

- if delta has no `CreateFile`/`UpdateFile`.

### Stage 3: AddFileOp

What it does:

- appends one op: `CreateFile` (70%) or `Mkdir` (30%)
- path is random absolute path
- if `guidance.enoent_paths` exists, path choice is biased toward those

Example:

```text
Input  D0: [ CreateFile("/input", "seed") ]
Output D3: [
  CreateFile("/input", "seed"),
  Mkdir("/tmp/run")
]
```

Or:

```text
[
  CreateFile("/input", "seed"),
  CreateFile("/etc/config", "v=1")
]
```

Skip case:

- when `ops.len() >= MAX_OPS` (`MAX_OPS = 20`).

### Stage 4: RemoveOp

What it does:

- removes one randomly selected operation

Starting exactly from generator seed `D0`, it will skip because there is only
one operation and removing it would produce an invalid empty delta.

Example from seed path:

```text
Input  D0: [ CreateFile("/input", "seed") ]
Output skipped (len <= 1)
```

Example after one growth mutation:

```text
Input  D3: [ CreateFile("/input", "seed"), Mkdir("/tmp/run") ]
Output D4: [ CreateFile("/input", "seed") ]
```

Skip case:

- when `ops.len() <= 1`.

### Stage 5: MutatePath

What it does:

- chooses one op
- changes one path component using internal path vocabulary

Example:

```text
Input  D0: [ CreateFile("/input", "seed") ]
Output D5: [ CreateFile("/output", "seed") ]
```

Path remains absolute.

Skip case:

- empty delta.

### Stage 6: SpliceDelta

What it does:

- picks a donor delta from corpus pool
- appends a random prefix of donor ops
- capped by `MAX_OPS`

In Phase A, donor pool is `initial_corpus_pool()`.

Example (donor starts with `Mkdir("/etc")`, `CreateFile("/etc/config", ...)`):

```text
Input  D0: [ CreateFile("/input", "seed") ]
Output D6: [
  CreateFile("/input", "seed"),
  Mkdir("/etc"),
  CreateFile("/etc/config", "[settings]...")
]
```

Skip case:

- empty donor pool, or no space left due to `MAX_OPS`.

### Stage 7: DestructiveMutator

What it does:

- appends one op chosen from:
  - `DeleteFile(path)`
  - `Rmdir(path)`
  - `Truncate(path, new_size)`
  - `SetTimes(path, mtime, atime)`

Example:

```text
Input  D0: [ CreateFile("/input", "seed") ]
Output D7: [
  CreateFile("/input", "seed"),
  Truncate("/input", 2)
]
```

Or:

```text
[
  CreateFile("/input", "seed"),
  SetTimes("/input", 1710000000, 0, 1710000100, 0)
]
```

Skip case:

- when `ops.len() >= MAX_OPS`.

## 5) End-to-End Mini Walkthrough

One possible iteration:

1. Generator produces `D0 = [CreateFile("/input", "seed")]`.
2. Chosen mutator: `AddFileOp`.
3. Mutated delta:

```text
D = [
  CreateFile("/input", "seed"),
  Mkdir("/tmp")
]
```

4. `apply_delta(vfs, &D)` converts each op to C `delta_add_*` calls.
5. `cp_apply_delta` applies ops and reports per-op success/failure.
6. Harness checks checksum change (semantic yield).
7. Harness resets snapshot to clean baseline for next iteration.

## 6) Why This Design Produces Valid Inputs Reliably

- Seed starts as a minimal, valid non-empty delta.
- Mutator stages either:
  - return `Mutated` with structurally valid output, or
  - return `Skipped` instead of making invalid changes.
- `MAX_OPS` prevents unbounded growth.
- Debug validation in FFI checks invariants before C conversion.
- Snapshot reset guarantees no stale state between iterations.

This combination is exactly what Week 5 Phase A needed: a stable mutation loop
that can be safely extended with guidance and live corpus logic in Phase B.
