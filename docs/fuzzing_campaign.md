# Phase B: LibAFL Integration & Fuzzing Campaign

## What was done

Replaced the hand-rolled dumb loop (`fuzz.rs`) with a real LibAFL `StdFuzzer` backed by
edge-coverage feedback via SanitizerCoverage (`-fsanitize-coverage=trace-pc-guard`).

The integration lives in `mutator/src/bin/fuzz_libafl.rs` and shares all nine mutator
stages already in `mutators.rs`. No architecture changes were needed — LibAFL's
`InProcessExecutor` wraps the existing `FsDelta` harness closure.

---

## Architecture (Phase B — no FUSE mount yet)

```
generate_seed_corpus()
initial_corpus_pool()
        │
        ▼
  InMemoryCorpus<FsDelta>
        │
  StdMutationalStage
  (HavocScheduledMutator over all 9 stages)
        │
        ▼
  harness closure
    primary_content(delta)   ← extract first CreateFile/UpdateFile content bytes
        │
        ▼
  fuzz_foobar(data, len)     ← in-process C target (clang -fsanitize-coverage)
  fuzz_libarchive(data, len)
        │
        ▼
  HitcountsMapObserver (EDGES_MAP, 65536 slots)
  MaxMapFeedback             → promote to corpus on new edges
  CrashFeedback              → write crash to OnDiskCorpus (solutions/)
```

Inputs flow as `FsDelta` → `primary_content()` → raw bytes → C target. No VFS apply,
no FUSE mount. The full delta apply+reset cycle is Phase C.

---

## Campaign targets

### foobar (default demo)

File: `demo/foobar_target.c`

A 6-gate crash target that aborts when `data[0..6] == "foobar"`. Purpose: verify the
LibAFL pipeline end-to-end. Each gate adds a distinct edge so
`HitcountsMapObserver` has something meaningful to track.

```
cargo run --bin fuzz_libafl -- foobar
```

**Observed results (debug build):**
- Seeds loaded: 11 (7 seed families + 4 corpus donors)
- Crash found: execution 57 (well under 100 — corpus donors contain useful byte patterns)
- Edge coverage: 2 → 3 edges during seeding; expands as mutations explore the gate chain
- Solutions written to: `solutions_foobar/`

### libarchive (real campaign)

File: `demo/libarchive_harness.c`

In-process libarchive parser — calls `archive_read_open_memory()` on the delta content
bytes. Supports all archive formats and filters via `archive_read_support_*_all()`.
No filesystem I/O — reads directly from the content extracted by `primary_content()`.

Requires `libarchive-dev`:

```
sudo apt install libarchive-dev
cargo run --bin fuzz_libafl -- libarchive
```

If `libarchive-dev` is absent at build time, the binary still compiles; the libarchive
campaign is disabled with a clear runtime message.

---

## Key implementation details

### LibAFL 0.15 API notes

LibAFL renamed `StdScheduledMutator` to `HavocScheduledMutator` in 0.15. The edge map
size constant is `EDGES_MAP_DEFAULT_SIZE` (not `MAX_EDGES_NUM`). The `add_input` method
requires `use libafl::Evaluator;` in scope.

### SanCov requires clang

GCC does not support `-fsanitize-coverage=trace-pc-guard`. `build.rs` explicitly sets
`.compiler("clang")` for the demo targets so `EDGES_MAP` is actually populated at
runtime. The `__sanitizer_cov_trace_pc_guard*` callbacks are resolved at link time by
`libafl_targets` (feature `sancov_pcguard_edges`) — no separate clang runtime lib needed.

### Seed corpus wiring

`generate_seed_corpus()` + `initial_corpus_pool()` produce the initial `Vec<FsDelta>`.
These are loaded into LibAFL's `InMemoryCorpus` via `fuzzer.add_input()`. The same
vector is also handed to `SpliceDelta` as its `LiveCorpus` for cross-pollination.

### Phase B limitations (by design)

- `primary_content()` extracts only the first file-content op from each delta.
  Metadata-only deltas (truncate, set_times) are skipped (return `ExitKind::Ok`).
- No VFS apply/reset — the target reads from a raw byte slice, not a mounted filesystem.
- Guidance (`MutationGuidance`) is wired but unpopulated — the FUSE log producer that
  fills `write_paths` / `enoent_paths` is Phase C.

---

## Phase C plan (next)

1. Apply `FsDelta` to VFS before each execution: `apply_delta(vfs, input)`.
2. Target reads through FUSE mount (`/mnt/vfs/...`).
3. FUSE write log (one entry per `write()` / `open()` syscall) drains into
   `MutationGuidance` via a new `FuseLogObserver`.
4. `FsAccessFeedback` treats novel `enoent_paths` / `write_paths` as interesting,
   promoting deltas that reach new filesystem paths into the corpus.
5. VFS reset to snapshot after each execution: `vfs_restore_snapshot(vfs)`.

---

## File map

| Path | Role |
|---|---|
| `mutator/src/bin/fuzz_libafl.rs` | LibAFL harness binary |
| `mutator/src/libafl_glue/mod.rs` | `primary_content()` bridge |
| `demo/foobar_target.c` | 6-gate crash demo target |
| `demo/libarchive_harness.c` | In-process libarchive parser |
| `mutator/build.rs` | Compiles C targets with clang + SanCov |
| `corpus_foobar/` | LibAFL active corpus (foobar campaign) |
| `solutions_foobar/` | Crashes written to disk |
