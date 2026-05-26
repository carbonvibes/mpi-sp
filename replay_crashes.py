#!/usr/bin/env python3
"""
replay_crashes.py — replay saved CombinedInput crash files against the crun harness.

Runs the harness directly (no forkserver protocol).  When fds 198/199 are not
present, afl-compiler-rt's __AFL_LOOP() runs the loop body exactly once
(first call → 1, second call → 0 because is_persistent==0), which is all we need.

Usage:
    sudo python3 replay_crashes.py [crash_file ...]
    # if no args, replays all files in /tmp/c3_1/crashes/

Prints CRASH / NO-CRASH / ERROR for each input.
"""

import json, os, signal, struct, subprocess, sys, shutil, tempfile, time

# ── Paths ─────────────────────────────────────────────────────────────────────
HARNESS   = "/nix/store/arwkshyi5pj6f4j6a6nrvrf89irhgdp4-crun-harness-1.23.1/bin/crun"
GRAMMAR   = "/nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1/share/grammar.py"
DECODE    = "/home/arjun/mpi-sp/mutator/target/release/decode_crash"
CRASHES   = "/tmp/c3_1/crashes"

# ── Rootfs builder ────────────────────────────────────────────────────────────
def create_base_rootfs(rootfs: str):
    """Recreate the in-memory VFS initial state as a real directory tree."""
    shutil.rmtree(rootfs, ignore_errors=True)
    for d in ["bin", "proc", "dev", "sys", "tmp", "etc", "var", "run"]:
        os.makedirs(os.path.join(rootfs, d), exist_ok=True)

    # /bin/true — copy from host
    true_src = shutil.which("true") or "/bin/true"
    if os.path.exists(true_src):
        shutil.copy2(true_src, os.path.join(rootfs, "bin", "true"))

    def wf(path, content):
        full = os.path.join(rootfs, path.lstrip("/"))
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "wb") as f:
            f.write(content)

    wf("/etc/passwd",     b"root:x:0:0:root:/root:/bin/sh\nnobody:x:65534:65534:nobody:/:/usr/sbin/nologin\n")
    wf("/etc/group",      b"root:x:0:\ndaemon:x:1:\nbin:x:2:\nnobody:x:65534:\n")
    wf("/etc/hosts",      b"127.0.0.1 localhost\n::1 localhost\n")
    wf("/etc/hostname",   b"fuzz\n")
    wf("/etc/resolv.conf",b"nameserver 8.8.8.8\n")

def apply_ops(rootfs: str, ops: list):
    """Apply FsDelta ops to the rootfs directory."""
    for op in ops:
        kind = op["kind"]
        path = os.path.join(rootfs, op["path"].lstrip("/"))
        target = op.get("target", "")
        content = bytes(op.get("content", []))

        try:
            if kind == "Mkdir":
                os.makedirs(path, exist_ok=True)

            elif kind in ("CreateFile", "UpdateFile"):
                os.makedirs(os.path.dirname(path), exist_ok=True)
                if os.path.lexists(path):
                    os.remove(path)
                with open(path, "wb") as f:
                    f.write(content)

            elif kind == "CreateSymlink":
                os.makedirs(os.path.dirname(path), exist_ok=True)
                if os.path.lexists(path):
                    os.remove(path)
                os.symlink(target, path)

            elif kind == "DeleteFile":
                if os.path.lexists(path):
                    os.remove(path)

            elif kind == "Rmdir":
                if os.path.islink(path):
                    os.remove(path)
                elif os.path.isdir(path):
                    shutil.rmtree(path, ignore_errors=True)
                elif os.path.exists(path):
                    os.remove(path)

            elif kind == "SetTimes":
                pass  # not needed for crash reproduction

        except Exception:
            pass  # best-effort

# ── Crash decoder ─────────────────────────────────────────────────────────────
def decode_crash(crash_file: str) -> dict:
    result = subprocess.run(
        [DECODE, crash_file, GRAMMAR],
        capture_output=True, timeout=60,
        cwd="/home/arjun/mpi-sp/mutator"
    )
    if result.returncode != 0:
        raise RuntimeError(f"decode_crash failed: {result.stderr.decode()}")
    return json.loads(result.stdout)

# ── Direct harness runner (no forkserver) ─────────────────────────────────────
def run_harness_direct(config_path: str) -> tuple[str, int | None]:
    """
    Run the harness directly without AFL forkserver protocol.

    afl-compiler-rt's __AFL_LOOP() without fds 198/199 connected:
      - first call  → returns 1  (loop body runs once)
      - second call → returns 0  (is_persistent==0, falls through to return 0)

    We wrap in 'unshare -m' for mount namespace isolation (same as the fuzzer).

    IMPORTANT: the harness's loop body does rmdir_rec("rootfs") + mkdir("rootfs",...)
    in its CWD, which would destroy our pre-built rootfs if CWD == workdir.
    We give the harness a *separate* scratch CWD so its internal rootfs creation
    happens in a throw-away directory, while our rootfs (absolute path in config.json)
    is unaffected.
    """
    harness_cwd = tempfile.mkdtemp(prefix="/tmp/hcwd_")
    try:
        result = subprocess.run(
            ["unshare", "-m", HARNESS, config_path],
            cwd=harness_cwd,
            capture_output=True,
            timeout=30,
        )
        rc = result.returncode
        if rc < 0:
            # Killed by signal
            return ("CRASH", -rc)
        else:
            return ("CLEAN", rc)
    except subprocess.TimeoutExpired:
        return ("ERROR: timeout", None)
    except Exception as e:
        return (f"ERROR: {e}", None)
    finally:
        shutil.rmtree(harness_cwd, ignore_errors=True)

# ── Per-crash reproducer ──────────────────────────────────────────────────────
def reproduce(crash_file: str) -> str:
    name = os.path.basename(crash_file)

    # 1. Decode
    try:
        decoded = decode_crash(crash_file)
    except Exception as e:
        return f"[{name}] ERROR decoding: {e}"

    config_json = decoded["config"]
    ops = decoded["rootfs_ops"]["ops"]

    # 2. Build rootfs in a temp dir
    workdir = tempfile.mkdtemp(prefix="/tmp/repro_")
    rootfs  = os.path.join(workdir, "rootfs")
    config_path = os.path.join(workdir, "config.json")
    try:
        create_base_rootfs(rootfs)
        apply_ops(rootfs, ops)

        # 3. Write config.json — fix root.path to our real rootfs
        config_json["root"] = {"path": rootfs, "readonly": False}
        config_json["ociVersion"] = "1.0.2"
        config_json.pop("platform", None)
        with open(config_path, "w") as f:
            json.dump(config_json, f)

        # 4. Run
        result, detail = run_harness_direct(config_path)

        if result == "CRASH":
            try:
                sig_name = signal.Signals(detail).name
            except Exception:
                sig_name = str(detail)
            return f"[{name}] ✓ CRASH confirmed — killed by SIG{sig_name} ({detail})"
        elif result == "CLEAN":
            return f"[{name}] ✗ No crash — exited cleanly (code {detail})"
        else:
            return f"[{name}] ? {result}"

    finally:
        shutil.rmtree(workdir, ignore_errors=True)


# ── Main ──────────────────────────────────────────────────────────────────────
if __name__ == "__main__":
    if os.geteuid() != 0:
        print("ERROR: must run as root (sudo python3 replay_crashes.py)", file=sys.stderr)
        sys.exit(1)

    crash_files = sys.argv[1:] if len(sys.argv) > 1 else sorted(
        os.path.join(CRASHES, f) for f in os.listdir(CRASHES)
        if not f.startswith(".")
    )

    if not crash_files:
        print("No crash files found.", file=sys.stderr)
        sys.exit(1)

    print(f"Replaying {len(crash_files)} crash(es)...\n")
    for cf in crash_files:
        result = reproduce(cf)
        print(result)
        sys.stdout.flush()
