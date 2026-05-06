# Dedicated runc/crun Fuzzing Harness

## Background

The specs call for a real-world fuzzing campaign against OCI runtimes using the
FUSE-based rootfs mutation infrastructure. An OCI runtime invocation requires **two
inputs**:

| Input | What it is | Current status |
|-------|-----------|---------------|
| `rootfs/` | Container filesystem layout | ✅ Done — FUSE VFS + FsDelta mutators |
| `config.json` | OCI runtime spec (JSON) | ⚠️ Partially addressed below |

The existing `fuzz_libafl.rs` is a kitchen-sink multi-campaign binary. We'll write
`fuzz_runc.rs` — a clean, focused harness for runc/crun only.

---

## config.json fuzzing — the gap and the plan

> [!IMPORTANT]
> Fully fuzzing an OCI runtime requires mutating **both** `rootfs/` and `config.json`.
> A dedicated JSON-schema-aware mutator doesn't exist yet. Here's what we'll do instead:

### What we CAN do right now (MVP)

**Approach: store `config.json` bytes in the VFS at `/config.json`.**

- The existing `ByteFlipFileContent`, `ReplaceFileContent`, `AddFileOp`, etc. mutators
  already operate on any file in the VFS — they will naturally mutate `/config.json`
  bytes just like any other file.
- On each iteration the harness reads `/config.json` from the FUSE mount, then
  **patches only the `"path"` field under `"root"`** to always point to the real FUSE
  rootfs path. Everything else (process args, capabilities, namespaces, mounts,
  rlimits…) can be arbitrarily corrupted/mutated by the fuzzer.
- The seed corpus includes ~10 structurally diverse valid config.json variants
  covering different OCI spec features (read-only root, extra capabilities, different
  namespace combos, mounts, annotations…).

This gives us:
- **Byte-level JSON mutation** → tests runc's JSON parser robustness (malformed JSON,
  truncated fields, invalid UTF-8, integer overflow in numeric fields…)
- **Structural variation** from diverse seeds → exercises different runc code paths
  (capability handling, mount setup, user-mapping edge cases, seccomp…)

### What we CANNOT do yet (future work)

- **Schema-aware structured mutation** (e.g. LibAFL `JsonMutator` or a custom
  `OciConfigMutator`) — needs a dedicated mutator that understands the OCI spec schema
  and can do semantically valid mutations like swapping namespace types, toggling
  capabilities, changing UID mappings, etc. This is the "proper" config.json fuzzer.
- **Grammar-based mutation** — generate syntactically valid but semantically edge-case
  configs (e.g. duplicate namespace entries, out-of-range capability numbers…).

> [!NOTE]
> For the paper/MVP the byte-level approach is sufficient to demonstrate the system
> works end-to-end. The structured mutator can be added later as a contribution.

---

## VFS Layout

```
VFS (in-memory)
  /config.json          ← OCI spec bytes; mutated by byte ops; harness patches root.path
  /rootfs/              ← container filesystem; mutated by all FsDelta mutators
    bin/true
    etc/passwd
    etc/hosts
    etc/hostname
    etc/resolv.conf
    proc/  dev/  sys/  tmp/  var/  run/

FUSE mount at /tmp/mpi-sp-fuse-<pid>/
  config.json           → VFS /config.json  (harness reads this each iteration)
  rootfs/               → VFS /rootfs/       (runc uses this as root.path)

Real bundle dir /tmp/runc-bundle-<pid>/
  config.json           ← harness writes this each iteration
                           (read from FUSE, root.path patched to FUSE rootfs path)

State dir /tmp/runc-state-<pid>/   (runc container state; per-container-ID)
```

---

## Harness per-iteration flow

```
1. vfs_reset_to_snapshot(vfs)
2. apply_delta(vfs, input)         // mutates /config.json and/or /rootfs/*
3. read  $mountpoint/config.json   // FUSE read of mutated config bytes
4. patch root.path → $mountpoint/rootfs  // always valid rootfs pointer
5. write patched bytes → $bundle_dir/config.json
6. runc/crun run --bundle $bundle_dir <unique-cid>
7. runc/crun delete --force <unique-cid>   // cleanup even on failure
8. classify exit: signal → Crash, else Ok
```

---

## Crash / interesting-result detection

| Condition | LibAFL ExitKind | Action |
|-----------|----------------|--------|
| SIGSEGV / SIGABRT / SIGBUS / SIGFPE / SIGILL | `Crash` | saved to `solutions_runc/` |
| runc panics (stderr contains "panic:" / "runtime error:") | `Crash` | saved |
| Timeout (> 8 s) | `Timeout` | logged, skip |
| Non-zero exit with known error strings | `Ok` | normal fuzzing |
| Exit 0 | `Ok` | baseline |

---

## Seed corpus

### config.json seeds (~10 variants)
1. Minimal (pid + mount + user ns only, no extra mounts)
2. Read-only root
3. Full mount set (proc, devtmpfs, sysfs, mqueue, shm)
4. Elevated capabilities (large bounding set)
5. Empty capabilities
6. Extra rlimits (NOFILE, NPROC, AS)
7. No `noNewPrivileges`
8. Annotations block
9. Empty args list (exercises runc arg validation)
10. Non-existent binary in args (`/bin/nonexistent`)

### rootfs seeds (~15 FsDelta variants — reuse from existing runc_rootfs_seeds)
- Remove proc/dev/sys dirs
- Delete/corrupt /bin/true
- Truncate /bin/true to 0
- Deep nesting
- Traversal-attempt path content
- Timestamp edge cases
- etc.

---

## Proposed Changes

### [NEW] `mutator/src/bin/fuzz_runc.rs`

A self-contained harness with:
- `Target` enum: `Runc` | `Crun` (auto-detect or `--target runc|crun` flag)
- `init_vfs`: populates `/config.json` + `/rootfs/` baseline, saves snapshot
- `config_seeds()`: returns 10 diverse config.json byte-vec seeds as FsDelta UpdateFile ops on `/config.json`
- `rootfs_seeds()`: runc-specific rootfs structure deltas
- FUSE startup + mount-ready wait
- Harness closure: reset → apply → read config from FUSE → patch root.path → write → exec → delete → classify
- Same mutator stack as existing harnesses (ByteFlip, Replace, AddFileOp, RemoveOp, MutatePath, SpliceDelta, DestructiveMutator, UpdateExistingFile, ReplayWriteFile) + I2S stage
- `LiveCorpus` for SpliceDelta to draw from

### [MODIFY] `mutator/Cargo.toml`
- Add `[[bin]]` entry for `fuzz_runc`

---

## Verification Plan

1. `cargo build --bin fuzz_runc --release` — must compile clean
2. Quick smoke: `cargo run --bin fuzz_runc --release -- --target runc --dry-run`
   (we'll add a `--dry-run` flag that runs 5 iterations and exits)
3. Full campaign: `cargo run --bin fuzz_runc --release -- --target runc`
   — confirm runc executes, FUSE is serving, corpus grows, no immediate panics

> [!NOTE]
> `--dry-run` flag: runs exactly N iterations then prints stats and exits cleanly.
> Useful for CI / smoke testing without running forever.
