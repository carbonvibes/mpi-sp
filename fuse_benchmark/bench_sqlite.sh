set -euo pipefail

FUSE_BIN="./fuse_bench_fs"
FUSE_MOUNT="/tmp/benchmount"
NATIVE_DIR="/tmp/native_bench"
SQLITE_ROWS=50000

die() { echo "ERROR: $*" >&2; exit 1; }
[[ -x "$FUSE_BIN" ]] || die "fuse_bench_fs not built — run 'make' first"
command -v sqlite3 >/dev/null || die "sqlite3 not found"

umount_fuse() { fusermount3 -u "$FUSE_MOUNT" 2>/dev/null || true; }

mount_fuse() {
    mkdir -p "$FUSE_MOUNT"
    "$FUSE_BIN" -f "$FUSE_MOUNT" &
    FUSE_PID=$!
    # wait until the mountpoint is actually live
    for i in $(seq 1 20); do
        mountpoint -q "$FUSE_MOUNT" && return 0
        sleep 0.1
    done
    die "FUSE mount did not come up after 2s"
}

run_sqlite() {
    local dir="$1"
    local db="$dir/bench.db"
    rm -f "$db" "$db-wal" "$db-shm"

    local start end
    start=$(date +%s%N)

    sqlite3 "$db" "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
                   CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);" > /dev/null

    (
        echo "BEGIN;"
        for i in $(seq 1 $SQLITE_ROWS); do
            echo "INSERT INTO t VALUES ($i, 'payload_${i}_xxxxxxxxxxxxxxxx');"
        done
        echo "COMMIT;"
    ) | sqlite3 "$db" > /dev/null

    sqlite3 "$db" "SELECT COUNT(*), MAX(id) FROM t;" > /dev/null
    sqlite3 "$db" "CREATE INDEX idx ON t(id);"        > /dev/null
    for id in 1000 5000 10000 25000 50000; do
        sqlite3 "$db" "SELECT val FROM t WHERE id=$id;" > /dev/null
    done

    end=$(date +%s%N)
    rm -f "$db" "$db-wal" "$db-shm"
    echo $(( (end - start) / 1000000 ))
}

mkdir -p "$NATIVE_DIR"

echo "================================================================"
echo "  SQLite benchmark: FUSE (in-memory) vs native /tmp"
echo "  Rows: $SQLITE_ROWS  |  WAL mode  |  insert + scan + lookup"
echo "================================================================"

echo ""
echo "  [FUSE]"
umount_fuse
mount_fuse
FUSE_MS=$(run_sqlite "$FUSE_MOUNT")
umount_fuse
echo "  elapsed: ${FUSE_MS} ms"

echo ""
echo "  [Native /tmp]"
NATIVE_MS=$(run_sqlite "$NATIVE_DIR")
echo "  elapsed: ${NATIVE_MS} ms"

echo ""
awk -v f="$FUSE_MS" -v n="$NATIVE_MS" \
    'BEGIN { printf "  overhead: %.2fx  (FUSE %d ms  native %d ms)\n", f/n, f, n }'
echo "================================================================"

umount_fuse
rm -rf "$FUSE_MOUNT" "$NATIVE_DIR"
