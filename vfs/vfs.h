#ifndef VFS_H
#define VFS_H

#include <stddef.h>
#include <stdint.h>

/* -------------------------------------------------------------------------
 * Types
 * ---------------------------------------------------------------------- */

typedef enum {
    VFS_FILE,
    VFS_DIR,
} vfs_kind_t;

typedef struct vfs_dirent vfs_dirent_t;
typedef struct vfs_node   vfs_node_t;

/*
 * Directory entry: singly-linked list of (name, node) pairs owned by
 * the parent directory node.
 */
struct vfs_dirent {
    char         *name;
    vfs_node_t   *node;
    vfs_dirent_t *next;
};

/*
 * Inode. Both files and directories share this struct; kind distinguishes
 * them. content/content_len are used only for VFS_FILE nodes;
 * children is used only for VFS_DIR nodes.
 */
struct vfs_node {
    uint64_t      ino;
    vfs_kind_t    kind;
    uint8_t      *content;      /* VFS_FILE only */
    size_t        content_len;  /* VFS_FILE only */
    vfs_dirent_t *children;     /* VFS_DIR only  */
    vfs_node_t   *parent;       /* NULL for root */
};

/* Top-level filesystem object. */
typedef struct {
    vfs_node_t *root;
    vfs_node_t *snapshot;   /* NULL if no snapshot has been saved */
    uint64_t    next_ino;
} vfs_t;

/* Result of vfs_getattr. */
typedef struct {
    uint64_t   ino;
    vfs_kind_t kind;
    size_t     size;   /* file byte count; 0 for directories */
} vfs_stat_t;

/*
 * Callback for vfs_readdir.
 * Called once per entry (including "." and "..").
 * Return 0 to continue, non-zero to stop early (that value is returned
 * from vfs_readdir to the caller).
 */
typedef int (*vfs_readdir_cb_t)(void *ctx, const char *name,
                                const vfs_stat_t *st);

/* -------------------------------------------------------------------------
 * Lifecycle
 * ---------------------------------------------------------------------- */

/* Allocate an empty filesystem with a single root directory. */
vfs_t *vfs_create(void);

/* Free all memory held by the filesystem (including any snapshot). */
void vfs_destroy(vfs_t *vfs);

/* -------------------------------------------------------------------------
 * Read-only operations
 * These are the only operations exposed through the FUSE layer.
 * ---------------------------------------------------------------------- */

/*
 * Fill *out with stat information for the node at path.
 * Returns 0 on success, -ENOENT if not found, -EINVAL for bad path.
 */
int vfs_getattr(vfs_t *vfs, const char *path, vfs_stat_t *out);

/*
 * Enumerate the directory at path by calling cb(ctx, name, stat) for
 * each entry, including "." and "..".
 * Returns 0 on success, -ENOENT / -ENOTDIR on error, or the first
 * non-zero return value from cb.
 */
int vfs_readdir(vfs_t *vfs, const char *path,
                vfs_readdir_cb_t cb, void *ctx);

/*
 * Copy up to size bytes from the file at path starting at offset into buf.
 * Sets *out_len to the number of bytes actually copied.
 * If offset >= file size, sets *out_len = 0 and returns 0 (not an error).
 * Returns 0 on success, negative errno on error.
 */
int vfs_read(vfs_t *vfs, const char *path, size_t offset, size_t size,
             uint8_t *buf, size_t *out_len);

/* -------------------------------------------------------------------------
 * Control-path mutating operations
 * These are only reachable through the fuzzer control path, not through FUSE.
 * ---------------------------------------------------------------------- */

/*
 * Create a new regular file at path with the given content.
 * The parent directory must already exist.
 * Returns -EEXIST if path already exists, -ENOENT if the parent does not.
 */
int vfs_create_file(vfs_t *vfs, const char *path,
                    const uint8_t *content, size_t content_len);

/*
 * Replace the content of the existing regular file at path.
 * Returns -ENOENT if path does not exist, -EISDIR if it is a directory.
 */
int vfs_update_file(vfs_t *vfs, const char *path,
                    const uint8_t *content, size_t content_len);

/*
 * Delete the regular file at path.
 * Returns -ENOENT if path does not exist, -EISDIR if it is a directory.
 */
int vfs_delete_file(vfs_t *vfs, const char *path);

/*
 * Create a directory at path. The parent must exist.
 * Returns -EEXIST if path already exists (as file or directory).
 */
int vfs_mkdir(vfs_t *vfs, const char *path);

/*
 * Delete the empty directory at path. Root cannot be removed.
 * Returns -ENOTEMPTY if the directory has children.
 */
int vfs_rmdir(vfs_t *vfs, const char *path);

/* -------------------------------------------------------------------------
 * Snapshot and reset
 * ---------------------------------------------------------------------- */

/*
 * Save a deep copy of the current filesystem state as the baseline snapshot.
 * Overwrites any previously saved snapshot.
 * Returns 0 on success, -ENOMEM on allocation failure.
 */
int vfs_save_snapshot(vfs_t *vfs);

/*
 * Replace the current filesystem state with the saved snapshot.
 * The snapshot itself is preserved; reset can be called repeatedly.
 * Returns -EINVAL if no snapshot has been saved, -ENOMEM on failure.
 */
int vfs_reset_to_snapshot(vfs_t *vfs);

#endif /* VFS_H */
