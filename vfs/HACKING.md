# HACKING — Build, Test, and Run Guide

Quick reference for building everything, running the tests, and using the FUSE demo.

---

## Prerequisites

```sh
sudo apt install libfuse3-dev pkg-config gcc make
```

Verify:

```sh
pkg-config --modversion fuse3   # should print 3.x.x
fusermount3 --version
```

---

## Build

```sh
make          # builds counter_fs, benchmark, vfs/vfs_test
```

Individual targets:

```sh
make counter_fs      # FUSE counter filesystem (toy backend)
make benchmark       # benchmark driver
make vfs/vfs_test    # in-memory VFS unit tests
```

Clean:

```sh
make clean
```

---

## Run the VFS unit tests

```sh
make test
```

Or directly:

```sh
./vfs/vfs_test
```

Expected output:

```
path_parsing
create_file
mkdir
readdir
read
update_file
delete_file
rmdir
nested
mutation_sequence
snapshot_reset
snapshot_nested
invariants
random_sequence

All 375 checks passed.
```

Exit code is 0 on success, 1 if any check fails.

---

## Run the FUSE benchmark

This mounts the counter filesystem, runs 60 seconds of open-read-close cycles, and prints ops/sec.

```sh
mkdir -p /tmp/testmount

# mount in foreground (background the process)
./counter_fs /tmp/testmount -f &

# sanity check
cat /tmp/testmount/counter   # prints 0
cat /tmp/testmount/counter   # prints 1

# run the benchmark (takes 60 seconds)
./benchmark

# unmount when done
fusermount3 -u /tmp/testmount
```

Expected result: ~14,000–15,000 ops/sec in this environment. See [docs/benchmark_baseline.md](docs/benchmark_baseline.md) for the recorded baseline and the regression threshold (10,000 ops/sec).

---

## Repository layout

```
vfs/
  vfs.h           public API for the in-memory VFS core
  vfs.c           implementation (no FUSE dependency)
  vfs_test.c      unit tests

docs/
  benchmark_baseline.md   recorded benchmark runs and machine info
  vfs_design_v1.md        VFS v1 scope: supported ops, deferred features, invariants

counter_fs.c              FUSE counter filesystem (toy backend, used for benchmarking)
benchmark.c               benchmark driver

others/
  counter_fs_offset0.c          variant: increment only at offset 0
  counter_fs_open_snapshot.c    variant: increment on open, stable per-handle snapshot

Makefile
specs.txt                 original project spec
project_execution_plan.md 10-week execution plan
```

---

## Key design decisions

- The VFS core (`vfs/`) has no FUSE dependency and can be tested standalone.
- The mounted filesystem is read-only from the target program's perspective; mutations go through the control path only.
- `vfs_save_snapshot` / `vfs_reset_to_snapshot` are the reset primitives for between-iteration cleanup.
- See [docs/vfs_design_v1.md](docs/vfs_design_v1.md) for the full v1 scope and what is explicitly deferred.
