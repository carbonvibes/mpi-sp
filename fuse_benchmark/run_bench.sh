set -euo pipefail

FUSE_BIN="./fuse_bench_fs"
BENCH_BIN="./bench_heavy"
FUSE_MOUNT="/tmp/benchmount"
NATIVE_DIR="/tmp/native_bench"

die() { echo "ERROR: $*" >&2; exit 1; }

[[ -x "$FUSE_BIN" ]]  || die "fuse_bench_fs not built — run 'make' first"
[[ -x "$BENCH_BIN" ]] || die "bench_heavy not built — run 'make' first"

umount_fuse() { fusermount3 -u "$FUSE_MOUNT" 2>/dev/null || true; }

umount_fuse
mkdir -p "$FUSE_MOUNT" "$NATIVE_DIR"

echo "  FUSE (in-memory) vs native /tmp"

echo ""
echo "--- FUSE mount ---"
"$FUSE_BIN" "$FUSE_MOUNT"
sleep 2
"$BENCH_BIN" "$FUSE_MOUNT"
umount_fuse
echo ""
echo "--- Native /tmp ---"
"$BENCH_BIN" "$NATIVE_DIR"

echo ""
rmdir -rf "$FUSE_MOUNT"/* "$NATIVE_DIR"/*

