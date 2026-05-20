# Fuzzer Instance Death Investigation

**Date:** 2026-05-19  
**Context:** Campaign 3 (`fuzz_combined_afl`) instances dying mid-run with LibAFL panic: `Unable to communicate with fork server (OOM?)`

---

## Observed Symptoms

- `c3_0` ran for ~6h 30m (255 exec/sec, 188 corpus entries), then died
- LibAFL panic in `Forkserver::read_st_timed` — timed out waiting for child to signal via status pipe
- Earlier: `c3_0` ran 13h, `c3_1` ran 2h, `c1_1` ran 3h before dying
- All instances die stochastically — not reproducible from any specific input

---

## Diagnostic Evidence Collected (2026-05-19)

### PID 1308434 — `fuzz_crun` stuck in D-state since 2026-05-04

```
wchan: request_wait_answer

/proc/1308434/stack:
[<0>] request_wait_answer+0x1be/0x2a0
[<0>] fuse_simple_request+0x18d/0x2f0
[<0>] fuse_dentry_revalidate+0x14f/0x3a0
[<0>] lookup_fast+0x84/0x100
[<0>] open_last_lookups+0x5f/0x400
[<0>] path_openat+0x99/0x2d0
[<0>] do_filp_open+0xaf/0x170
[<0>] do_sys_openat2+0xb3/0xe0
[<0>] __x64_sys_openat+0x55/0xa0
```

`fuzz_crun` called `openat()` on a file inside a FUSE-mounted rootfs. The kernel sent a FUSE request via `fuse_simple_request`. The FUSE server (which lived inside the fuzzer process) had already died. Kernel is now permanently blocked in `request_wait_answer` — D-state, unkillable. Has been stuck 15+ days.

### Current c3_0 survivor processes

```
pid=3969896  state=T (stopped)  wchan=do_signal_stop
pid=3969901  state=T (stopped)  wckan=0
pid=3969902  state=T (stopped)  wchan=do_signal_stop
```

These are SIGSTOP'd (`T`), not D-state. They are leftover from a previous failed restart attempt, not from the crash itself. No D-state crun child was found for c3_0 — the child either exited or was already cleaned up before we could inspect it.

### dmesg

Only noise: repeated `proc: Unknown parameter 'mode'` and `sysfs: Unknown parameter 'mode'`.

These are harmless — crun passes a `mode=` mount option when mounting proc/sysfs inside the container's mount namespace, and the current kernel rejects unrecognised options with a warning. Fires every iteration (~255/sec). No OOM, no panic, no relevant kernel warnings.

---

## Theory: FUSE Orphaned Mount Deadlock

### How `fuzz_combined_afl` (Campaign 3) is structured

```
fuzz_combined_afl process
├── LibAFL main thread   — runs fuzz loop, forks child, waits on status pipe
├── FUSE server thread   — serves rootfs file requests from crun children
└── child: crun          — runs container, accesses FUSE mount during setup/exec
```

### Failure sequence (theory)

1. Something causes the FUSE server thread to crash or permanently block (unknown trigger — could be a panic in the FUSE library, an unhandled error, a lock the main thread holds)
2. Child `crun` is mid-execution, doing `openat()` on the FUSE rootfs
3. Kernel sends FUSE request → nobody answers → child enters D-state in `request_wait_answer`
4. LibAFL main thread is blocked in `waitpid()` for child — child never wakes up
5. `read_st_timed` timeout fires → LibAFL panics: "Unable to communicate with fork server"
6. `fuzz_combined_afl` exits. FUSE mount is **not** unmounted — it stays registered in the kernel as a zombie
7. Any future `openat()` on that mount by any process will block in D-state forever

PID 1308434 (`fuzz_crun`, different older binary) is direct proof that step 6+7 happen: a 15-day-old zombie FUSE mount is actively causing D-state.

### Why this is not confirmed for c3_0 specifically

We caught c3_0 too late — no D-state crun child was surviving when we looked. The child may have exited before LibAFL's timeout, or it died for an unrelated reason (cgroup/mount namespace teardown hang, OOM, etc.) and the FUSE aspect wasn't involved this time.

The FUSE deadlock is a **confirmed risk** in this codebase (proven by pid 1308434), but whether it caused c3_0's specific crash on 2026-05-19 is unconfirmed.

---

## Alternative Hypotheses

| Hypothesis | Evidence For | Evidence Against |
|---|---|---|
| FUSE deadlock (FUSE thread dies, child blocks in openat) | pid 1308434 confirms the exact mechanism; stochastic timing fits | No D-state child observed for c3_0 this time |
| Kernel D-state in cgroup/mount teardown | Campaign 1 (no FUSE) also dies; generic stress from millions of unshare -m iterations | No wchan evidence for current crashes |
| OOM kill | "(OOM?)" hint in LibAFL error message | No dmesg OOM entries |

---

## What to Do Next Time an Instance is Dying

The window is narrow — catch it before LibAFL panics (exec/sec drops to 0 but process still running).

```bash
# Find the stuck crun child PID
FUZZER_PID=$(pgrep fuzz_combined_afl | head -1)
CHILD_PID=$(pgrep -P $FUZZER_PID | head -1)

# Get wchan and kernel stack
cat /proc/$CHILD_PID/wchan
sudo cat /proc/$CHILD_PID/stack
sudo dmesg -T | tail -30
```

**Key interpretation:**
- `request_wait_answer` in wchan → **FUSE deadlock confirmed**
- `cgroup_*` or `mnt_*` → kernel resource exhaustion from container teardown
- `futex_wait` → userspace lock contention
- No child found → child already exited, timeout is from something else

---

## Cleanup Required

Before restarting any c3_* instance:

```bash
# Check for zombie FUSE mounts
cat /proc/mounts | grep fuse
# Unmount any stale ones
fusermount -u <mountpoint>   # or: sudo umount -l <mountpoint>
```

Also kill the SIGSTOP'd survivors from the old restart:
```bash
sudo kill -9 3969896 3969901 3969902
```

---

## Potential Fix

Register a panic hook in `fuzz_combined_afl` that unmounts the FUSE filesystem before the process exits. Without this, every crash leaves a zombie mount that can D-state-lock any process touching it indefinitely.

```rust
// In fuzz_combined_afl main(), after FUSE mount is set up:
let mount_path = fuse_mount_path.clone();
std::panic::set_hook(Box::new(move |_| {
    let _ = std::process::Command::new("fusermount")
        .args(["-u", mount_path.to_str().unwrap_or("")])
        .status();
}));
```

Note: panic hooks don't run on `std::process::exit()` or signals — a `Drop` on a cleanup guard is more robust, but also has limits (SIGKILL bypasses it).
