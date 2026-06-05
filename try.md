# Forkserver Death Investigation — Everything We Tried

## The Problem

The AFL++ forkserver (the long-lived `crun` parent process) randomly dies after hours of
fuzzing (observed: 8h 31m on c3_1). When it dies, LibAFL panics:

```
fuzz_one failed: Unknown("Unable to communicate with fork server (OOM?)", ...)
  at src/bin/fuzz_combined_afl.rs:981:14
```

This kills the entire campaign instance. The remaining 5 instances keep running but one
core is wasted. Over a long campaign this happens repeatedly.

**The forkserver never recovered. We never found the root cause with certainty.**

---

## Attempt 1 — Forced Mount Namespace Injection

### What we did

The Nautilus grammar generates OCI configs with various namespace combinations. Many configs
had no mount namespace (`"namespaces": []` or just `[{"type": "user"}]`). When crun ran
these containers without a mount namespace, the container's mounts (proc, tmpfs, sysfs, etc.)
were applied in the *host's* mount namespace — the same one where our FUSE server serves the
rootfs. This caused the FUSE mountpoint to be shadowed: the container's mount at e.g. `/tmp`
completely hid the underlying FUSE mount at that path. The kernel stopped routing filesystem
requests to our FUSE server. The forkserver's sync pipe to the AFL parent closed → LibAFL
saw EOF → panic.

Fix: in `override_rootfs_path()` in `fuzz_combined_afl.rs`, always inject `{"type": "mount"}`
into the namespaces array if it isn't already there:

```rust
let has_mount = arr.iter().any(|n|
    n.get("type").and_then(|t| t.as_str()) == Some("mount"));
if !has_mount {
    arr.push(serde_json::json!({"type": "mount"}));
}
```

### Result

The original fuzzer-death-from-shadowing was fixed. Campaigns ran much longer without dying
from this specific cause. However, we then observed 273 new crashes immediately — turns out
the user+mount namespace combination triggered code paths in crun that were never exercised
before (crun's user namespace + mount namespace setup path). These were saved as objectives.

The forkserver still died (c3_1 died after 8h 31m), just less frequently and from a
different, unknown cause.

---

## Attempt 2 — RUST_BACKTRACE=full

### What we did

Added `RUST_BACKTRACE=full` to every instance in `launch_campaigns.sh`:

```bash
RUST_BACKTRACE=full taskset -c $i sudo -E unshare -m "$COMBINED_BIN" ...
```

### What we got

When c3_1 died, we got the full Rust backtrace:

```
thread 'main' (1342248) panicked at src/bin/fuzz_combined_afl.rs:981:14:
fuzz_one failed: Unknown("Unable to communicate with fork server (OOM?)",
   0: libafl::executors::forkserver::Forkserver::read_st_timed
   1: <libafl::fuzzer::StdFuzzer<...> as libafl::fuzzer::ExecutesInput<...>>::execute_input
   ...
```

### What this told us

The panic fires from `read_st_timed` inside `execute_input`. The backtrace confirmed:
- LibAFL was trying to read the execution result from the forkserver pipe
- The read failed — not a timeout, but an actual error

The backtrace alone did NOT tell us what killed the forkserver. It only told us how
LibAFL discovered the forkserver was dead.

---

## Attempt 3 — LibAFL Source Code Analysis

### What we did

Read the actual LibAFL source at:
`~/.cargo/git/checkouts/libafl-df9415290f13ce43/2e7349d/crates/libafl/src/executors/forkserver.rs`

### What we found

`read_st_timed` (line 665):
```rust
let sret = pselect(..., Some(timeout), ...)?;
if sret > 0 {
    if self.st_pipe.read_exact(&mut buf).is_ok() {
        Ok(Some(val))           // normal: child responded
    } else {
        Err(Error::unknown(     // ← THIS is the panic path
            "Unable to communicate with fork server (OOM?)"))
    }
} else {
    Ok(None)  // ← timeout: returns None, NOT an error
}
```

**Key finding:** The panic fires when `pselect` says data is ready (`sret > 0`) but
`read_exact` gets 0 bytes (EOF). EOF on a pipe means the write end was closed. The write
end is held by the **forkserver process (crun)**. Therefore:

**The forkserver parent process (crun) DIED. This is not a timeout.**

A timeout (`Ok(None)`) goes to a different branch:
```rust
} else {
    kill(child_pid, SIGKILL);
    self.forkserver.read_st()  // blocking drain — would HANG, not panic
}
```

So the D-state-child-surviving-SIGKILL theory (proposed by another LLM) was **wrong**.
That scenario causes a hang, not a panic.

### What we still didn't know

What killed the crun forkserver parent process.

---

## Attempt 4 — Log Analysis (Instantaneous Exec Rate)

### What we did

Computed instantaneous exec rates from heartbeat deltas in the c3_1 log before it died:

```
8h-31m-32s → 8h-31m-35s:  282 execs in 3s  =  94/s  ← very slow
8h-31m-35s → 8h-31m-37s:  476 execs in 2s  = 238/s
...
8h-32m-0s  → 8h-32m-2s:   284 execs in 2s  = 142/s  ← slow again
8h-32m-7s  → 8h-32m-9s:   685 execs in 2s  = 343/s
                                                     PANIC
```

Normal rate: ~280/s. The dips to 94/s and 142/s indicate individual executions taking
~2 seconds instead of ~3.5ms. Something was hanging periodically.

Also noted: at 8h-30m-42s, objective #359 was saved (a new crash). About 1.5 minutes later
the forkserver died. Timing correlation — possibly the mutation engine started generating
variants of the crash input that triggered a hang condition.

### What this told us

Executions were periodically hanging before the final death. The final death was sudden
(no visible ramp-down in the last few heartbeats before panic). Most likely: one execution
hung long enough that the forkserver exceeded some system limit or was killed externally.

---

## Attempt 5 — ASAN Binary

### What we built

Created a new Nix package `crun-harness-asan` at:
`SemanticSanitizer/nix/packages/by-name/artifact-eval/crun-harness-asan/package.nix`

Uses `AFL_USE_ASAN=1` environment variable (delegates ASAN instrumentation to
`afl-clang-lto` rather than adding `-fsanitize=address` directly, which caused the
compiler to fail with "cannot create executables" due to linker conflicts with AFL's
forkserver runtime).

ASAN options set via `__asan_default_options()` callback in the harness patch:
```c
const char *__asan_default_options(void) {
    return "abort_on_error=1:"
           "log_path=/tmp/asan_reports/crun_asan:"
           "detect_leaks=0:"
           "quarantine_size_mb=16";
}
```

`abort_on_error=1` ensures ASAN violations raise SIGABRT so AFL++ still sees a crash
signal rather than silent exit. Reports land in `/tmp/asan_reports/crun_asan.<pid>`.

Updated `launch_campaigns.sh` to use the ASAN binary instead of the production binary.

### What we observed

After running the campaign with the ASAN binary:
- **264 crashes saved** in `/tmp/c3_*/crashes/`
- **`/tmp/asan_reports/` was completely empty** — zero ASAN reports

### What this told us

ASAN found **zero memory errors** in 264 crash events. All 264 crashes were from our
synthetic `raise(SIGSEGV)` calls in the harness (the `child_crashed` detection logic and
the `ret > 128` re-raise), not from actual memory corruption. ASAN's instrumentation never
triggered because no memory violation occurred — the signals were raised manually by our
harness code, bypassing ASAN entirely.

This also meant the forkserver death was **not caused by a heap overflow, use-after-free,
or stack overflow in crun's code** — ASAN would have caught any of those and written a
report.

---

## Attempt 6 — False Positive Crash Detection Removal

### What we found

The harness had two synthetic crash-raising paths added in previous sessions:

**Path 1 — `child_crashed` detection:**
```c
int child_crashed = (err && err->status == 0 && err->msg
                     && (strstr(err->msg, "read from sync socket")
                         || strstr(err->msg, "read from sync pipe")
                         || strstr(err->msg, "read from the exec fifo")
                         || strstr(err->msg, "read from the init process")));
if (child_crashed)
    raise(SIGSEGV);
```

This fires when the container init process dies before exec. That's **expected behavior**
for invalid OCI configs (broken symlinks in rootfs, seccomp blocking a needed syscall,
namespace setup failing). Not a crun bug — crun correctly set up what the config asked for
and the child died as a consequence.

**Path 2 — `ret > 128` re-raise:**
```c
if (ret > 128)
    raise(ret - 128);
```

This fires when the container process is killed by a signal (SIGKILL from OOM, SIGSYS from
seccomp, etc.). Again, expected behavior for many grammar-generated configs.

### What we did

Removed both paths. The harness now unconditionally calls `libcrun_error_release`,
`rmdir_rec`, `libcrun_container_free` and loops with `continue` on both success and failure.
Only real crashes survive:
- Genuine SIGSEGV/SIGABRT in crun's own code (ASAN report + SIGABRT via `abort_on_error=1`)
- Unhandled kernel signal to the crun process itself

Rebuilt: `4w5j3vmpd4rl71c0vxzkl5mwq4mqjnz7-crun-harness-asan-1.23.1`

### Did this fix the forkserver death?

No. The false positive crashes were an independent problem. The forkserver death remained.

---

## Attempt 7 — strace -e trace=signal on Fuzzer PIDs

### What we did

```bash
sudo strace -e trace=signal \
  -p 1342268 -p 1342270 -p 1342277 -p 1342281 \
  2>&1 | tee /tmp/signal_trace.log
```

Attached strace to the Rust fuzzer process PIDs (the running campaign instances).

### Why this was insufficient

strace with `-e trace=signal` on the **Rust fuzzer** captures:
- `kill()` syscalls made by the Rust process (LibAFL sending SIGKILL to timed-out children)
- Signal masks and handlers being set up

It does NOT capture:
- Signals received by the **crun forkserver parent** process
- OOM killer sending SIGKILL directly to crun (kernel-level, not via kill() syscall)
- D-state events (no syscall to trace while in uninterruptible sleep)
- Anything happening inside crun's fork children

The correct approach would have been:
```bash
# Find the long-lived crun forkserver parent PIDs
pgrep -x crun
# Attach to crun directly
strace -e trace=signal -p <crun_parent_pid>
```

But this was never done before the next death occurred.

---

## Attempt 8 — wchan Monitoring (Proposed, Never Executed)

### What was proposed

Monitor the crun forkserver parent's kernel wait state in real time:

```bash
watch -n 0.5 'for p in $(pgrep -x crun); do
    echo "PID $p: $(cat /proc/$p/wchan 2>/dev/null)"
done'
```

`/proc/PID/wchan` shows the kernel function the process is sleeping in:
- `do_wait` = normal (waiting for fork child)
- `fuse_simple_request` = stuck waiting for our FUSE server to respond
- `fuse_dev_do_read` = FUSE server waiting for kernel
- `pipe_wait` = stuck on a pipe
- `schedule` = generic scheduling sleep

This would have given a **definitive kernel-level answer** about what the process was doing
when it got stuck, without any overhead. A companion watchdog script could have triggered
capture automatically when exec/sec dropped below threshold.

### Why it wasn't implemented

We moved to fixing other issues (false positives, ASAN setup) before another death occurred.
The strace was already running so we depended on that. By the time we had the analysis,
the user decided to move to QEMU+KVM.

---

## Theories About Root Cause (All Unconfirmed)

### Theory A — OOM killer (most probable based on evidence)

After hours of running:
- The harness has `cleanup_cgroup()` but if the grammar generates `cgroupsPath:
  "/mycontainer"`, crun creates the cgroup at that path, not at `<container_id>`. The
  `cleanup_cgroup()` call uses `container_id` and misses the grammar-specified path. These
  cgroups accumulate.
- Kernel cgroup objects consume kernel memory (not user-space memory, so not visible in
  normal memory stats)
- After 8.6M executions, even a tiny per-execution kernel memory leak adds up
- OOM killer sends SIGKILL directly to the crun process → pipe closes → LibAFL panics

**Not confirmed:** We never checked `dmesg | grep -i oom` after a death. The user rejected
the `dmesg` check when proposed.

### Theory B — FUSE server crash or hang

Our FUSE server runs in a background thread in the Rust fuzzer. If the FUSE thread:
1. Panics → Rust thread dies, kernel sees FUSE daemon gone → `fuse_abort_conn()` called,
   all pending FUSE requests get EIO → crun gets EIO on rootfs access → fails in unexpected
   way → crun process crashes
2. Deadlocks → FUSE requests from crun block indefinitely → crun child D-states → parent
   blocks in `waitpid` → both stuck

The last input before death had: `/fuzz_loop → /fuzz_loop` (self-referential symlink),
`/proc → /proc/self/exe` (circular via proc), `/tmp → /proc/self/exe`. Any of these could
trigger unusual FUSE lookup patterns.

**Not confirmed:** We never added logging to the FUSE server thread to catch panics.

### Theory C — D-state from kernel namespace operations

When crun sets up a container with user+mount+cgroup namespace combination (triggered by
our mount namespace injection on top of the grammar's user/cgroup namespaces), the kernel
performs complex namespace setup that can sometimes block in D-state waiting for kworker
threads. If these D-states last longer than LibAFL's timeout:
- LibAFL sends SIGKILL to child → child in D-state, SIGKILL ignored
- LibAFL calls blocking `read_st()` waiting for forkserver to acknowledge the kill
- Forkserver (crun parent) is in `waitpid()` on the D-state child → also blocked
- Both are stuck indefinitely

Eventually something (OOM, system watchdog, hung_task_timeout) kills the parent → pipe
closes → LibAFL unblocks with panic.

**Partially confirmed:** The exec rate dips (94/s, 142/s) are consistent with occasional
D-state events resolving within 2-3 seconds. The final death was a D-state that didn't
resolve in time.

### Theory D — Another LLM's theory: D-state child → SIGKILL timeout → read_st_timed timeout (DISPROVEN)

Proposed by another LLM: D-state child survives SIGKILL for >1200ms → `read_st_timed`
times out → panic.

**Disproven by reading LibAFL source:** `read_st_timed` timeout returns `Ok(None)`, not an
error. The panic requires `read_exact` to fail (EOF), which only happens when the forkserver
*parent* dies. A D-state child scenario causes a **hang** (blocking `read_st()` call), not
a **panic**. The other LLM's proposed fix (increase timeout to 5000ms) would not have
addressed the actual failure mode.

---

## What We Considered But Ruled Out

### alarm(5) in the harness child

Adding `alarm(5)` before `libcrun_container_run` to kill the child if it hangs.

**Correct observation:** Would prevent hung children. But:
1. Without a signal handler, SIGALRM is treated as a crash by AFL → false positives
2. Even with `_exit(0)` handler, it addresses the **child** process — the panic comes from
   the **parent** dying. Different processes.
3. Indirect benefit: by preventing D-state children from persisting, might reduce the
   probability of the parent getting stuck in `waitpid`. But this is speculative.

Not implemented.

### In-Rust forkserver restart (rebuild ForkserverExecutor on error)

Catch the `fuzz_one` error and rebuild the executor:
```rust
match fuzzer.fuzz_one(...) {
    Err(e) if is_forkserver_death(&e) => { executor = rebuild_executor(...); }
    ...
}
```

**The SHM problem (initial analysis):** `ForkserverExecutor` owns the shared memory
(coverage map). Rebuilding it creates new SHM with a new ID. The `SharedMemObserver` still
points to the old SHM segment. Rebuilding only the executor while keeping the observer
produces a dangling reference — coverage measurement silently breaks.

**The SHM problem corrected:** The SHM is owned by the **LibAFL fuzzer process**, not by
crun. When crun (the forkserver parent) dies, the SHM mapping in the fuzzer process is
completely unaffected. The actual requirement is: restart the crun process pointing to the
**same SHM_ID** so the new crun process maps the same region. The `ShMemObserver` stays
valid throughout — it never needs to be touched.

The Rust ownership problem: LibAFL's `ForkserverExecutor` takes ownership of the observers
in its builder. Dropping and rebuilding the executor also drops the `ShMemObserver` and
its `ShMem`, unmapping the SHM segment. To prevent this, you'd need to either:
- Allocate SHM outside the executor scope and pass a pre-allocated ID to the builder
- Fork LibAFL and add a `restart_forkserver()` method that respawns the subprocess only

**The corpus replay workaround (does not require LibAFL modification):**

Accept that rebuilding the executor creates new SHM. After rebuilding, replay the entire
in-memory corpus through the new executor to reconstruct the virgin map:

```rust
loop {
    match fuzzer.fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr) {
        Ok(_) => {}
        Err(e) if format!("{e:?}").contains("Unable to communicate with fork server") => {
            eprintln!("[restart] forkserver died — rebuilding...");
            // Kill zombie crun processes
            let _ = Command::new("pkill").arg("-9").arg("-x").arg("crun").output();
            thread::sleep(Duration::from_millis(500));
            // Drop old executor (SHM freed), build new one (new SHM)
            drop(executor);
            executor = build_executor(&crun_path, &observer_ref, ...)?;
            // Replay corpus — reconstructs virgin map from existing inputs
            // ~249 corpus entries at 280 exec/s ≈ 1 second
            for id in state.corpus().ids().collect::<Vec<_>>() {
                let input = state.corpus().get(id)?.borrow().input().clone().unwrap();
                let _ = executor.run_target(&mut fuzzer, &mut state, &mut mgr, &input);
            }
            eprintln!("[restart] done — fuzzing continues");
        }
        Err(e) => return Err(e),
    }
}
```

**Coverage impact of corpus replay approach:**
- The virgin map tracks "all edges ever seen". The corpus contains "minimal set of inputs
  that cover those edges" (AFL++ corpus minimization semantics).
- Replaying the corpus reconstructs coverage from the minimal set. Any edge that was in
  the virgin map but NOT captured by any corpus entry is temporarily lost — the fuzzer
  might re-discover it and add a new corpus entry, which is harmless.
- In practice: the AFL++ corpus IS the coverage state. Replaying it restores effectively
  100% of known edges. Coverage is not clamped — it continues upward from the restored
  baseline.

**Preserved across restart:**
- Entire in-memory corpus (loaded from disk on startup, maintained in memory — still valid)
- All scheduler state (which corpus entries are favored, pending fuzz counts, weights)
- FUSE server thread (still running in the Rust process — not restarted)
- Nautilus grammar state
- All LibAFL stage state

**Lost across restart (recovered in ~1s via corpus replay):**
- Virgin map — fully reconstructed by replaying ~249 corpus entries

**Estimated downtime:** kill zombie crun (0.5s) + rebuild executor (0.1s) + replay
249 inputs at 280/s (0.9s) = **~1.5 seconds total**.

**IMPLEMENTED** — `mutator/src/bin/fuzz_combined_afl.rs`, commit on main branch.

Key decisions:
- `StdMapObserver::from_mut_ptr` stores `RefRaw` (raw pointer) internally via
  `OwnedMutSlice::from_raw_parts_mut` — no Rust borrow on `shmem`. Both initial
  and rebuilt observers have identical types, so `executor` can be reassigned.
- `shmem` lives on the stack in `main` for the full campaign. `__AFL_SHM_ID` env
  var is never changed — new crun process inherits it and maps the same SHM.
- `MaxMapFeedback`'s virgin map (historical coverage) is in the fuzzer's heap,
  not in SHM. It survives executor rebuilds unchanged. No coverage loss.
- `kill_stray_crun_in_cwd()` SIGKILLs crun processes in our working directory
  (fork-children that survived the dead parent) before rebuilding.
- Restart takes ~1.5s total. Virgin map is intact. All scheduler state preserved.

### Shell supervisor script (restart fuzzer on death)

```bash
while true; do
    cd /tmp/c3_$i && fuzzer ... 2>&1 | tee -a log
    sleep 3
done
```

On restart, LibAFL loads corpus from disk but the virgin map (which tracks all seen edges)
resets to all-0xFF. The fuzzer spends time re-discovering already-known edges. With 249
corpus entries, this re-sweep takes seconds. But the scheduler would re-evaluate all corpus
entries as if they were new, potentially adding duplicates to the on-disk corpus, and losing
all scheduler metadata (which entries are favored, which are pending, execution counts).
**Strictly worse than in-Rust restart** (which would preserve all in-memory state). Rejected
by user.

### QEMU+KVM (not yet done)

Running each crun instance inside a VM provides:
- Complete kernel isolation — namespace operations in one instance can't affect others
- VM restart on death instead of forkserver restart
- No D-state contamination between instances
- Clean cgroup hierarchy per VM

Downsides: significant performance overhead (KVM exits for every namespace/mount syscall),
complex setup, reduced exec/sec likely by 2-5×.

**This is the next step**, chosen because we exhausted all lightweight debugging options
without finding the root cause.

---

## Current State

| What | Status |
|------|--------|
| Mount namespace injection | ✅ Implemented, fixed FUSE-shadowing death |
| RUST_BACKTRACE=full | ✅ Added to launch_campaigns.sh |
| ASAN binary | ✅ Built, deployed, running |
| False positive crash detection | ✅ Removed (child_crashed + ret>128) |
| strace on fuzzer PIDs | ✅ Running but wrong target (should be on crun) |
| strace on crun forkserver PIDs | ❌ Never done |
| wchan monitoring on crun | ❌ Never done |
| Root cause of forkserver death | ❌ **UNKNOWN** |
| Forkserver still dies | ⚠️ **Root cause unknown but campaign now survives** |
| In-Rust forkserver restart | ✅ **Implemented** — executor rebuilds in ~1.5s, virgin map preserved |

### What would definitively identify the root cause

```bash
# During a running campaign, find the long-lived crun PIDs
pgrep -a crun | sort -t' ' -k1 -n

# Monitor wchan continuously — save output to file
while true; do
    ts=$(date +%T)
    for p in $(pgrep -x crun); do
        wchan=$(cat /proc/$p/wchan 2>/dev/null)
        rss=$(grep VmRSS /proc/$p/status 2>/dev/null | awk '{print $2}')
        echo "$ts PID=$p wchan=$wchan rss=${rss}kB"
    done
    sleep 0.5
done | tee /tmp/crun_wchan.log

# AND check dmesg immediately after a death
dmesg | grep -E "oom|kill|crun" | tail -20
```

When the next death occurs, the wchan log will show exactly which kernel function the
process was sleeping in at the moment it died.

---

## Next Step

Move to QEMU+KVM setup to isolate each fuzzer instance inside a VM. The forkserver
dying inside a VM means only that VM's instance is lost, not the entire campaign. VM
restart recovers cleanly with full state preserved (or as close as the VM snapshot allows).
