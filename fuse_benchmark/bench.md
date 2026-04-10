# FUSE Benchmark Results

All benchmarks run on: AMD EPYC 7763, GitHub Codespace (containerized)
Linux 6.8.0-1044-azure, gcc 13.3.0, fuse3 3.14.0.

The FUSE filesystem is a standalone in-memory implementation with no disk I/O. Native comparisons run against `/tmp` (tmpfs).

---

## Workload 1: Basic open/read/close (counter_fs baseline)

Single pre-existing file, looped for 60 seconds.

| | ops/sec |
|---|---|
| FUSE | 14,609.00 |
| Native | 204,469.72 |
| Overhead | ~13x |

---

## Workload 2: Heavy file ops (create/write/read/rename/delete)

2000 files, 4096 bytes each, timed per phase.

| Phase | FUSE | Native | Overhead |
|---|---|---|---|
| create 2000 files | 261 ms | 43 ms | ~6x |
| write 2000 x 4096 B | 223 ms | 21 ms | ~10x |
| read 2000 files | 273 ms | 14 ms | ~19x |
| rename 2000 files | 163 ms | 42 ms | ~4x |
| delete 2000 files | 63 ms | 36 ms | ~1.7x |
| **total** | **983 ms** | **156 ms** | **~6x** |

Sustained cycles (full create→write→read→rename→delete loop, 60 seconds):

| | cycles/sec |
|---|---|
| FUSE | 2,748 |
| Native | 16,507 |
| Overhead | ~6x |

---

## Workload 3: SQLite (WAL insert + scan + lookup)

50,000 row batch insert inside a single WAL transaction, followed by a full
table scan and indexed point lookups.

| | elapsed |
|---|---|
| FUSE | 620 ms |
| Native | 427 ms |
| Overhead | 1.45x |

---

## Workload 4: Real-world program (Python 3.12 stdlib, 5779 entries, 95 MB)

Uses the Python 3.12 standard library as a realistic directory tree. Three
phases: tar extraction (write-heavy), grep -r across all .py files
(read-heavy), and a full find traversal (metadata-heavy).

| Phase | FUSE | Native | Overhead |
|---|---|---|---|
| tar extract (5779 files) | 2310 ms | 274 ms | ~8.4x |
| grep -r (read all .py files) | 698 ms | 42 ms | ~16.6x |
| find traversals | 291 ms | 18 ms | ~16.2x |
| **total** | **3737 ms** | **488 ms** | **~7.7x** |

Read-heavy operations (grep, find) show higher overhead than writes because
each file open and stat is a separate FUSE round-trip with no batching.

---

## Summary

| Workload | FUSE overhead vs native |
|---|---|
| open/read/close (simple) | ~13x |
| create/write/read/rename/delete | ~6x |
| SQLite (transactional) | ~1.45x |
| real-world (tar + grep + find) | ~7.7x |


