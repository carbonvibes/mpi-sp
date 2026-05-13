# Fuzzing Campaign Plan

## Research Goal

Demonstrate that combining grammar-guided config mutation (Moritz's approach)
with filesystem/rootfs mutation (our FUSE VFS approach) achieves greater
coverage than either approach alone.

---

## Three Campaigns (Ablation Study)

All three campaigns must use **identical instrumentation** (`afl-clang-lto`)
so that edge counts are directly comparable.

### Campaign 1 — Config-only (Moritz's baseline)

`sudo unshare -m /nix/var/nix/profiles/default/bin/nix run .#artifact-eval.crun-campaign`

**What it tests:** Grammar-guided config mutation, fixed rootfs.

**Implementation:** Moritz's existing setup, run as-is.

- Instrumentation: `afl-clang-lto` (via AFL++ forkserver)
- Config mutation: Nautilus grammar (`grammar.py`) — always valid OCI JSON
- Rootfs: Hardcoded 6 dirs (rootfs/, tmp, proc, sys, dev, dev/shm) recreated
  each iteration inside `__AFL_LOOP(10000)`
- Executor: LibAFL `ForkserverExecutor`
- Fuzzer binary: `SemanticSanitizer/case-studies/oci/fuzzer/src/main.rs`
- crun version: 1.23.1 patched with `0001-crun-add-harness.patch`
- Build: `CC=afl-clang-lto ./configure && make`
- Known result: ~12% (1361/11200 edges) after warm-up

**Nothing to build** — run Moritz's campaign and record `fuzzer_stats` /
`plot_data` over 24 hours.

---

### Campaign 2 — Rootfs-only (our baseline)

**What it tests:** FUSE rootfs mutation, fixed config.

**Implementation:** New fuzzer binary `fuzz_rootfs_afl` using ForkserverExecutor
with FUSE-backed VFS mutations but a static baseline config.json.

- Instrumentation: `afl-clang-lto` (exact same Nix-built crun binary as Campaign 1 → 11200 edges)
- Config mutation: Fixed baseline config.json — not mutated, just passed
  as-is every iteration so the only variable is the rootfs state
- Rootfs: FUSE-backed VFS with full mutation suite (our existing mutators:
  ByteFlipFileContent, ReplaceFileContent, AddFileOp, RemoveOp, MutatePath,
  SpliceDelta, DestructiveMutator, UpdateExistingFile, ReplayWriteFile)
- Executor: LibAFL `ForkserverExecutor`
- FUSE: fuzzer process holds the VFS, applies FsDelta before each fork-server
  execution, child sees the mutated rootfs through the FUSE mount (kernel fs,
  accessible to all processes)
- Fuzzer binary: new `src/bin/fuzz_rootfs_afl.rs`

**Changes needed:**
1. Recompile crun with `afl-clang-lto` (same binary as Campaign 1)
2. Write `fuzz_rootfs_afl.rs`:
   - Switch from `InProcessExecutor` to `ForkserverExecutor`
   - Set up AFL shared memory instead of `EDGES_MAP`
   - Keep FUSE VFS startup and mutation loop from `fuzz_crun.rs`
   - Before each `fuzz_one`: apply FsDelta to VFS (rootfs mutated)
   - Pass fixed config.json path as argument to crun fork-server
   - Add `AflStatsStage` for `fuzzer_stats` / `plot_data` output

---

### Campaign 3 — Combined (grammar config + FUSE rootfs)

**What it tests:** Both dimensions mutated simultaneously.

**Implementation:** Moritz's Nautilus grammar for config + our FUSE VFS
for rootfs, running together in one fuzzer.

- Instrumentation: `afl-clang-lto` (exact same Nix-built crun binary → 11200 edges)
- Config mutation: Nautilus grammar — generates valid OCI config.json each
  iteration, written to the FUSE VFS at `/config.json`
- Rootfs mutation: FUSE VFS FsDelta mutations applied before each execution
- Executor: LibAFL `ForkserverExecutor`
- Fuzzer binary: new `src/bin/fuzz_combined_afl.rs`

**Changes needed:**
1. Same afl-clang-lto crun binary
2. Write `fuzz_combined_afl.rs`:
   - Nautilus grammar context loaded from `grammar.py`
   - `NautilusInput` drives config.json content — written into FUSE VFS
     at `/config.json` before each execution
   - `FsDelta` drives rootfs state — applied to FUSE VFS before each execution
   - `ForkserverExecutor` reads config from FUSE mount path
   - Combined input type: `(NautilusInput, FsDelta)` — both mutated each round
   - Mutators: `NautilusRandomMutator` + `NautilusRecursionMutator` +
     `NautilusSpliceMutator` for config; full FsDelta mutator suite for rootfs
   - Feedback: `MaxMapFeedback` (AFL shmem) + `NautilusFeedback` +
     `TimeFeedback`
   - Scheduler: `IndexesLenTimeMinimizerScheduler` (Moritz's choice, better
     than QueueScheduler)
   - `AflStatsStage` for comparable stats output

---

## crun Binary

> **All three campaigns use the exact same Nix-built crun binary from SemanticSanitizer.**

```
/nix/store/wgwpvlvpw94s1k7ir5rkw01v56454mpy-crun-harness-1.23.1/bin/crun
```

This is crun 1.23.1 patched with `0001-crun-add-harness.patch` (3rd rebuild — all bugs fixed),
compiled with `afl-clang-lto` via Nix. Using the same binary across all campaigns guarantees:
- Identical instrumentation
- Identical total edge count: **11072 edges** for all three campaigns
- Results are directly comparable

Do NOT use the manually built `crun-afl/crun` — it produces 19776 edges (different
systemd linkage) making cross-campaign comparison invalid.

---

## Running the Web Dashboard

The dashboard reads live stats from the campaign working directories.
Run it in a separate terminal before or during any campaign.

```bash
# Single-campaign live view (Campaign tab)
# The log path must match the tee destination used when launching the campaign
python3 /home/arjun/mpi-sp/fuzz_dashboard/server.py /tmp/campaign1_fuzz.log campaign1
# For Campaign 2: python3 ... /tmp/campaign2_fuzz.log campaign2
# For Campaign 3: python3 ... /tmp/campaign3_fuzz.log campaign3

# Then open: http://localhost:8080
# - "Campaign" tab  → live coverage chart + log viewer for the active campaign
# - "Comparison" tab → coverage/exec/corpus plots for all 3 campaigns (auto-refresh 60s)
#   reads /tmp/campaign1/fuzzer_stats, /tmp/campaign2/fuzzer_stats, /tmp/campaign3/fuzzer_stats
```

---

## Campaign 1 — Run Commands

### What to build first
```bash
cd /home/arjun/mpi-sp/SemanticSanitizer/case-studies/oci/fuzzer
cargo build --release
# binary: target/release/forkserver_simple
```

### Directory setup
```bash
mkdir -p /tmp/campaign1/corpus

# Seed corpus: Moritz's single baseline config.json
cp /home/arjun/mpi-sp/SemanticSanitizer/case-studies/oci/corpus \
   /tmp/campaign1/corpus/1
```

### Run (needs root — crun creates namespaces/mounts)
```bash
cd /tmp/campaign1   # fuzzer_stats and plot_data are written relative to CWD

CRUN=/nix/store/wgwpvlvpw94s1k7ir5rkw01v56454mpy-crun-harness-1.23.1/bin/crun
FUZZER=/nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1

sudo unshare -m $FUZZER/bin/forkserver_simple \
  -g $FUZZER/share/grammar.py \
  $CRUN @@ \
  2>&1 | tee /tmp/campaign1_fuzz.log
```

The `tee` writes stdout+stderr to `/tmp/campaign1_fuzz.log` so the dashboard
"Campaign" tab can parse LibAFL heartbeat lines from it in real time.

**What `@@` does:** LibAFL's ForkserverExecutor replaces `@@` with a temp file
path containing the grammar-generated config.json for each execution.
The harness reads it via `argv[1]`.

### Files created during run
```
/tmp/campaign1/
  fuzzer_stats    # updated every 15s by AflStatsStage — dashboard reads this
  plot_data       # time-series CSV — dashboard + offline plotter read this
  crashes/        # created automatically if crashes are found
  corpus/         # seed corpus (you created this before running)
```

### Stop
`Ctrl+C` — LibAFL handles it cleanly. Stats are flushed before exit.

---

## Run Duration and Comparison

Run all three campaigns for the **same wall-clock time** (24 hours) on the
**same machine** sequentially (not in parallel — shared CPU affects throughput).

Record from `fuzzer_stats` every 15 seconds (LibAFL's `AflStatsStage` interval):
- `edges_found` — cumulative edges covered
- `execs_done` — total executions
- `exec_per_sec` — throughput
- `corpus_count` — corpus size
- `run_time` — elapsed seconds

---

## Dashboard Changes

The PI requested proper coverage-over-time plots, not just final numbers.

### New plot: Coverage vs Time (all 3 campaigns)

- X axis: wall-clock time (hours, 0–24)
- Y axis: edges covered (absolute count)
- One line per campaign (Campaign 1, 2, 3), different colors
- Rendered from `fuzzer_stats` `plot_data` files written by `AflStatsStage`
- Tool: `afl-plot` (AFL++ built-in) generates gnuplot PNGs from `plot_data`
  OR custom Python (`matplotlib`) reading the same file format for web embed

### New plot: Exec/sec vs Time

- Shows throughput comparison between approaches
- Same X axis (time), Y axis exec/sec from `fuzzer_stats`

### New plot: Corpus size vs Time

- Shows corpus growth rate — indicates when fuzzing stalls
- From `corpus_count` field in `fuzzer_stats`

### Dashboard implementation

Extend `fuzz_dashboard/server.py`:
- New endpoint `/api/campaign/{id}/plot_data` — reads `plot_data` file from
  each campaign's working directory, returns parsed JSON
- New endpoint `/api/compare` — returns all three campaigns' time-series data
  in one response for the comparison chart

Extend `fuzz_dashboard/index.html`:
- Add comparison tab with three charts (coverage, exec/sec, corpus size)
- Use Chart.js (already likely available or CDN) for line charts
- Auto-refresh every 60s while campaigns are running
- Final summary table: Campaign | Duration | Max edges | Max coverage % | Avg exec/sec

### Directory layout for dashboard to read

```
/tmp/campaign1/   fuzzer_stats   plot_data   crashes/
/tmp/campaign2/   fuzzer_stats   plot_data   crashes/
/tmp/campaign3/   fuzzer_stats   plot_data   crashes/
```

Each fuzzer writes its stats to its own working directory, dashboard reads all three.

---

## CPU Pinning Decision

**Do NOT pin to a single CPU.**

All three campaigns are run without explicit CPU affinity. Reasons:
- Measured on this machine: `taskset -c 7` gave **35 exec/sec**, unpinned gave **67 exec/sec** — pinning hurts here because disabling HT removes sibling-thread parallelism that crun's fork/exec relies on.
- Pinning benefits assume idle dedicated cores (AMD EPYC in the paper). This machine is different.
- Consistency: all three campaigns run under identical conditions (no pinning, sequential, same machine). Coverage is what matters, not raw exec/sec.
- `sudo unshare -m` already provides mount namespace isolation without CPU constraints.

If you ever want to pin, use `taskset -c <core>` before the `sudo unshare -m` command. But do it for ALL campaigns or none.

---

## Campaign 2 — Run Commands

### What was built
```
mutator/src/bin/fuzz_rootfs_afl.rs   — Campaign 2 fuzzer source
mutator/target/release/fuzz_rootfs_afl — compiled binary (ready)
```

Key design decisions locked in:
- **Executor**: `ForkserverExecutor` — basic forkserver (no `is_persistent`), one fork per input. Matches Campaign 1 exactly.
- **Crun binary**: same Nix-built binary as Campaign 1 (11200 total edges, directly comparable).
- **Config**: fixed `config.json` written to `/tmp/campaign2/config.json` at startup; points to FUSE mountpoint as rootfs.
- **Rootfs**: FUSE VFS served from `/tmp/campaign2-fuse-<pid>/`; mutated via `FsDelta` before each execution.
- **Scheduler**: `QueueScheduler` (same as Campaign 1).
- **Stage**: `StdMutationalStage` with `HavocScheduledMutator` over the full FsDelta mutator suite.
- **Stats**: `AflStatsStage` writes `fuzzer_stats` + `plot_data` to CWD every 15s.

### Directory setup (once)
```bash
mkdir -p /tmp/campaign2/corpus /tmp/campaign2/crashes
```

### Run (as root, from /tmp/campaign2/)
```bash
cd /tmp/campaign2

CRUN=/nix/store/wgwpvlvpw94s1k7ir5rkw01v56454mpy-crun-harness-1.23.1/bin/crun

sudo unshare -m /home/arjun/mpi-sp/mutator/target/release/fuzz_rootfs_afl \
  $CRUN \
  2>&1 | tee /tmp/campaign2_fuzz.log
```

This is identical in structure to Campaign 1:
- `sudo unshare -m` → isolated mount namespace, no leaked mounts after Ctrl-C
- Same Nix crun binary → identical 11072 total edges
- `tee` → stdout goes to log file for dashboard live view

### Stop
`Ctrl-C` — LibAFL flushes stats before exit. The FUSE mountpoint is cleaned up automatically when the process exits.

### Files created during run
```
/tmp/campaign2/
  config.json      # fixed baseline config (written at startup, never changes)
  fuzzer_stats     # updated every 15s — dashboard reads this
  plot_data        # time-series CSV — same format as Campaign 1
  crashes/         # crashing FsDelta inputs
  corpus/          # interesting FsDelta inputs (coverage-increasing)
/tmp/campaign2-fuse-<pid>/   # FUSE mount (auto-removed on exit)
```

### Dashboard
```bash
# In a separate terminal:
python3 /home/arjun/mpi-sp/fuzz_dashboard/server.py /tmp/campaign2_fuzz.log campaign2
# Open http://localhost:8080
```

---

## Ordering

- [x] Nix-built crun binary confirmed at `/nix/store/dl0ncis1aanb8jxk6vj18iqdkgfi5ijj-crun-harness-1.23.1/bin/crun` (11200 edges — OLD, pre-fix)
- [x] **Harness bugs found and fixed** — see Harness Bug Analysis section below
- [x] 2nd rebuild at `/nix/store/j25zzfyvqvvfp2z988h22rr0i4rknn0v-crun-harness-1.23.1/bin/crun` (11072 edges — fixed cgroup/state-dir leaks + persistent mode)
- [x] **Mount accumulation bug found and fixed** (exec/sec dropped from 158→81 over 18 min due to stacked OCI bind-mounts) — `umount(path)` → `while (umount2(path, MNT_DETACH) == 0);` in `rmdir_rec`
- [x] 3rd rebuild at `/nix/store/wgwpvlvpw94s1k7ir5rkw01v56454mpy-crun-harness-1.23.1/bin/crun` (11072 edges — **use this**)
- [x] Campaign 1 fuzzer available at `/nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1/bin/forkserver_simple`
- [x] Dashboard: Comparison tab with 3-campaign charts → `fuzz_dashboard/`
- [x] Write `mutator/src/bin/fuzz_rootfs_afl.rs` (Campaign 2 fuzzer) — compiles clean, logic verified
- [x] **Run Campaign 1** (~17h) — plot_data saved at `/tmp/semsan_plot_data` (3095 rows, 732k execs, 1426/11200 edges, 12.7% coverage) — ran with OLD buggy binary
- [ ] **Re-run Campaign 1** (24h) with fixed binary — `CRUN=/nix/store/wgwpvlvpw94s1k7ir5rkw01v56454mpy-crun-harness-1.23.1/bin/crun`
- [ ] **Run Campaign 2** (24h) with fixed binary
- [ ] Write `mutator/src/bin/fuzz_combined_afl.rs` (Campaign 3 fuzzer)
- [ ] Run Campaign 3 (24h)
- [ ] Generate offline comparison plots (`plot_compare.py` with matplotlib)

---

## Expected Hypothesis

```
Campaign 1 (config-only grammar):  ~12%  — grammar reaches deep config paths
Campaign 2 (rootfs-only, naive):   ~8%   — rootfs variation, shallow config
Campaign 3 (combined):             >15%  — both dimensions, additive coverage
```

The delta Campaign3 - Campaign1 quantifies the contribution of rootfs mutation.
The delta Campaign3 - Campaign2 quantifies the contribution of grammar config.

---

## Harness Bug Analysis and Fix

**Date discovered:** 2026-05-12  
**Symptom:** 59,912 dead cgroups found in `/sys/fs/cgroup` after a single campaign. Fuzzer plateaued at ~12% coverage. Flavio (PI) flagged the cgroup leak as likely cause.

### File changed

**`SemanticSanitizer/case-studies/oci/0001-crun-add-harness.patch`**  
This is the patch applied to `src/crun.c` during the Nix build of the crun harness binary.

### Bugs found (all in the original patch by Moritz)

#### Bug 1 — Wrong API sequence: `create` silently runs the container then leaks
`libcrun_container_create(options=0)` sets `context->detach = 1` then calls
`libcrun_container_run_internal` synchronously (`container.c:3201`). The container
runs, but on **success** `force_delete_container_status` is NOT called (only on error).
Result: state dir and cgroup both left behind after every successful run.

#### Bug 2 — `libcrun_container_run` always fails with "container already exists"
After `create` writes the state dir, `libcrun_container_run` calls
`libcrun_status_check_directories` (`container.c:3078`) which sees the existing state dir
and returns error (`status.c:509`). So `run` is dead code — the container was already
run inside `create`.

#### Bug 3 — `libcrun_container_kill` is dead code
The harness calls `return ret` on `run` failure, exiting `main()` before reaching `kill`.

#### Bug 4 — Cgroup leaked every iteration
`create` used `detach=1` so `run_internal`'s success path (`container.c:2966`) skips
`cleanup_watch`. The only cgroup cleanup path is `container_delete_internal` reached via
`libcrun_container_delete` — which was never called. One dead cgroup per AFL iteration.

#### Bug 5 — State dir leaked every iteration
Each call to `create` creates `~/.local/share/crun/<random_id>/` via
`libcrun_status_check_directories` (`status.c:511`). Never cleaned up without
`libcrun_container_delete`.

#### Bug 6 — `__AFL_LOOP` persistent mode broken
`return ret` inside `__AFL_LOOP(10000)` exits `main()` after exactly 1 container
per AFL fork. Persistent mode's 10,000-iteration benefit is completely lost.
Measured: ~40 exec/sec (one fork per exec). After fix: ~130 exec/sec.

#### Bug 7 — Residual 0.67% cgroup leak via failed `force_delete_container_status`
Even after switching to `libcrun_container_run` only, ~0.67% of runs left a cgroup.
Root cause: `libcrun_cgroup_destroy` error is swallowed as a warning (`container.c:1855`);
when sub-processes haven't fully exited before the 500-retry rmdir loop completes, the
cgroup is not removed. Cgroup names matched our 19-char `rand_str` container IDs,
confirming these were our leaked cgroups.

#### Bug 9 — `__AFL_LOOP(10000)` causes each config to be run 10,000 times instead of 1 time
With `continue` (Bug 6 fix) and `__AFL_LOOP(10000)`, AFL forks once and runs the SAME config.json
10,000 times before forking again. At 157 exec/sec this means only ~15 unique grammar-generated
configs explored per 16 minutes, vs ~38,000 with the original (broken) harness where `return ret`
effectively made it `__AFL_LOOP(1)`. Root cause confirmed via `wait_for_process` line 2103:
`if (args->context->detach && args->notify_socket < 0) return 0;` — Moritz's `create`-based
harness returned immediately (one config per fork, 40 exec/sec, 26% coverage in 20h). Our
persistent mode ran each config 10,000 times (~15 unique configs in 16 min, stuck at 12%).
Fix: `__AFL_LOOP(10000)` → `__AFL_LOOP(1)`. The forkserver still avoids full process startup
per test case; exec/sec drops slightly but unique configs per second increases 10,000×.

#### Bug 8 — Stacked OCI bind-mounts accumulate in persistent mode, exec/sec drops linearly
crun creates OCI-spec bind-mounts in the harness process's own mount namespace during container
setup (masking `/proc/kcore`, `/proc/latency_stats`, `/proc/sys`, `/proc/bus`,
`/proc/sysrq-trigger`, `/proc/irq`, `/sys`). In AFL persistent mode the same process runs
10,000 iterations, so these mounts stack up: after ~18 minutes the process had 13,412 stacked
mounts (`/proc/<pid>/mountinfo`) causing exec/sec to drop from 158 → 81 and continuing to fall.

The original `rmdir_rec` called `umount(path)` once per path — this removes only the **topmost**
mount from a stack of thousands. Fix: `while (umount2(path, MNT_DETACH) == 0);` which loops
until all stacked layers are detached. `MNT_DETACH` (lazy unmount) is required because some
mounts have child mounts.

### Changes made to the patch

| What | Before | After |
|---|---|---|
| `libcrun_container_create` | called | **removed** |
| `libcrun_container_run` | fails every time ("already exists") | **works correctly, cleans up via `force_delete_container_status`** |
| `libcrun_container_kill` | dead code | **removed** |
| Error handling in loop | `return ret` (exits process) | `continue` (loop stays alive) |
| `container_load_from_file` failure | `libcrun_fail_with_error` (aborts) | `continue` |
| `cleanup_cgroup()` function | absent | **added** — belt-and-suspenders direct rmdir of `/sys/fs/cgroup/<container_id>` after each run |
| `umount(path)` in `rmdir_rec` | single call — removes only top layer | `while (umount2(path, MNT_DETACH) == 0);` — drains all stacked layers |
| `__AFL_LOOP(10000)` | 10,000 runs of same config per fork | `__AFL_LOOP(1)` — 1 run per fork, 10,000× more unique configs explored |

### Edge count change: 11200 → 11072

The edge count is determined by `afl-clang-lto` instrumentation of the entire compiled
binary (all of `crun.c` + libcrun). Removing `libcrun_container_create` and
`libcrun_container_kill` made their call chains dead code; LTO eliminated them, removing
128 edges. The old 11200 included edges from code that was never reachable in practice
(since `run` always failed). All three campaigns use the **same fixed binary**
(`j25zzfyvqvvfp2z988h22rr0i4rknn0v`), so coverage percentages remain directly comparable.

### Rebuilt binaries

```
1st (buggy):  /nix/store/dl0ncis1aanb8jxk6vj18iqdkgfi5ijj-crun-harness-1.23.1/bin/crun  (11200 edges)
2nd (bugs 1-7 fixed): /nix/store/j25zzfyvqvvfp2z988h22rr0i4rknn0v-crun-harness-1.23.1/bin/crun  (11072 edges)
3rd (bug 8 fixed):    /nix/store/wgwpvlvpw94s1k7ir5rkw01v56454mpy-crun-harness-1.23.1/bin/crun  (11072 edges) ← USE THIS
```

Rebuild command (from `SemanticSanitizer/` — use `path:` prefix to avoid needing a git commit):
```bash
sudo unshare -m /nix/var/nix/profiles/default/bin/nix build \
  "path:/home/arjun/mpi-sp/SemanticSanitizer#artifact-eval.crun-harness"
```

### Verification

After 2nd rebuild (bugs 1-7), one Campaign 1 instance at 130.9 exec/sec for 98s:
- Cgroup count: 38 → 125 (87 leaked / 12,884 executions = 0.67% residual from bug 7)
- State dirs: 0 (confirmed cleanup working for 99.3% of runs)

After 3rd rebuild (bug 8), exec/sec should hold stable at ~130-158 rather than dropping
linearly over time. Mount count in `/proc/<pid>/mountinfo` should stay near baseline (~30)
instead of climbing to thousands.
