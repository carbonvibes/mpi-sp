# Fuzzer Coverage Improvement Suggestions

Current coverage plateaus at ~14%. Root causes and fixes ranked by impact.

---

## 1. Exec never succeeds (biggest gap)

All post-exec code paths are permanently unreachable: `wait_for_process`,
`reap_subprocesses`, cleanup, signal re-raise, post-create hooks, etc.

Two blockers:

**a) FUSE serves all files as `0644` (no execute bit)**

`find_executable` calls `access(path, X_OK)` after `pivot_root`. Since FUSE
reports `S_IFREG | 0644` for every file (`fuse_vfs/fuse_vfs.c:27`),
`access` returns EACCES and `find_executable` bails with
`"the path X exists but it is not executable"` — a normal error, never a crash.

Fix: change line 27 in `fuse_vfs/fuse_vfs.c`:
```c
// before
st->st_mode = S_IFREG | 0644;
// after
st->st_mode = S_IFREG | 0755;
```

**b) `/bin/true` in VFS baseline is dynamically linked**

The fuzzer reads `/bin/true` bytes from the host at startup and puts them in
the VFS. But `/bin/true` on this system is dynamically linked — it needs
`/lib64/ld-linux-x86-64.so.2` and `libc.so.6`. After `pivot_root`, the kernel
looks for the dynamic linker inside the FUSE rootfs — not there — `execv`
returns ENOENT. Pointing the fuzzer at a statically linked host binary is not
sufficient either: after `pivot_root` the kernel has no access to host paths,
so the only libraries it can find are those present in the FUSE rootfs itself.

Fix: embed a pre-compiled **static ELF blob** directly in the fuzzer source as
a Rust `const [u8; N]` — a minimal `exit(0)` binary compiled with
`musl-gcc -static` or taken from `busybox-static`. Remove the
`std::fs::read("/bin/true")` call entirely so the VFS baseline always gets a
known-good static binary regardless of what is installed on the host.

---

## 2. Mount entries exercise almost no options coverage

The grammar can generate at most one mount entry per config, and the
`MOUNT_ITEM` rule has no `options` field. `libcrun_set_mounts` contains a large
amount of code covering bind-mount flag combinations, tmpfs size/mode options,
`MS_REC`, propagation flags (`rprivate`, `shared`, `slave`), and per-type
devtmpfs setup — none of which is reachable with a single options-less mount.

Fix:
- Add a `MOUNT_LIST` recursive rule (mirroring the existing `NAMESPACE_LIST`
  pattern) so configs can contain multiple mounts.
- Add an `OPTIONS` sub-rule to `MOUNT_ITEM` generating arrays like
  `["nosuid","noexec","nodev"]`, `["bind"]`, `["rbind","rprivate"]`,
  `["mode=755","size=65536k"]` — these drive the flag-parsing branches in
  `libcrun_set_mounts`.

---

## 3. Grammar generates process paths that do not exist in the VFS

When the grammar outputs `process: /bin/bash`, exec fails at `find_executable`
with ENOENT because `/bin/bash` is not in the VFS baseline. This silently
wastes every input that uses a non-`/bin/true` process path. The grammar also
generates `./app` with `cwd: /app` — a double miss since neither the binary
nor the working directory exists.

Fix (pick one or both):
- Constrain the grammar to only generate `/bin/true` for the process args field.
- Add more static binaries to the VFS baseline (`/bin/sh`, `/bin/bash`) so
  those paths actually resolve.

---

## Quick wins (do these first)

1. Fix `0644 → 0755` in `fuse_vfs/fuse_vfs.c:27` — one line change.
2. Embed a static ELF blob for `/bin/true` in the fuzzer — remove the
   `std::fs::read("/bin/true")` call and replace with a `const` byte array.

These two together should push coverage well past the 14% plateau.

Ops applied to in-memory VFS → FUSE serves it at /tmp/campaign3-fuse-XXXX/
Config JSON written with root.path pointing to that FUSE mount
crun forks a child (container init)
Child sets up namespaces, then reads things from the FUSE rootfs before pivot_root — /etc/passwd and /etc/group to resolve UID/GID for the process user (supplementary group membership), not for cgroups
Cgroups are set up separately using the host's /sys/fs/cgroup based on cgroupsPath from the config — the FUSE rootfs isn't involved there at all
Child sets up mounts (proc, sysfs, devtmpfs) at paths inside the FUSE rootfs
pivot_root — now the FUSE mount is /
After pivot_root: find_executable("/bin/bash") → access("/bin/bash", X_OK) → hits our FUSE file now served as 0755 → passes
exec("/bin/bash") → our static exit(0) runs → exits
Parent cleans up cgroup + state dir


Hello , quick update on coverage, we were plateauing at ~14% and figured out something. After `pivot_root`, crun checks execute permission on the process binary before exec'ing it, but our FUSE layer was serving files as 0644 so that check always failed and exec never happened. Fixed the FUSE mode to 0755 and replaced the dummy placeholder binaries in the rootfs baseline (like `/bin/bash`) with actual static exit(0) binaries, they were essentially empty/non-executable before so exec always failed anyway. Also populated the baseline with the other paths the grammar generates. So technically we can now cover code past crun exec'ing. Now hitting 1680 edges (15%+) within the first few minutes vs plateauing at 1650 after 10+ hours before.
