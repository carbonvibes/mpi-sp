# Pre-Week 4 Side Quest: Rename and Symlink Support

## Why this exists

Before starting Week 4 (control plane and mutation model), two missing VFS/FUSE
features should be knocked out. They are not needed for the toy demo harness
(Week 6), but they ARE needed before the real-world OCI runtime integration
(Week 8), and implementing them now keeps the VFS API stable — adding them
later would require touching the same files again mid-integration.

Both are small (30–60 lines each). Doing them now removes two known gaps before
complexity rises.

---

## 1. `vfs_rename`

### What it is

Move a node (file or directory) from one path to another within the VFS tree.
This is the POSIX `rename(2)` / `renameat2(2)` semantic.

### Why it is needed

OCI runtimes (runc, crun) routinely rename temp files into their final
locations during container setup — for example, writing `/etc/hostname.tmp`
and then renaming it to `/etc/hostname`. Without rename support, those
operations return `ENOSYS` and the runtime aborts before reaching any
interesting code paths.

### What needs to be implemented

**VFS core (`vfs/vfs.h` and `vfs/vfs.c`):**

Add declaration to `vfs.h`:

```c
/*
 * Rename (move) oldpath to newpath.
 * - If newpath exists and is an empty directory, it is replaced.
 * - If newpath exists and is a regular file, it is atomically replaced.
 * - Moving a directory into its own subtree returns -EINVAL.
 * - Root cannot be renamed.
 */
int vfs_rename(vfs_t *vfs, const char *oldpath, const char *newpath);
```

Implementation logic in `vfs.c` (in order):

1. Reject `oldpath == "/"` and `newpath == "/"` → `-EINVAL`
2. Call `resolve_parent` on both paths to get `old_parent`/`oldname` and
   `new_parent`/`newname`
3. Look up `src = dir_lookup_child(old_parent, oldname)` → `-ENOENT` if absent
4. Look up `dst = dir_lookup_child(new_parent, newname)` (may be NULL)
5. If `src == dst`: return 0 (same inode, no-op)
6. **Cycle check** (if src is a directory): walk `new_parent` upward via
   `->parent` pointers; if any ancestor equals `src`, return `-EINVAL`
7. If `dst` exists:
   - `dst` is dir and `src` is not dir → `-EISDIR`
   - `dst` is not dir and `src` is dir → `-ENOTDIR`
   - `dst` is dir and `dst->children != NULL` → `-ENOTEMPTY`
   - Otherwise: `dir_remove_child(new_parent, newname)` + `node_free_deep(dst)`
8. `dir_remove_child(old_parent, oldname)` — detach src
9. `src->parent = new_parent`
10. `dir_add_child(new_parent, newname, src)` — attach src under new name
11. On allocation failure in step 10: reattach to old parent and return error

**FUSE layer (`fuse_vfs/fuse_vfs.c`):**

```c
static int fvfs_rename(const char *oldpath, const char *newpath,
                       unsigned int flags)
{
    (void)flags;   /* RENAME_NOREPLACE / RENAME_EXCHANGE not supported */
    return vfs_rename(g_vfs, oldpath, newpath);
}
```

Add `.rename = fvfs_rename` to the `fuse_operations` struct.

### Test cases to add to `vfs_test.c`

Add a `test_rename()` function:

| Case | Expected result |
|------|----------------|
| Rename file within same dir (`/a` → `/b`) | 0, old path gone, new path has content |
| Rename file to different dir (`/a` → `/d/b`) | 0, moved correctly |
| Rename overwrites existing file | 0, dst content replaced by src |
| Rename a directory | 0, dir moved with children intact |
| src does not exist | `-ENOENT` |
| dst is a non-empty directory | `-ENOTEMPTY` |
| dst is a directory, src is a file | `-EISDIR` |
| dst is a file, src is a directory | `-ENOTDIR` |
| Rename root `/` | `-EINVAL` |
| Move directory into its own subtree | `-EINVAL` |
| src and dst are the same path | `0` (no-op) |

---

## 2. Symlinks

### What they are

A symlink is a filesystem node that stores a target path string instead of
file content. The kernel resolves symlinks transparently — when a process
opens `/lib/libc.so` and `/lib` is a symlink to `/usr/lib`, the kernel follows
the link before invoking any FUSE callback for the final component.

### Why they are needed

Every real Linux rootfs uses symlinks. Typical examples:

```
/bin  → usr/bin
/lib  → usr/lib
/sbin → usr/sbin
/lib64 → usr/lib
```

Without symlink support, importing a real container rootfs into the VFS silently
drops all symlinks, and path lookups that go through them fail with `ENOENT`.
The OCI runtime fails before reaching interesting code.

### What needs to be implemented

**VFS core (`vfs/vfs.h` and `vfs/vfs.c`):**

1. Add `VFS_SYMLINK` to the `vfs_kind_t` enum:
   ```c
   typedef enum { VFS_FILE, VFS_DIR, VFS_SYMLINK } vfs_kind_t;
   ```

2. Add a `link_target` field to `vfs_node_t`:
   ```c
   char *link_target;   /* VFS_SYMLINK only; heap-allocated, NUL-terminated */
   ```

3. Add to `vfs.h`:
   ```c
   /* Create a symlink at path pointing to target. */
   int vfs_symlink(vfs_t *vfs, const char *path, const char *target);

   /* Copy link target into buf (up to bufsz bytes, not NUL-terminated). */
   int vfs_readlink(vfs_t *vfs, const char *path, char *buf, size_t bufsz);
   ```

4. In `vfs.c`:
   - `vfs_symlink`: allocate node with kind `VFS_SYMLINK`, `strdup` the target
     into `link_target`, attach to parent via `dir_add_child`
   - `vfs_readlink`: resolve path, check kind is `VFS_SYMLINK` (else `-EINVAL`),
     `memcpy` up to `bufsz` bytes of `link_target` into buf, return byte count
   - `node_free_self`: add `free(n->link_target)`
   - `node_deepcopy`: add `dst->link_target = strdup(src->link_target)` for
     `VFS_SYMLINK` nodes
   - `fill_stat`: for `VFS_SYMLINK`, set `st->size = strlen(n->link_target)`

5. `vfs_delete_file` already goes through `dir_remove_child` + `node_free_deep`,
   so symlink deletion works through `unlink` with no extra changes.

**FUSE layer (`fuse_vfs/fuse_vfs.c`):**

1. `vfs_stat_to_stat`: add case for `VFS_SYMLINK`:
   ```c
   } else if (vs->kind == VFS_SYMLINK) {
       st->st_mode  = S_IFLNK | 0777;
       st->st_nlink = 1;
       st->st_size  = (off_t)vs->size;   /* strlen(target) */
   }
   ```

2. Add `fvfs_symlink` and `fvfs_readlink`:
   ```c
   static int fvfs_symlink(const char *target, const char *linkpath)
   {
       return vfs_symlink(g_vfs, linkpath, target);
   }

   static int fvfs_readlink(const char *path, char *buf, size_t size)
   {
       return vfs_readlink(g_vfs, path, buf, size);
   }
   ```
   Note: FUSE's `symlink` callback has `(target, linkpath)` order (reversed from
   the intuitive order — match it exactly or the kernel gets confused).

3. Add `.symlink = fvfs_symlink, .readlink = fvfs_readlink` to the ops struct.

### Test cases to add to `vfs_test.c`

Add a `test_symlink()` function:

| Case | Expected result |
|------|----------------|
| Create symlink, `vfs_readlink` returns correct target | 0, target matches |
| `vfs_getattr` on symlink returns `VFS_SYMLINK` kind | kind == VFS_SYMLINK |
| `vfs_getattr` size equals `strlen(target)` | size correct |
| `vfs_readlink` on a file → `-EINVAL` | `-EINVAL` |
| `vfs_readlink` on non-existent path → `-ENOENT` | `-ENOENT` |
| Delete symlink via `vfs_delete_file` | 0, no longer exists |
| Symlink preserved across snapshot/restore | readlink returns same target |
| Duplicate symlink at same path → `-EEXIST` | `-EEXIST` |

---

## Effort estimate

| Task | Lines of code | Risk |
|------|--------------|------|
| `vfs_rename` (core + FUSE + tests) | ~80 | Low — uses existing helpers |
| Symlink (core + FUSE + tests) | ~80 | Low — no path resolver changes needed |

The kernel follows symlinks automatically before FUSE callbacks see the path,
so there is no need to change `resolve_path`. That is the main reason symlink
support is simpler than it looks.

---

## Validation before proceeding to Week 4

- `make test` in `vfs/` passes all checks including the new rename and symlink
  suites
- Manual shell validation of rename: `mv file newname` on the mounted filesystem
  succeeds and the old path is gone
- Manual shell validation of symlink: `ln -s target linkname` creates the link,
  `ls -la` shows it, `readlink` returns the correct target
