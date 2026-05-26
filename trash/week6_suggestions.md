# Week 6 Suggestions

This document is a critique and proposed refinement of the current Week 6 plan
in `project_execution_plan.md`. The short version: the current direction is
right, but Week 6 should be made more focused. It should become the symlink week:
finish `CreateSymlink`, make symlink replacement robust, and target crun's
specific path-resolution behavior instead of spreading effort across unrelated
filesystem metadata ops.

## Overall Position

I mostly agree with the Week 6 plan. The important insight is correct: symlinks
are high-value for crun because crun performs sensitive path operations before
`pivot_root`, while paths are still interpreted from the host side of the
rootfs. Mutating regular file contents will not reliably reach this class of
behavior. Mutating filesystem shape, especially symlinked parents and mount
destinations, can.

However, I would modify the plan in three ways:

1. Make path replacement with symlinks reliable.
2. Add crun-specific coordinated mutators for mount options and executable
   paths.
3. Push unrelated op vocabulary, such as xattrs, chmod, and chown, out of Week
   6 unless everything symlink-related is already done.

One stale assumption to remove from the plan is the idea that mount destination
replacement can be expressed directly as `rmdir(path) + create_symlink(path,
target)`. That is only true for empty directories. The better framing is:

> No new serialized fuzzing op is needed beyond `CreateSymlink`. Higher-level
> replacement is handled by a Rust-side helper, `replace_with_symlink(...)`,
> which expands into delete/rmdir/create-symlink primitives based on path kind
> and known children.

## Fix The Motivation Text

The current plan says the pre-pivot symlink issue is "the exact bug class that
killed runc (CVE-2019-5736)." That is close in spirit but technically too
strong. CVE-2019-5736 centered on `/proc/self/exe` overwrite during container
execution, not simply pre-pivot symlink traversal.

Suggested replacement:

> This overlaps with the same family of runtime path-resolution and
> `/proc/self/exe` escape hazards, including runc-style attacks, but the
> specific crun target here is pre-pivot and mount-destination path handling.

That wording keeps the argument strong without overstating the CVE mapping.

## Highest Priority Change: Robust Symlink Replacement

The current plan often uses this pattern:

```rust
FsOp::rmdir("/proc")
FsOp::create_symlink("/proc", "../../proc")
```

That only works when `/proc` is an empty directory. For paths like `/etc`,
`/dev`, `/bin`, `/usr`, and `/lib`, this will often fail because the directory
has children. If the `rmdir` fails, the `create_symlink` will also fail with
`EEXIST`, and the supposedly interesting seed will not actually create the
symlink.

Week 6 should add a helper concept:

```rust
replace_with_symlink(path, target)
```

This does not need to be a serialized `FsOpKind` at first. It can be a Rust-side
mutator/seed helper that expands into valid primitive ops:

- Existing file or symlink: `delete_file(path)`, then `create_symlink(path, target)`.
- Empty directory: `rmdir(path)`, then `create_symlink(path, target)`.
- Non-empty directory: delete children in postorder, `rmdir(path)`, then
  `create_symlink(path, target)`.

This is probably the most important correction. Without it, many high-value
mount-point and path-component seeds will silently fail before crun reaches the
interesting path.

The helper should be designed around a baseline path-kind index rather than
assuming every LibAFL mutator can inspect the live VFS. Seed generation can
inspect the baseline VFS/tree directly. Mutation code should use cached metadata
plus the current `FsDelta`, or fall back to conservative replacement sequences
for known baseline paths.

Suggested metadata shape:

```rust
enum PathKind {
    File,
    Dir,
    Symlink,
}

struct PathInfo {
    path: String,
    kind: PathKind,
    children: Vec<String>,
}
```

This keeps `replace_with_symlink` implementable in both seed generation and
LibAFL mutation contexts.

## Full Stack Work For `CreateSymlink`

The plan mentions Rust-side plumbing, but the current system applies Rust
`FsDelta` values through the C control plane. So `CreateSymlink` needs to be
threaded through the full stack:

- `mutator/src/delta.rs`: add `FsOpKind::CreateSymlink`.
- `mutator/src/delta.rs`: add `FsOp::create_symlink(path, target)`.
- `mutator/src/delta.rs`: add a dedicated `target: String` field to `FsOp`.
- `control_plane/delta.h`: add a symlink op kind and `delta_add_symlink`.
- `control_plane/delta.c`: store and free the symlink target correctly.
- `control_plane/control_plane.c`: dispatch to `vfs_symlink`.
- `mutator/src/ffi.rs`: add the `delta_add_symlink` FFI binding and match arm.
- tests: serialization, apply path, snapshot/reset, and FUSE `readlink`.

I would avoid stuffing the symlink target into `content`. A `target: String`
field makes the JSON corpus easier to read and avoids confusing file bytes with
link targets.

The important implementation detail is that Rust should continue flowing through
`cp_apply_delta()`. Week 6 should not accidentally introduce a second direct
Rust-to-`vfs_symlink` apply path. The intended route is:

```text
FsOp::CreateSymlink
  -> mutator/src/ffi.rs delta_add_symlink(...)
  -> control_plane/delta.{h,c}
  -> cp_apply_delta(...)
  -> vfs_symlink(...)
  -> FUSE readlink/getattr observes the symlink
```

## New Or Modified Mutators

### 1. `MountDestinationSymlinkMutator`

Keep this as the highest-priority mutator, but make it config-aware. It should
target actual `config.mounts[*].destination` paths when available, not only the
hardcoded defaults.

Targets:

- `/proc`
- `/dev`
- `/sys`
- `/tmp`
- generated mount destinations from OCI config
- nested destinations such as `/a/b/c`
- parent components of mount destinations, such as `/a/b` for `/a/b/c`

This mutator should use `replace_with_symlink`, not raw `rmdir + create_symlink`.

Target categories:

- Relative escape: `../../proc`, `../../../dev`, `../../etc/passwd`.
- Absolute target: `/proc`, `/dev`, `/sys`, `/etc/passwd`.
- Wrong type: `/proc -> /etc/passwd`, `/dev -> /bin/true`.
- Dangling: `/proc -> /nonexistent`.
- Special proc/dev paths: `/proc/self/fd`, `/proc/self/exe`, `/dev/null`.

### 2. `ParentComponentSymlinkMutator`

This is more important than only creating leaf symlinks. It targets a parent
component of a path crun is likely to touch.

Examples:

```text
/etc -> ../../etc
/bin -> ../../bin
/usr/bin -> ../../../usr/bin
/dev -> /proc/self/fd
```

Useful source paths:

- parent of `process.args[0]`
- parent of each mount destination
- parent of `/etc/passwd`
- parent of `/etc/group`
- parent of `/dev/null`
- parent of `/dev/console`
- parent of `/proc/self/fd`

This mutator should also use `replace_with_symlink` so non-empty directories are
handled correctly.

### 3. `AbsoluteTargetSymlinkMutator`

The current plan leans heavily on `../../...` targets. Keep those, but add
absolute symlink targets too:

```text
/proc
/proc/self
/proc/self/exe
/proc/self/fd
/dev
/dev/null
/sys
/etc/passwd
/bin/sh
```

Absolute symlinks are important because pre-pivot and post-pivot path resolution
can differ. If crun's safe-open logic mishandles an absolute symlink before the
rootfs boundary is enforced, that is a high-value finding.

### 4. `MountOptionSymlinkMutator`

This should be added because it is very crun-specific. crun has explicit logic
for bind mount symlink options, including:

- `copy-symlink`
- `src-nofollow`
- `dest-nofollow`
- bind mounts onto symlink destinations

The mutator should coordinate config, host fixtures, and rootfs state:

- for source-related cases, create a host-side temporary symlink fixture and use
  that path as `mount.source`
- for destination-related cases, create a symlink inside the FUSE rootfs and use
  that path as `mount.destination`
- generate options like `["bind", "copy-symlink"]`
- generate options like `["bind", "src-nofollow"]`
- generate options like `["bind", "dest-nofollow"]`
- generate invalid combinations such as `copy-symlink` plus `src-nofollow`
  or `dest-nofollow`

This distinction matters because OCI bind mount `source` paths are generally
host paths. crun's `copy-symlink` handling reads `mount.source` from the host
namespace, while destination symlink behavior is driven by the container rootfs.
So a source symlink created only inside the FUSE rootfs may not exercise the
intended crun branch.

This is likely more useful than generic symlink chains because it drives crun
through symlink-specific branches in mount handling.

### 5. `ExecutableSymlinkMutator`

Split the executable-path part out from the generic coordinated mutator and make
it explicit.

Operation:

1. Pick a synthetic executable path.
2. Override `process.args[0]` to that path.
3. Create that path in the rootfs as a symlink.

Interesting examples:

```text
/bin/target -> /proc/self/exe
/bin/target -> ../../bin/sh
/bin/target -> /dev/null
/bin/target -> /proc/self/fd/0
/usr/local/bin/x -> /nonexistent
```

This directly explores the case where config says "execute X" and rootfs says
"X is a symlink to Y." Without this coordination, the fuzzer only finds these
pairs accidentally.

### 6. `LoopAndDepthMutator`

Keep symlink loops and chains, but lower their priority. They are useful for
robustness and error handling, but they are probably less bug-finding than mount
destinations, parent components, and executable paths.

Useful cases:

- self-loop: `/loop -> /loop`
- two-cycle: `/a -> /b`, `/b -> /a`
- chain lengths: 39, 40, 41
- very long target strings near `PATH_MAX`
- targets with repeated slashes, such as `////proc//self//exe`

The current `{1, 5, 10, 39, 40, 41, 100}` set is fine, but length 100 should not
receive much energy. It mostly confirms `ELOOP` handling.

## Seed Corpus Changes

The seed corpus should be rewritten to use `replace_with_symlink` expansion
where appropriate. The textual seed descriptions can remain high-level, but the
actual emitted ops must successfully remove/replace non-empty paths.

Add or emphasize these seed categories:

- Mount destination is a symlink.
- Parent of mount destination is a symlink.
- Executable path is a symlink.
- Parent of executable path is a symlink.
- `/etc/passwd` and `/etc/group` are symlinks.
- Parent of `/etc/passwd` or `/etc/group` is a symlink.
- Bind mount source is a symlink with `copy-symlink`.
- Bind mount destination is a symlink with `dest-nofollow`.
- Absolute symlink target escapes.
- Relative symlink target escapes.
- Dangling symlink at a path crun expects to exist.
- Symlink loop at a path crun opens.

The snippets below are conceptual. Seed generation should expand each
`replace_with_symlink(...)` helper call into a concrete `Vec<FsOp>` before
constructing the final `FsDelta`.

Example high-value seeds:

```rust
// Mount destination replacement.
replace_with_symlink("/proc", "/proc");
replace_with_symlink("/dev", "/proc/self/fd");
replace_with_symlink("/sys", "../../sys");

// Parent component replacement.
replace_with_symlink("/etc", "../../etc");
replace_with_symlink("/bin", "../../bin");
replace_with_symlink("/usr/bin", "../../../usr/bin");

// Executable path replacement.
replace_with_symlink("/bin/true", "/proc/self/exe");
replace_with_symlink("/bin/true", "/dev/null");
replace_with_symlink("/usr/local/bin/target", "../../bin/sh");

// Config lookup files.
replace_with_symlink("/etc/passwd", "../../etc/passwd");
replace_with_symlink("/etc/group", "/etc/group");

// Loops and dangling paths.
FsDelta::new(vec![FsOp::create_symlink("/loop", "/loop")]);
FsDelta::new(vec![FsOp::create_symlink("/bin/missing", "/nonexistent")]);
```

## New Ops: What To Add And What To Delay

### Add Now

Add only one fuzzing op now:

```rust
CreateSymlink { path, target }
```

It is the essential primitive for this week.

### Add As A Helper, Not A Serialized Op

Add this as a Rust helper used by mutators and seed generation:

```rust
replace_with_symlink(path, target)
```

It should expand into existing primitive ops plus `CreateSymlink`. This keeps
the serialized op vocabulary small while making generated inputs actually work.

### Consider Later

Consider adding this in Week 7 or Week 8:

```rust
DeleteTree { path }
```

It would simplify replacement of non-empty directories, but it broadens the
control-plane semantics. I would not add it until the helper approach becomes
too painful.

### Test-Only Helper

A test-only `readlink` helper would be useful, but it does not need to be part
of the fuzzing op vocabulary. It should verify:

- symlink target serialization
- `apply_delta` created the symlink
- FUSE `readlink` sees the target
- snapshot/reset preserves symlink targets
- symlink loops do not hang VFS/FUSE

## Things To Remove Or Deprioritize

Remove these from Week 6's must-finish scope:

- `SetXattr`
- `RemoveXattr`
- `Chmod`
- `Chown`

They are real bug classes, but they dilute the week. They need VFS support,
FUSE callbacks, control-plane plumbing, mutators, and seeds. That is enough work
to steal focus from symlink support.

Also deprioritize heavy symlink-chain fuzzing. Keep loop/depth cases for
coverage and robustness, but do not let them dominate the mutation schedule.
Mount destinations, executable paths, mount options, and parent components are
more crun-specific.

## Trivial But Useful Additions

Add these small pieces if time permits:

- `enumerate_vfs_symlink_paths`.
- baseline path-kind metadata: file, directory, symlink, children, all.
- guidance treats `ELOOP`, `ENOTDIR`, `EEXIST`, and `EINVAL` as meaningful.
- guidance can choose symlink creation when a crun access path returns `ENOENT`.
- guidance can replace a path with a symlink after observing a read/open on that
  path.
- tests for absolute targets, relative targets, long targets, repeated slashes,
  dangling targets, and loop targets.
- corpus minimization should not reward a failed `rmdir + create_symlink`
  sequence unless the symlink was actually created or crun behavior changed.

## Revised Week 6 Priority Order

1. Implement `CreateSymlink` through the full stack.
2. Add a dedicated `target` field to `FsOp`.
3. Add baseline path-kind metadata and symlink path enumeration.
4. Add robust `replace_with_symlink` helper.
5. Add crun mount-destination symlink seeds and mutator.
6. Add crun mount-option symlink seeds and mutator.
7. Add executable/config coordinated symlink mutator.
8. Add parent-component symlink mutator.
9. Add loop/depth/path-length symlink mutator.
10. Push chmod/chown/xattr work out of Week 6.

Minimum exit criteria should include:

- `CreateSymlink` works through `FsOp -> FFI -> control_plane -> VFS -> FUSE`.
- baseline path-kind index is available to seed generation and mutators.
- `replace_with_symlink` is used anywhere an existing path is replaced.
- mount-option tests distinguish host-side bind sources from rootfs
  destinations.
- seed snippets are expanded into concrete primitive ops before entering the
  corpus.

## Bottom Line

The existing Week 6 plan has the right center of gravity, but it should be
tighter. Make symlinks first-class, make symlink replacement reliable, and spend
mutation energy on crun-specific path handling. Avoid broadening into xattrs,
chmod, and chown until the symlink campaign is complete and producing useful
coverage.
