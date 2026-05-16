# Handoff Document — crun Fuzzing Campaign
# Full context for next session / new LLM

---

## 1. Who Is Who

- **Arjun** (user, carbonvibes) — running this project, owns this repo at `/home/arjun/mpi-sp/`
- **Moritz Sanft** (msanft on GitHub) — PhD student / collaborator, wrote the original crun harness and Campaign 1 fuzzer, owns the `SemanticSanitizer` submodule
- **Flavio** — the PI (supervisor). Communicates over chat. Has been asking about coverage plateau, cgroup leaks, and the FS vs config distinction.

The `SemanticSanitizer/` directory is Moritz's repo, checked out as a submodule. Arjun does NOT have commit access to it. To build modified versions of it without committing, always use the `path:` Nix prefix (see Rebuild section).

---

## 2. Research Goal

Ablation study on fuzzing crun (container runtime):

| Campaign | What is mutated | What is fixed | Fuzzer |
|---|---|---|---|
| 1 (Moritz baseline) | `config.json` (OCI spec) via Nautilus grammar | rootfs (6 static dirs) | `forkserver_simple` |
| 2 (Arjun's) | rootfs via FUSE VFS + FsDelta | `config.json` (fixed baseline) | `fuzz_rootfs_afl` |
| 3 (not yet built) | both simultaneously | — | `fuzz_combined_afl` (TODO) |

**Hypothesis**: Campaign 3 > Campaign 1 > Campaign 2 in coverage, showing that combining both mutation dimensions is additive.

All three campaigns MUST use the same instrumented crun binary (same `afl-clang-lto` build) so edge counts are directly comparable.

---

## 3. Key File Paths

```
Working directory:
  /home/arjun/mpi-sp/

Moritz's repo (submodule, no commit access):
  /home/arjun/mpi-sp/SemanticSanitizer/

Harness patch (THIS is where all harness changes live):
  /home/arjun/mpi-sp/SemanticSanitizer/case-studies/oci/0001-crun-add-harness.patch

Nix package definition for crun harness:
  /home/arjun/mpi-sp/SemanticSanitizer/nix/packages/by-name/artifact-eval/crun-harness/package.nix

Campaign 1 fuzzer source (Moritz's):
  /home/arjun/mpi-sp/SemanticSanitizer/case-studies/oci/fuzzer/src/main.rs

Campaign 2 fuzzer source (Arjun's):
  /home/arjun/mpi-sp/mutator/src/bin/fuzz_rootfs_afl.rs

Campaign 2 fuzzer compiled binary:
  /home/arjun/mpi-sp/mutator/target/release/fuzz_rootfs_afl

Launch scripts:
  /home/arjun/mpi-sp/launch_campaigns.sh        — 6 instances (C1×3 + C2×3), cores 0-5
  /home/arjun/mpi-sp/launch_c1_parallel.sh      — 3 instances C1 only, cores 0-2

Dashboard:
  /home/arjun/mpi-sp/web_campaign/server.py     — http://localhost:8090
  /home/arjun/mpi-sp/web_campaign/plot_final.py — offline final plot generator

Plan (historical notes, may be partially stale):
  /home/arjun/mpi-sp/plan.md
```

---

## 4. Nix Store Binaries (Critical)

```
Campaign 1 fuzzer (forkserver_simple + grammar):
  /nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1/bin/forkserver_simple
  /nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1/share/grammar.py
  /nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1/share/corpus

crun harness binaries (four rebuilds, use the FOURTH):
  1st — BUGGY (original Moritz patch):
    /nix/store/dl0ncis1aanb8jxk6vj18iqdkgfi5ijj-crun-harness-1.23.1/bin/crun
    → 11200 edges — DO NOT USE
  2nd — bugs 1-7 fixed (create→run, continue, cleanup_cgroup):
    /nix/store/j25zzfyvqvvfp2z988h22rr0i4rknn0v-crun-harness-1.23.1/bin/crun
    → 11072 edges
  3rd — all bugs fixed including umount2 loop (BUG 8):
    /nix/store/wgwpvlvpw94s1k7ir5rkw01v56454mpy-crun-harness-1.23.1/bin/crun
    → 11072 edges
  4th — __AFL_LOOP(10000) → __AFL_LOOP(1) for clean non-persistent exit:
    /nix/store/nbr1grvslmk4qf9pqk0bnfd8y33ldaz7-crun-harness-1.23.1/bin/crun
    ← USE THIS ONE
```

Edge count dropped from 11200 → 11072 between 1st and 2nd build because removing `libcrun_container_create` and `libcrun_container_kill` made their call chains dead code; LTO eliminated them. This is expected and correct.

---

## 5. Rebuild Instructions (Without Git Commit)

Arjun has no commit access to Moritz's repo. To build modified versions:

```bash
cd /home/arjun/mpi-sp/SemanticSanitizer
nix build "path:/home/arjun/mpi-sp/SemanticSanitizer#artifact-eval.crun-harness"
# result symlink: SemanticSanitizer/result -> /nix/store/<hash>-crun-harness-1.23.1
readlink -f result/bin/crun   # get the new store path
```

The `path:` prefix tells Nix to read from the filesystem directly instead of the git tree, so uncommitted changes to the patch file are included.

To rebuild the Campaign 1 fuzzer (forkserver_simple):
```bash
nix build "path:/home/arjun/mpi-sp/SemanticSanitizer#artifact-eval.crun-fuzzer"
```

---

## 6. What the Harness Patch Does

The patch (`0001-crun-add-harness.patch`) modifies `src/crun.c` in crun 1.23.1 to replace `main()` with an AFL fuzzing harness. It adds:

1. `rand_str()` — generates random 19-char container IDs
2. `cleanup_cgroup()` — forcibly removes `/sys/fs/cgroup/<container_id>` after each run
3. `rmdir_rec()` — recursively deletes the `rootfs/` directory after each run
4. New `main()` — AFL fuzzing loop with `__AFL_LOOP(10000)`

The harness loop per iteration:
1. `mkdir("rootfs", ...)` + subdirs (tmp, proc, sys, dev, dev/shm)
2. `init_libcrun_context(...)` with a random container ID
3. `libcrun_container_load_from_file(argv[1], ...)` — reads the fuzz-generated config.json
4. `libcrun_container_run(...)` — runs the container synchronously
5. `cleanup_cgroup(container_id)` — belt-and-suspenders cgroup cleanup
6. `rmdir_rec("rootfs")` — cleans up rootfs including stacked mounts
7. `libcrun_container_free(container)`
8. `continue` on any error (stays in loop)

---

## 7. All Bugs Found and Fixed (Chronological)

### Bugs 1-6: Wrong libcrun API sequence + broken loop

**Moritz's original harness called:** `create → run → kill`

**Bug 1 — `libcrun_container_create(options=0)` secretly runs the container**
- In `container.c:3201`, `create` sets `context->detach = 1` then calls `libcrun_container_run_internal` synchronously
- On SUCCESS: `force_delete_container_status` is NOT called (only called on error path)
- Result: container runs inside `create`, but cgroup + state dir are left behind every iteration
- Verified: 59,912 dead cgroups found in `/sys/fs/cgroup` after one campaign

**Bug 2 — `libcrun_container_run` always fails with "container already exists"**
- `run` calls `libcrun_status_check_directories` (`container.c:3078`) which sees state dir already written by `create` and returns error (`status.c:509`)
- So `run` was dead code — the container was already run inside `create`

**Bug 3 — `libcrun_container_kill` is dead code**
- Never reached because error handling used `return ret` (Bug 6) which exits before kill

**Bug 4 — Cgroup leaked every iteration**
- `create` set `detach=1` so `run_internal`'s success path skips `cleanup_watch` (`container.c:2966`: `if (!context->detach)`)
- `libcrun_cgroup_destroy` only reachable via `container_delete_internal` → `libcrun_container_delete` — never called

**Bug 5 — State dir leaked every iteration**
- Each `create` call writes `~/.local/share/crun/<random_id>/` via `libcrun_status_check_directories` (`status.c:511`)
- Never cleaned up without `libcrun_container_delete`

**Bug 6 — `__AFL_LOOP` persistent mode broken**
- `return ret` inside `__AFL_LOOP(10000)` exits `main()` after 1 iteration
- AFL has to fork a new process for EVERY test case
- Measured: ~40 exec/sec before fix

**Fix for Bugs 1-6:** Remove `create` and `kill` entirely, use only `libcrun_container_run` with `detach=0`. This is the correct API:
- `run` calls `libcrun_status_check_directories` → creates state dir
- Calls `libcrun_container_run_internal` synchronously with `detach=0`
- **Unconditionally** calls `force_delete_container_status` on both success and error (`container.c:3089-3090`)
- → Cgroup destroyed, state dir cleaned up, no leaks

Changed `return ret` to `continue` on all error paths to keep the AFL loop alive.

---

### Bug 7 — Residual 0.67% cgroup leak

Even after switching to `run`, ~0.67% of runs leaked a cgroup. Root cause: `libcrun_cgroup_destroy` errors are swallowed as warnings (`container.c:1855`). When child processes haven't fully exited before the 500-retry rmdir loop completes, the cgroup is not removed. The leaked cgroup names matched our 19-char `rand_str` container IDs, confirming they were ours.

**Fix:** Added `cleanup_cgroup(container_id)` function that:
1. Reads `/sys/fs/cgroup/<id>/cgroup.procs` and SIGKILLs any lingering pids
2. Recursively rmdirs sub-cgroups (leaves first)
3. rmdirs the top-level cgroup directory

Called immediately after `libcrun_container_run` returns, before the error check.

---

### Bug 8 — Stacked OCI bind-mounts accumulate in persistent mode

**Symptom:** exec/sec started at ~158, dropped to ~81 over 18 minutes, continued falling with no floor.

**Root cause:** crun creates OCI-spec bind-mounts inside the fuzzer process's own mount namespace on every iteration — masking `/proc/kcore`, `/proc/latency_stats`, `/proc/sys`, `/proc/bus`, `/proc/sysrq-trigger`, `/proc/irq`, `/sys`. Also mounts proc/tmpfs/sysfs onto `rootfs/proc`, `rootfs/dev`, `rootfs/sys`.

The original `rmdir_rec` called `umount(path)` once per path, which removes only the TOPMOST mount from a stack. After ~18 minutes the process had 13,412 stacked mounts in `/proc/<pid>/mountinfo`.

**Fix:** Changed `umount(path)` to `while (umount2(path, MNT_DETACH) == 0);` loop in `rmdir_rec`. `MNT_DETACH` (lazy unmount) is required because some mounts have child mounts. The loop drains the entire stack before moving on.

---

### The 26% vs 12% Coverage Gap (UNRESOLVED)

Moritz claimed ~26% coverage in his original experiments. Our fixed harness reaches ~12%.

**Leading hypothesis:** Moritz's original `libcrun_container_create` call, despite being "buggy" for cleanup, exercised additional code paths that `run` alone does not:
- Exec FIFO handling (crun creates a FIFO for synchronizing exec in the "created" state)
- "Created" container state machine transitions (distinct from "running" state)
- `libcrun_container_kill` code path (reached in Moritz's harness but dead in ours)
- Possibly ghost coverage from orphaned container child processes that blocked on exec FIFO then got killed

**Key libcrun detail:** `wait_for_process` at `container.c:2103-2104`:
```c
if (args->context->detach && args->notify_socket < 0) return 0;
```
This makes `create` return immediately while the container child is alive but blocked on exec FIFO. The orphaned child eventually gets a SIGKILL but may contribute coverage hits in the meantime.

**Status:** Not yet fixed. No decision made on whether to bring back `create` with explicit `libcrun_container_delete` cleanup.

---

## 8. AFL Persistent Mode Investigation (THIS SESSION)

### The discovery

`forkserver_simple` (Campaign 1) builds `ForkserverExecutor` without calling `.is_persistent(true)`:

```rust
// SemanticSanitizer/case-studies/oci/fuzzer/src/main.rs ~line 210
let mut executor = ForkserverExecutor::builder()
    .program(opt.executable)
    .debug_child(debug_child)
    .shmem_provider(&mut shmem_provider)
    .autotokens(&mut tokens)
    .parse_afl_cmdline(args)
    .coverage_map_size(MAP_SIZE)
    .timeout(Duration::from_millis(opt.timeout))
    .kill_signal(opt.signal)
    // .is_persistent(true) ← MISSING
    .build(tuple_list!(time_observer, edges_observer))
    .unwrap();
```

LibAFL's `ForkserverExecutor` defaults `is_persistent: false` (confirmed at `forkserver.rs:1467`). When `is_persistent: false`, LibAFL does NOT set `__AFL_PERSISTENT` env var (confirmed at `forkserver.rs:486-488`).

### What `__AFL_LOOP(10000)` does without `__AFL_PERSISTENT`

Disassembled `__afl_persistent_loop` in the crun binary (at offset `0x15810`):
- First call: sets `first_pass=1`, `cycle_cnt=10000`, returns 1 ✓
- Second call: decrements `cycle_cnt`, calls `raise(SIGSTOP)` (signal 19), returns 1

`raise(SIGSTOP)` stops the child process. The AFL forkserver in the child notifies the parent. But since `is_persistent: false`, the parent (LibAFL) doesn't handle the stopped child correctly — it doesn't send a new "go" signal. The child stays stopped until the timeout fires, then gets killed. Result: **only 1 iteration per fork regardless of the 10000**.

### PI response

Moritz said: "Definitely not intentional. Back then, I eyeballed the performance and figured it was too fast to only have 1 exec per fork. But if it doesn't, we should definitely fix it."

### The fix (TODO — highest priority for next session)

**Campaign 1** (`forkserver_simple`): Add `.is_persistent(true)` to `ForkserverExecutor::builder()` in `SemanticSanitizer/case-studies/oci/fuzzer/src/main.rs`. Then rebuild the fuzzer Nix package.

**Campaign 2** (`fuzz_rootfs_afl`): Do NOT add persistent mode. Reason: Campaign 2's rootfs mutation is applied by the Rust parent process (applies FsDelta to FUSE VFS) before each fork. In persistent mode, the child runs 10000 iterations but the parent only applies ONE FsDelta before the fork — all 10000 iterations would see the same frozen rootfs state, defeating the purpose of rootfs mutation.

To theoretically support persistent mode in Campaign 2 would require a major redesign (IPC to update FUSE state between child iterations). Not worth it — FUSE latency dominates anyway, not fork overhead.

### After enabling persistent mode — verify mount accumulation

With 10000 iterations per fork, Bug 8 could potentially return for mounts outside `rootfs/`. Our `umount2` loop handles `rootfs/` cleanup. The masking mounts on host `/proc/kcore`, `/proc/sysrq-trigger`, etc. should be cleaned up by crun when the container exits (they're in the mount namespace). Monitor `/proc/<pid>/mountinfo` line count over a short run to confirm it stays flat.

---

## 9. Campaign 2 Architecture (Detailed)

`fuzz_rootfs_afl.rs` design:

- **Executor**: `ForkserverExecutor` (basic, `is_persistent: false`) — one fork per input, matches Campaign 1 structure
- **Target**: same Nix-built crun binary as Campaign 1
- **Config**: Fixed `config.json` written to `/tmp/campaign2/config.json` at startup, never mutated. Points to FUSE mountpoint as rootfs.
- **Rootfs**: FUSE VFS served from `/tmp/campaign2-fuse-<pid>/`. The VFS is in the Rust process. Before each fork, `FsDeltaConverter::to_target_bytes()` resets the VFS to baseline snapshot then applies the FsDelta.
- **Input type**: `FsDelta` — a list of `FsOp` (mkdir, rmdir, create_file, update_file, delete_file, truncate)
- **Mutators**: ByteFlipFileContent, ReplaceFileContent, AddFileOp, RemoveOp, MutatePath, SpliceDelta, DestructiveMutator, UpdateExistingFile, ReplayWriteFile
- **Scheduler**: QueueScheduler
- **Stats**: AflStatsStage writes `fuzzer_stats` + `plot_data` to CWD every 15s

VFS baseline contents:
- `/bin/true` (real binary copied from host)
- `/proc`, `/dev`, `/sys`, `/tmp`, `/etc`, `/var`, `/run` directories
- `/etc/passwd`, `/etc/group`, `/etc/hosts`, `/etc/hostname`, `/etc/resolv.conf`

---

## 10. What Campaign 1 and Campaign 2 Fuzz (Attack Surfaces)

The PI asked about "fs vs config" difference:

- **Campaign 1 (config fuzzing)**: The Nautilus grammar generates valid-ish OCI `config.json` files. The fuzzer mutates the JSON spec (namespaces, capabilities, mounts, UID/GID, seccomp, cgroups, process args). The rootfs is always the same 6 static directories. Finds bugs in crun's **spec parsing and enforcement logic**.

- **Campaign 2 (FS fuzzing)**: The config is fixed (always the same baseline JSON). The FUSE VFS mutates what files/dirs/contents exist inside the container's rootfs. Finds bugs in crun's **rootfs setup, path resolution, symlink handling, chroot/pivot_root, mount setup** logic.

---

## 11. Launch Script Details

### `launch_c1_parallel.sh` (3×Campaign 1, created this session)

```bash
FUZZER=/nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1
CRUN=/nix/store/nbr1grvslmk4qf9pqk0bnfd8y33ldaz7-crun-harness-1.23.1/bin/crun
CAMPAIGN_DIRS=(/tmp/c1_0 /tmp/c1_1 /tmp/c1_2)
CORES=(0 1 2)
```

Each instance: `taskset -c $i sudo unshare -m $FUZZER/bin/forkserver_simple -g $FUZZER/share/grammar.py $CRUN @@`

Ctrl+C handler: kills by CWD lookup in `/proc/<pid>/cwd`, plots with `web_campaign/plot_final.py`, rm -rf dirs.

**IMPORTANT**: After fixing persistent mode and rebuilding forkserver_simple, update `FUZZER=` and `CRUN=` paths in this script to the new Nix store hashes.

### Dashboard compatibility

`web_campaign/server.py` reads from `/tmp/c1_0`, `/tmp/c1_1`, `/tmp/c1_2`, `/tmp/c2_0`, `/tmp/c2_1`, `/tmp/c2_2`. When running only Campaign 1 instances (c1_0, c1_1, c1_2), the C2 cards show as "not running" — that's fine.

---

## 12. Nix Build System Notes

The crun harness is built by `SemanticSanitizer/nix/packages/by-name/artifact-eval/crun-harness/package.nix`. It:
1. Fetches crun 1.23.1 from GitHub
2. Applies `0001-crun-add-harness.patch`
3. Compiles with `afl-clang-lto` via a custom AFL++ stdenv
4. Installs the `crun` binary

The fuzzer (`crun-fuzzer`) is a separate Nix package that builds Moritz's LibAFL-based `forkserver_simple` binary.

When you `nix build "path:/home/arjun/mpi-sp/SemanticSanitizer#..."`, the result symlink appears in whatever directory you ran the command from (not necessarily `SemanticSanitizer/`). Use `find . -name result -type l` if you can't find it.

---

## 13. Observations and Measurements

- Cgroup count after campaigns: 38 → 125 in 98s (87 leaked / 12,884 execs = 0.67%) — this was after 2nd rebuild (bugs 1-7 fixed but before umount2 fix). State dirs: 0.
- Exec/sec: ~158 at start, dropped to ~81 at 18 minutes (Bug 8). After umount2 fix: holds stable.
- Coverage: ~1307/11072 edges (~11.8%) in early campaign runs with fixed binary.
- Coverage with original buggy binary: ~1426/11200 (~12.7%) after 17h.
- Moritz's claimed coverage: ~26% (1.23.1, his original harness, 20h run).

---

## 14. Immediate TODO List (Priority Order)

1. **[HIGHEST] Fix persistent mode in Campaign 1**
   - Add `.is_persistent(true)` to `ForkserverExecutor::builder()` in `SemanticSanitizer/case-studies/oci/fuzzer/src/main.rs`
   - Rebuild: `nix build "path:/home/arjun/mpi-sp/SemanticSanitizer#artifact-eval.crun-fuzzer"`
   - Update FUZZER path in `launch_c1_parallel.sh` and `launch_campaigns.sh`
   - Run short test, verify exec/sec increases significantly and mount count stays flat

2. **[HIGH] Run Campaign 1 for 24h** with fixed persistent mode binary

3. **[HIGH] Run Campaign 2 for 24h** with same crun binary

4. **[MEDIUM] Investigate 26% vs 12% coverage gap**
   - Consider whether to bring back `libcrun_container_create` + `libcrun_container_delete` (with proper cleanup) to match Moritz's original code paths
   - Or accept the gap and document it as a harness correctness fix

5. **[LOW] Write Campaign 3** (`fuzz_combined_afl.rs`) — Nautilus config grammar + FUSE rootfs mutation simultaneously

6. **[LOW] Generate final comparison plots** from 24h runs

---

## 15. Conversation Context / Things Not to Get Wrong

- Do NOT commit changes to `SemanticSanitizer/` — Arjun has no commit access. Always use `path:` Nix prefix.
- The harness patch file is the source of truth for harness code. Do NOT edit crun source directly.
- Do NOT use the 1st build (`dl0ncis1...`) — it has all the original bugs.
- Do NOT use `--rebuild` flag with `nix build` — it fails. Just `nix build` without it.
- `result` symlink from `nix build` appears in CWD, not necessarily in `SemanticSanitizer/`.
- Campaign 2 fuzzer needs to be run as root (`sudo unshare -m`) for FUSE + container namespaces.
- Arjun's email: carbonfibercanhack@gmail.com. PI is Flavio. Collaborator is Moritz.
- The PI conversation tone is casual/direct. Arjun sends brief natural-sounding messages.
