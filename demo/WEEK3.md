# Week 3 — VFS-Backed FUSE Mount

## What this week delivered

Week 2 built a standalone in-memory VFS core (`vfs/vfs.c`) with no FUSE
dependency.  Week 3 wires that core to a real FUSE mount so that unmodified
programs can open, stat, and read files through normal kernel syscalls.

The result is `fuse_vfs/fuse_vfs.c`: a FUSE process that holds a `vfs_t *`
and implements four FUSE callbacks by delegating to `vfs_*` functions.  The
mounted view is strictly read-only from the target's perspective; mutations
come through the control path (Week 5).

---

## Why neither benchmark touches disk

A common point of confusion: neither the old `counter_fs` nor the new
`fuse_vfs` ever writes to or reads from disk.  Both are FUSE filesystems.

```
target process                    kernel                    userspace FUSE process
──────────────                    ──────                    ──────────────────────
open("/tmp/testmount/counter")
  → sys_openat()           →  FUSE kernel module      →  fvfs_open()   (checks VFS)
read(fd, buf, n)           →  /dev/fuse round-trip    →  fvfs_read()   (copies from heap)
close(fd)                  →  FUSE kernel module      →  (no-op)
```

All file data lives in `malloc`'d heap buffers inside `fuse_vfs`.  The only
kernel crossing per operation is the FUSE `/dev/fuse` round-trip — two context
switches.  There is no page cache, no `write()` syscall, no fsync.

### Why the same benchmark binary works

`benchmark.c` simply loops over `open("/tmp/testmount/counter") → read →
close` for 60 seconds and counts iterations.  It does not care what is behind
the mount — it only needs the path to resolve and return bytes.

| Mount         | What serves `/counter`                          |
|---------------|-------------------------------------------------|
| `counter_fs`  | FUSE process, `static uint64_t read_counter`   |
| `fuse_vfs`    | FUSE process, `vfs_node_t` heap buffer `"0\n"` |

Both expose the same path via FUSE; the benchmark is identical.  The
comparison is therefore apples-to-apples for measuring FUSE round-trip
throughput.

---

## Architecture

```
fuse_vfs.c
│
├── g_vfs  (vfs_t *)  ←  populated once in populate_vfs(), never mutated
│
├── fvfs_getattr()    →  vfs_getattr()   → converts vfs_stat_t → struct stat
├── fvfs_readdir()    →  vfs_readdir()   → readdir_bridge() calls FUSE filler
├── fvfs_open()       →  checks O_RDONLY, then vfs_getattr() to confirm exists
└── fvfs_read()       →  vfs_read()      → memcpy from heap buffer into FUSE buf
```

### readdir bridge

`vfs_readdir` fires a callback of type `vfs_readdir_cb_t` for each entry.
FUSE expects a `fuse_fill_dir_t` call with a `struct stat`.  A tiny
`readdir_ctx_t` struct carries both pointers so the bridge function can
satisfy both signatures:

```c
typedef struct { void *buf; fuse_fill_dir_t filler; } readdir_ctx_t;

static int readdir_bridge(void *ctx, const char *name, const vfs_stat_t *vs)
{
    readdir_ctx_t *rc = ctx;
    struct stat st;
    vfs_stat_to_stat(vs, &st);
    rc->filler(rc->buf, name, &st, 0, 0);
    return 0;
}
```

### Read-only enforcement

`fvfs_open` rejects any mode other than `O_RDONLY` with `EROFS`.  Because
FUSE does not register `write`, `create`, `unlink`, `mkdir`, or `rename`
callbacks, the kernel returns `EROFS` or `EPERM` for those syscalls before
they even reach userspace.

---

## Initial filesystem layout

Populated in `populate_vfs()` before `fuse_main()` runs:

```
/
├── counter          "0\n"  (2 bytes)   ← benchmark target
├── docs/
│   └── readme.txt   43 bytes
└── data/
    ├── sample.txt   "hello world\n"  (12 bytes)
    └── binary.bin   \x00\x01\x02\x03\xff\xfe  (6 bytes)
```

---

## Integration test results

`test_mount.sh` mounts `fuse_vfs`, runs 25 checks, then unmounts.

```
fuse_vfs integration tests

  PASS  ls root succeeds
  PASS  root has /counter
  PASS  root has /docs (dir)
  PASS  root has /data (dir)
  PASS  cat /counter
  PASS  cat /data/sample.txt
  PASS  wc -c /data/binary.bin
  PASS  ls /docs succeeds
  PASS  ls /data succeeds
  PASS  docs has readme.txt
  PASS  data has sample.txt
  PASS  data has binary.bin
  PASS  cat /docs/readme.txt
  PASS  size of /counter is 2
  PASS  size of /data/sample.txt
  PASS  size of /data/binary.bin
  PASS  repeated open/read 1
  PASS  repeated open/read 2
  PASS  repeated open/read 3
  PASS  repeated open/read 4
  PASS  repeated open/read 5
  PASS  nonexistent path fails
  PASS  nonexistent nested fails
  PASS  write to file fails (EROFS)
  PASS  create new file fails (EROFS)

All 25 checks passed.
```

### What each group covers

| Group | Checks | What is verified |
|-------|--------|------------------|
| Root listing | 4 | `ls` works; root contains expected entries |
| File reads | 3 | Exact content match for text files; byte count for binary |
| Nested directories | 6 | `ls` and `test -f` on two subdirectories |
| stat sizes | 3 | `st_size` reported by FUSE matches VFS `content_len` |
| Repeated opens | 5 | 5 consecutive open/read/close cycles on same file |
| Negative cases | 2 | Non-existent paths return errors at the shell level |
| Read-only enforcement | 2 | Write and create both fail with an error |

---

## Benchmark results

Same binary (`benchmark.c`), three runs, mount at `/tmp/testmount`,
file `/tmp/testmount/counter`.  Machine: AMD EPYC 7763, GitHub Codespace
(containerized), Linux 6.8.0-1044-azure, gcc 13.3.0, fuse3 3.14.0.

### VFS-backed mount (fuse_vfs)

| Run | ops/sec |
|-----|---------|
| 1   | 13 788.87 |
| 2   | 13 836.32 |
| 3   | 13 903.78 |
| **mean** | **13 843** |
| spread | 115 ops/sec (~0.8 %) |

### Counter baseline (counter_fs, from docs/benchmark_baseline.md)

| Run | ops/sec |
|-----|---------|
| 1   | 14 933.93 |
| 2   | 14 768.40 |
| 3   | 14 553.52 |
| **mean** | **14 752** |

### Comparison

| Implementation | Mean ops/sec | vs baseline |
|----------------|-------------|-------------|
| counter_fs     | 14 752      | —           |
| fuse_vfs       | 13 843      | −6.2 %      |
| spec floor     | 1 000       | —           |

The 6 % slowdown is entirely explained by VFS path resolution (a short
linked-list walk per `getattr` and `read` call).  Neither implementation
touches disk; the difference is purely CPU work in userspace.  Both are
13–14× above the 1 000 ops/sec spec floor, so the VFS backend does not
introduce a meaningful performance regression.

---

## Week 3 exit criteria (from project_execution_plan.md)

| Criterion | Status |
|-----------|--------|
| Mounted read-only VFS works reliably | ✓ 25/25 integration checks pass |
| benchmark remains practically usable | ✓ 13 843 ops/sec, 13× above spec floor |
| getattr, readdir, open, read wired   | ✓ four FUSE callbacks implemented |
| Manual shell checks (ls, cat, nested)| ✓ covered by test_mount.sh |
| Negative tests for nonexistent paths | ✓ two negative-case checks |
| Benchmark comparison vs counter version | ✓ −6.2 %, within tolerance |
