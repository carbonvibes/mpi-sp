#!/usr/bin/env bash
# run_fuzzing.sh — mount the FUSE filesystem then launch afl-fuzz.
#
# Usage:
#   ./run_fuzzing.sh            # start fresh
#   ./run_fuzzing.sh -r         # resume a previous session (-r → afl-fuzz -i-)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MOUNT="$SCRIPT_DIR/mnt"
CORPUS="$SCRIPT_DIR/corpus"
FINDINGS="$SCRIPT_DIR/findings"

RESUME=0
[[ "${1:-}" == "-r" ]] && RESUME=1

# ── pre-flight checks ────────────────────────────────────────────────────────

if ! command -v afl-fuzz &>/dev/null; then
    echo "ERROR: afl-fuzz not found.  Install AFL++ with:"
    echo "  sudo apt install afl++"
    exit 1
fi

if [[ ! -x "$SCRIPT_DIR/fuse_single_file" || ! -x "$SCRIPT_DIR/afl_harness" ]]; then
    echo "ERROR: binaries missing — run 'make' first."
    exit 1
fi

# ── set up directories ───────────────────────────────────────────────────────

mkdir -p "$MOUNT" "$CORPUS" "$FINDINGS"

# Seed corpus: a single byte so AFL++ has something to start from.
if [[ ! -f "$CORPUS/seed1" ]]; then
    printf 'A' > "$CORPUS/seed1"
fi

# ── (re-)mount FUSE ──────────────────────────────────────────────────────────

# Unmount any stale mount before starting.
fusermount3 -u "$MOUNT" 2>/dev/null || true

# -s: single-threaded — avoids races in the simple in-memory buffer.
"$SCRIPT_DIR/fuse_single_file" -s "$MOUNT" &
FUSE_PID=$!

# Wait for the mount to become visible.
for i in $(seq 1 10); do
    sleep 0.1
    [[ -e "$MOUNT/input" ]] && break
    if [[ $i -eq 10 ]]; then
        echo "ERROR: FUSE mount did not appear at $MOUNT/input"
        kill "$FUSE_PID" 2>/dev/null
        exit 1
    fi
done

echo "FUSE mounted at $MOUNT  (pid $FUSE_PID)"

# Unmount when this script exits for any reason.
trap 'echo "Unmounting FUSE..."; fusermount3 -u "$MOUNT" 2>/dev/null || true' EXIT

# ── AFL++ environment tweaks ─────────────────────────────────────────────────

# Required on many systems to allow fuzzing without core dumps redirected.
export AFL_I_DONT_CARE_ABOUT_MISSING_CRASHES=1
export AFL_SKIP_CPUFREQ=1

# ── launch afl-fuzz ──────────────────────────────────────────────────────────

if [[ $RESUME -eq 1 ]]; then
    INPUT_FLAG="-i-"
    echo "Resuming previous AFL++ session in $FINDINGS ..."
else
    INPUT_FLAG="-i $CORPUS"
fi

echo "Starting afl-fuzz ..."
echo "  corpus:   $CORPUS"
echo "  output:   $FINDINGS"
echo "  target:   $SCRIPT_DIR/afl_harness"
echo "  FUSE:     $MOUNT/input"
echo ""

# -t 5000: 5-second timeout per execution — generous for FUSE latency.
# No @@ needed: harness reads from __AFL_FUZZ_TESTCASE_BUF (persistent mode).
afl-fuzz \
    $INPUT_FLAG \
    -o "$FINDINGS" \
    -t 5000 \
    -- "$SCRIPT_DIR/afl_harness"
