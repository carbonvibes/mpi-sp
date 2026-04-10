#!/bin/bash

# Benchmark script comparing native filesystem vs FUSE filesystem
# Only tests READ operations (write not implemented in counter_fs)

TEST_DIR="/tmp/benchmark_test_data"
NATIVE_TIME_FILE="/tmp/native_times.txt"
FUSE_TIME_FILE="/tmp/fuse_times.txt"
FUSE_MOUNT="/tmp/testmount"

echo "=== Filesystem Benchmark: Native vs FUSE ==="
echo ""

# Step 1: Create test data
echo "[1/6] Creating test data..."
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"

# Create 30 files with varying sizes
for i in {1..30}; do
  dd if=/dev/urandom of="$TEST_DIR/file_$i.bin" bs=1K count=$((RANDOM % 50 + 10)) 2>/dev/null
done

# Create a tar archive for tar extraction test
tar -czf "$TEST_DIR.tar.gz" "$TEST_DIR" 2>/dev/null

echo "Test data created: $(du -sh $TEST_DIR | cut -f1)"
echo ""

# Function to time a command
time_command() {
  local start=$(date +%s%N)
  "$@" > /dev/null 2>&1
  local end=$(date +%s%N)
  echo "scale=3; ($end - $start) / 1000000000" | bc
}

# Step 2: Benchmark on native filesystem
echo "[2/6] Benchmarking native filesystem..."
> "$NATIVE_TIME_FILE"

# Test 1: tar extraction
echo -n "  - tar extraction: "
rm -rf /tmp/native_extract
tar_time=$(time_command tar -xzf "$TEST_DIR.tar.gz" -C /tmp)
echo "${tar_time}s"
echo "tar_extraction: $tar_time" >> "$NATIVE_TIME_FILE"

# Test 2: find operation
echo -n "  - find (20 iterations): "
find_time=$(time_command bash -c "for i in {1..20}; do find \"$TEST_DIR\" -type f > /dev/null 2>&1; done")
echo "${find_time}s"
echo "find: $find_time" >> "$NATIVE_TIME_FILE"

# Test 3: grep recursive
echo -n "  - grep -r (5 iterations): "
grep_time=$(time_command bash -c "for i in {1..5}; do grep -r 'test' \"$TEST_DIR\" > /dev/null 2>&1 || true; done")
echo "${grep_time}s"
echo "grep: $grep_time" >> "$NATIVE_TIME_FILE"

# Test 4: cat all files
echo -n "  - cat all files (5 iterations): "
cat_time=$(time_command bash -c "for i in {1..5}; do for f in \"$TEST_DIR\"/*; do cat \"\$f\" > /dev/null 2>&1; done; done")
echo "${cat_time}s"
echo "cat: $cat_time" >> "$NATIVE_TIME_FILE"

echo ""

# Step 3: Mount FUSE filesystem
echo "[3/6] Mounting FUSE filesystem..."

# Kill any existing counter_fs processes
pkill -f "counter_fs" 2>/dev/null || true
sleep 1

rm -rf "$FUSE_MOUNT"
mkdir -p "$FUSE_MOUNT"

# Start FUSE filesystem
./counter_fs "$FUSE_MOUNT" -f > /tmp/fuse.log 2>&1 &
FUSE_PID=$!

# Wait for mount with retries
MAX_RETRIES=10
RETRY_COUNT=0
while ! mountpoint -q "$FUSE_MOUNT" && [[ $RETRY_COUNT -lt $MAX_RETRIES ]]; do
  sleep 0.5
  RETRY_COUNT=$((RETRY_COUNT + 1))
done

# Verify mount
if ! mountpoint -q "$FUSE_MOUNT"; then
  echo "ERROR: FUSE mount failed after $MAX_RETRIES retries"
  cat /tmp/fuse.log 2>/dev/null
  kill $FUSE_PID 2>/dev/null || true
  exit 1
fi

echo "FUSE mounted at $FUSE_MOUNT (PID: $FUSE_PID)"
echo ""

# Step 4: Test FUSE with read-only operations
echo "[4/6] Testing FUSE filesystem (read-only)..."

# Since counter_fs is read-only, we'll test the counter file
echo -n "  Verifying counter file: "
if cat "$FUSE_MOUNT/counter" > /dev/null 2>&1; then
  echo "OK"
else
  echo "FAILED"
  fusermount3 -u "$FUSE_MOUNT" 2>/dev/null || true
  kill $FUSE_PID 2>/dev/null || true
  exit 1
fi

sleep 1

# Step 5: Benchmark FUSE counter reads
echo ""
echo "[5/6] Benchmarking FUSE counter (read-only)..."
> "$FUSE_TIME_FILE"

echo -n "  - Sequential counter reads (100 reads): "
counter_time=$(time_command bash -c "for i in {1..100}; do cat \"$FUSE_MOUNT/counter\" > /dev/null; done")
echo "${counter_time}s"
echo "counter_reads: $counter_time" >> "$FUSE_TIME_FILE"

echo -n "  - Random counter reads (50 reads): "
counter_random=$(time_command bash -c "for i in {1..50}; do cat \"$FUSE_MOUNT/counter\" > /dev/null; done")
echo "${counter_random}s"
echo "counter_random: $counter_random" >> "$FUSE_TIME_FILE"

echo ""

# Step 6: Cleanup and report
echo "[6/6] Cleanup and reporting..."
fusermount3 -u "$FUSE_MOUNT" 2>/dev/null || true
kill $FUSE_PID 2>/dev/null || wait $FUSE_PID 2>/dev/null || true
sleep 1

echo ""
echo "=== RESULTS ==="
echo ""
echo "=== Native Filesystem (tar, find, grep, cat) ==="
printf "%-18s | %11s\n" "Operation" "Time (s)"
echo "-------------------|----------"

while IFS=": " read -r test_name native_time; do
  printf "%-18s | %11s\n" "$test_name" "$native_time"
done < "$NATIVE_TIME_FILE"

echo ""
echo "=== FUSE Filesystem (/counter file - read-only) ==="
printf "%-18s | %11s\n" "Operation" "Time (s)"
echo "-------------------|----------"

while IFS=": " read -r test_name fuse_time; do
  printf "%-18s | %11s\n" "$test_name" "$fuse_time"
done < "$FUSE_TIME_FILE"

echo ""
echo "Test complete. Logs available at:"
echo "  Native times: $NATIVE_TIME_FILE"
echo "  FUSE times:   $FUSE_TIME_FILE"
echo "  FUSE log:     /tmp/fuse.log"
echo ""

# Cleanup
rm -rf "$TEST_DIR" "$TEST_DIR.tar.gz" /tmp/native_extract /tmp/fuse_extract
