#!/usr/bin/env bash
#
# test_mount.sh — integration test for the VFS-backed FUSE mount.
#
# Mounts fuse_vfs at a temporary directory, runs read/stat/ls checks, then
# unmounts cleanly.  Exit code 0 = all pass, 1 = any failure.
#
# Usage:  bash test_mount.sh          (or: make test)

set -euo pipefail

MOUNT=/tmp/fuse_vfs_test
BINARY=./fuse_vfs

PASS=0
FAIL=0

check() {
    local desc="$1"; shift
    if "$@" >/dev/null 2>&1; then
        echo "  PASS  $desc"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $desc"
        FAIL=$((FAIL + 1))
    fi
}

check_eq() {
    local desc="$1" expected="$2" actual="$3"
    if [ "$actual" = "$expected" ]; then
        echo "  PASS  $desc"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $desc  (expected='$expected'  got='$actual')"
        FAIL=$((FAIL + 1))
    fi
}

die() { echo "FATAL: $*" >&2; exit 1; }

# -------------------------------------------------------------------------
# Setup: mount and register cleanup handler
# -------------------------------------------------------------------------

[ -x "$BINARY" ] || die "Binary not found: $BINARY  (run 'make' first)"

mkdir -p "$MOUNT"

cleanup() {
    fusermount3 -u "$MOUNT" 2>/dev/null || true
    rmdir "$MOUNT"  2>/dev/null || true
}
trap cleanup EXIT

# Mount (FUSE daemonises by default).
"$BINARY" "$MOUNT"

# Wait up to 2 s for the mountpoint to become active.
for _ in $(seq 1 20); do
    mountpoint -q "$MOUNT" 2>/dev/null && break
    sleep 0.1
done
mountpoint -q "$MOUNT" || die "Mount did not appear after 2 s"

# -------------------------------------------------------------------------
# Tests
# -------------------------------------------------------------------------

echo ""
echo "fuse_vfs integration tests"
echo ""

# --- Root directory ---
check    "ls root succeeds"             ls "$MOUNT"
check    "root has /counter"            test -f "$MOUNT/counter"
check    "root has /docs (dir)"         test -d "$MOUNT/docs"
check    "root has /data (dir)"         test -d "$MOUNT/data"

# --- File reads ---
check_eq "cat /counter"                 "0"           "$(cat "$MOUNT/counter" | tr -d '\n')"
check_eq "cat /data/sample.txt"         "hello world" "$(cat "$MOUNT/data/sample.txt" | tr -d '\n')"
check_eq "wc -c /data/binary.bin"       "6"           "$(wc -c < "$MOUNT/data/binary.bin" | tr -d ' ')"

# --- Nested directories ---
check    "ls /docs succeeds"            ls "$MOUNT/docs"
check    "ls /data succeeds"            ls "$MOUNT/data"
check    "docs has readme.txt"          test -f "$MOUNT/docs/readme.txt"
check    "data has sample.txt"          test -f "$MOUNT/data/sample.txt"
check    "data has binary.bin"          test -f "$MOUNT/data/binary.bin"
check    "cat /docs/readme.txt"         cat "$MOUNT/docs/readme.txt"

# --- stat sizes ---
check_eq "size of /counter is 2"        "2"  "$(stat -c%s "$MOUNT/counter")"
check_eq "size of /data/sample.txt"     "12" "$(stat -c%s "$MOUNT/data/sample.txt")"
check_eq "size of /data/binary.bin"     "6"  "$(stat -c%s "$MOUNT/data/binary.bin")"

# --- Repeated opens (stress the open/read/close path) ---
for i in 1 2 3 4 5; do
    check "repeated open/read $i"       cat "$MOUNT/counter"
done

# --- Negative cases ---
check    "nonexistent path fails"       bash -c "! stat '$MOUNT/nonexistent' 2>/dev/null"
check    "nonexistent nested fails"     bash -c "! cat '$MOUNT/docs/nope' 2>/dev/null"

# --- Write: overwrite existing file ---
check    "write to existing file"       bash -c "echo 'updated' > '$MOUNT/counter'"
check_eq "read back overwritten file"   "updated" "$(cat "$MOUNT/counter" | tr -d '\n')"

# --- Write: create new file ---
check    "create new file via touch"    touch "$MOUNT/newfile"
check    "new file visible after touch" test -f "$MOUNT/newfile"
check    "write content to new file"    bash -c "echo 'newcontent' > '$MOUNT/newfile'"
check_eq "read back new file"           "newcontent" "$(cat "$MOUNT/newfile" | tr -d '\n')"

# --- Write: append (write past EOF) ---
check    "append to file"               bash -c "printf 'line2\n' >> '$MOUNT/data/sample.txt'"
check    "appended content readable"    grep -q "line2" "$MOUNT/data/sample.txt"

# --- mkdir and rmdir ---
check    "mkdir new directory"          mkdir "$MOUNT/newdir"
check    "new dir is visible"           test -d "$MOUNT/newdir"
check    "create file inside new dir"   bash -c "echo 'inside' > '$MOUNT/newdir/f'"
check_eq "read file inside new dir"     "inside" "$(cat "$MOUNT/newdir/f" | tr -d '\n')"
check    "unlink file inside dir"       rm "$MOUNT/newdir/f"
check    "rmdir empty directory"        rmdir "$MOUNT/newdir"
check    "removed dir is gone"          bash -c "! test -d '$MOUNT/newdir' 2>/dev/null"

# --- unlink ---
check    "unlink file"                  rm "$MOUNT/newfile"
check    "unlinked file is gone"        bash -c "! test -f '$MOUNT/newfile' 2>/dev/null"

# -------------------------------------------------------------------------
# Summary
# -------------------------------------------------------------------------

echo ""
if [ "$FAIL" -eq 0 ]; then
    echo "All $PASS checks passed."
    exit 0
else
    echo "$FAIL/$((PASS + FAIL)) checks FAILED."
    exit 1
fi
