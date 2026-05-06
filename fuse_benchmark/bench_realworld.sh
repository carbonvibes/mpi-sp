#!/usr/bin/env bash
set -euo pipefail

FUSE_BIN="./fuse_bench_fs"
FUSE_MOUNT="/tmp/benchmount"
NATIVE_DIR="/tmp/native_bench"
TARBALL="/tmp/python_stdlib.tar"
SOURCE="/opt/conda/lib/python3.12"

die() { echo "ERROR: $*" >&2; exit 1; }
[[ -x "$FUSE_BIN" ]] || die "fuse_bench_fs not built — run 'make' first"
[[ -d "$SOURCE" ]]   || die "source directory $SOURCE not found"

umount_fuse() { fusermount3 -u "$FUSE_MOUNT" 2>/dev/null || true; }

mount_fuse() {
    mkdir -p "$FUSE_MOUNT"
    "$FUSE_BIN" -f "$FUSE_MOUNT" &
    FUSE_PID=$!
    for i in $(seq 1 20); do
        mountpoint -q "$FUSE_MOUNT" && return 0
        sleep 0.1
    done
    die "FUSE mount did not come up after 2s"
}

ms() {
    local start="$1" end="$2"
    echo $(( (end - start) / 1000000 ))
}

if [[ ! -f "$TARBALL" ]]; then
    echo "  preparing tarball from $SOURCE ..."
    tar cf "$TARBALL" --no-same-permissions --no-same-owner -C "$(dirname "$SOURCE")" \
        "$(basename "$SOURCE")"
    echo "  done ($(du -sh "$TARBALL" | cut -f1))"
fi

FILE_COUNT=$(tar tf "$TARBALL" | wc -l)

echo "================================================================"
echo "  Real-world benchmark: FUSE (in-memory) vs native /tmp"
echo "  Source: Python 3.12 stdlib  |  $FILE_COUNT entries"
echo "================================================================"

run_workload() {
    local dir="$1"
    local extract_dir="$dir/python3.12"

    rm -rf "$extract_dir"

    local t0 t1
    t0=$(date +%s%N)
    tar xf "$TARBALL" --no-same-permissions --no-same-owner -C "$dir"
    t1=$(date +%s%N)
    printf "  tar extract:    %5d ms\n" "$(ms $t0 $t1)"

    t0=$(date +%s%N)
    grep -r "def __init__" "$extract_dir" --include="*.py" -l > /dev/null 2>&1 || true
    t1=$(date +%s%N)
    printf "  grep -r:        %5d ms\n" "$(ms $t0 $t1)"

    t0=$(date +%s%N)
    find "$extract_dir" -type f > /dev/null
    t1=$(date +%s%N)
    printf "  find traversal: %5d ms\n" "$(ms $t0 $t1)"

    rm -rf "$extract_dir"
}

echo ""
echo "  [FUSE]"
umount_fuse
mount_fuse
FUSE_START=$(date +%s%N)
run_workload "$FUSE_MOUNT"
FUSE_END=$(date +%s%N)
printf "  ─────────────────────────\n"
printf "  total:          %5d ms\n" "$(ms $FUSE_START $FUSE_END)"
umount_fuse

echo ""
echo "  [Native /tmp]"
mkdir -p "$NATIVE_DIR"
NATIVE_START=$(date +%s%N)
run_workload "$NATIVE_DIR"
NATIVE_END=$(date +%s%N)
printf "  ─────────────────────────\n"
printf "  total:          %5d ms\n" "$(ms $NATIVE_START $NATIVE_END)"

echo ""
FUSE_TOTAL=$(ms $FUSE_START $FUSE_END)
NATIVE_TOTAL=$(ms $NATIVE_START $NATIVE_END)
awk -v f="$FUSE_TOTAL" -v n="$NATIVE_TOTAL" \
    'BEGIN { printf "  overhead: %.2fx  (FUSE %d ms  native %d ms)\n", f/n, f, n }'

echo "================================================================"

umount_fuse
rm -rf "$NATIVE_DIR" "$FUSE_MOUNT" "$TARBALL"
