# MPI-SP: Fuzzer Filesystem Interface — Progress Log

## What this project is

The goal is to build something that lets a fuzzer (AFL++/LibAFL) mutate filesystem inputs and have those mutations show up as a real, kernel-visible filesystem that an unmodified target program can just open and read. The motivating use case is fuzzing OCI container runtimes — they need both a `config.json` and a `rootfs/` directory, and the `rootfs` part makes standard file-based fuzzing painful: you either rewrite the target, or you re-create a real filesystem on disk for every single iteration, which is slow.

The plan is to use FUSE to expose an in-memory virtual filesystem, with LibAFL on the mutation side. First though, we needed to check whether FUSE is even fast enough to not be a bottleneck.

---

## Where we are: Phase 1 done

**April 2026**

The first phase is done. We wrote a minimal FUSE filesystem and benchmarked it. Here's what happened.

---

## The counter filesystem

The spec called for a FUSE filesystem with a single file whose content is an integer that increments on every read. Simple enough as a model — it forces the filesystem to do real work on every access, so the benchmark measures actual round-trip overhead rather than cached reads.

**Main file:** [`counter_fs.c`](counter_fs.c)

A FUSE3 filesystem with one file, `/counter`. Reading it gives you the current counter value as a string and bumps the counter. Implements `getattr`, `read`, and `readdir`, nothing else.

### The correctness issue we ran into

The first version incremented inside `read()`, which seemed obvious but is actually wrong. `read()` gets called once per read *syscall*, not once per logical file read — if the content is longer than the buffer, the kernel will call `read()` again with a non-zero offset to drain the rest. That means the counter could increment more than once per open-read-close cycle depending on buffer sizes.

Two fixes ended up in `others/`:

| File | What it does |
|------|--------------|
| [`others/counter_fs_offset0.c`](others/counter_fs_offset0.c) | Only increments when `offset == 0`, so follow-up reads don't double-count |
| [`others/counter_fs_open_snapshot.c`](others/counter_fs_open_snapshot.c) | Increments on `open()` instead, saves the value in a per-handle struct via `fuse_file_info->fh`; all `read()` calls for that handle see the same snapshot |

The open-snapshot version is the right design. It means each `open()` gets one stable value, regardless of how many `read()` syscalls follow. This also happens to be exactly the model we want for the real fuzzer integration: one testcase content per execution, stable throughout the run, with mutations taking effect on the next open.

All three versions were benchmarked. They all came out roughly the same — ~13,300 ops/sec for the offset0 and open-snapshot variants, ~13,400 for the original. That makes sense once you understand how the kernel handles short reads.

The initial worry was that the kernel would keep calling `read()` with an increasing offset until it got 0 bytes back — which is the general contract for draining a file — and during that second call the counter would increment again. But in practice, the kernel's VFS layer treats a short read (returned fewer bytes than requested) as EOF and doesn't issue another call. `cat` requests 4096 bytes, the handler returns 2 bytes (`"0\n"`), and the kernel just stops there. So the double-increment bug in the original `counter_fs.c` never actually fires for small content like this. The correctness issue would show up if the content were large enough that a single `read()` couldn't hold it all, forcing a real second call with a non-zero offset — but for a small integer that's never going to happen here.

---

## Benchmark

**File:** [`benchmark.c`](benchmark.c)

Hammers `open` → `read` → `close` on `/tmp/testmount/counter` in a loop for 60 seconds and prints ops/sec:

```c
while (time(NULL) - start < 60) {
    int fd = open("/tmp/testmount/counter", O_RDONLY);
    read(fd, buf, sizeof(buf));
    close(fd);
    count++;
}
printf("%.2f ops/sec\n", count / 60.0);
```

### Result

| Version | Result |
|---------|--------|
| `counter_fs.c` (original) | ~13,400 ops/sec |
| `counter_fs_offset0.c` | ~13,300 ops/sec |
| `counter_fs_open_snapshot.c` | ~13,300 ops/sec |

All three are essentially the same. The spec target was ≥ 1k ops/sec, so this clears it comfortably. FUSE looks fine to continue with.

---

## How to run it yourself

```sh
# build
gcc -o counter_fs counter_fs.c $(pkg-config --cflags --libs fuse3)
gcc -o benchmark benchmark.c

# mount
mkdir -p /tmp/testmount
./counter_fs /tmp/testmount -f &

# sanity check
cat /tmp/testmount/counter   # 0
cat /tmp/testmount/counter   # 1

# benchmark
./benchmark

# cleanup
fusermount3 -u /tmp/testmount
```

---

## Key takeaways from this phase

- FUSE is fast enough. ~13,300–13,400 ops/sec gives plenty of headroom above the 1k/sec target.
- Incrementing on `read()` is subtly wrong — the open-snapshot approach in `counter_fs_open_snapshot.c` is the correct design and carries forward naturally to the real VFS. That said, all three versions scored the same in the benchmark because the content is small enough to fit in one `read()` call, so the bug never manifests here.
- Virtualized environments (containers, codespaces) tank FUSE performance significantly due to extra context-switch overhead. Benchmark numbers from those environments aren't meaningful.

---

## What's next: Phase 2 — In-Memory VFS

Now that we know FUSE works, the next step is replacing the toy counter backend with a real in-memory filesystem:

1. Write a design note (`vfs_design_v1.md`) covering supported node types and operations, and what's explicitly out of scope for v1
2. Implement the VFS core standalone, no FUSE dependency, so it can be unit tested cleanly
3. Wire FUSE onto the VFS core and verify mounted behavior
4. Add mutation application and reset — update the live VFS between iterations without remounting

Full schedule in [`project_execution_plan.md`](project_execution_plan.md).

---

## Repo layout

```
counter_fs.c                  # FUSE counter filesystem
benchmark.c                   # benchmark tool
others/
  counter_fs_offset0.c        # variant: only increment at offset 0
  counter_fs_open_snapshot.c  # variant: increment on open, snapshot per handle (preferred)
specs.txt                     # original project spec
project_execution_plan.md     # 10-week execution plan
```
