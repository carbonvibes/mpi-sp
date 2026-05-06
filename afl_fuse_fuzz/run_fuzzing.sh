#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MOUNT="$SCRIPT_DIR/mnt"
CORPUS="$SCRIPT_DIR/corpus"
FINDINGS="$SCRIPT_DIR/findings"

RESUME=0
[[ "${1:-}" == "-r" ]] && RESUME=1

if ! command -v afl-fuzz &>/dev/null; then
    echo "ERROR: afl-fuzz not found.  Install AFL++ with:"
    echo "  sudo apt install afl++"
    exit 1
fi

if [[ ! -x "$SCRIPT_DIR/fuse_single_file" || ! -x "$SCRIPT_DIR/afl_harness" ]]; then
    echo "ERROR: binaries missing — run 'make' first."
    exit 1
fi

mkdir -p "$MOUNT" "$CORPUS" "$FINDINGS"

if [[ ! -f "$CORPUS/seed1" ]]; then
    printf 'A' > "$CORPUS/seed1"
fi

fusermount3 -u "$MOUNT" 2>/dev/null || true

# -s: single-threaded to avoid races in the simple in-memory buffer
"$SCRIPT_DIR/fuse_single_file" -s "$MOUNT" &
FUSE_PID=$!

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

trap 'echo "Unmounting FUSE..."; fusermount3 -u "$MOUNT" 2>/dev/null || true' EXIT

export AFL_I_DONT_CARE_ABOUT_MISSING_CRASHES=1
export AFL_SKIP_CPUFREQ=1

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

# -t 5000: generous timeout for FUSE latency; no @@ needed (persistent mode reads from __AFL_FUZZ_TESTCASE_BUF)
afl-fuzz \
    $INPUT_FLAG \
    -o "$FINDINGS" \
    -t 5000 \
    -- "$SCRIPT_DIR/afl_harness"
