#ifndef VFS_H
#define VFS_H

#include <stddef.h>
#include <stdint.h>
#include <time.h>

typedef enum {
    VFS_FILE,
    VFS_DIR,
    VFS_SYMLINK,
} vfs_kind_t;

typedef struct vfs_dirent vfs_dirent_t;
typedef struct vfs_node   vfs_node_t;

struct vfs_dirent {
    char         *name;
    vfs_node_t   *node;
    vfs_dirent_t *next;
};

struct vfs_node {
    uint64_t        ino;
    vfs_kind_t      kind;
    uint8_t        *content;      /* VFS_FILE only */
    size_t          content_len;  /* VFS_FILE only */
    vfs_dirent_t   *children;     /* VFS_DIR only  */
    char           *link_target;  /* VFS_SYMLINK only */
    vfs_node_t     *parent;       /* NULL for root */
    struct timespec mtime;
    struct timespec atime;
};

typedef struct {
    vfs_node_t *root;
    vfs_node_t *snapshot;
    uint64_t    next_ino;
} vfs_t;

typedef struct {
    uint64_t        ino;
    vfs_kind_t      kind;
    size_t          size;
    struct timespec mtime;
    struct timespec atime;
} vfs_stat_t;

/* return 0 to continue, non-zero to stop early */
typedef int (*vfs_readdir_cb_t)(void *ctx, const char *name,
                                const vfs_stat_t *st);

vfs_t *vfs_create(void);
void vfs_destroy(vfs_t *vfs);

int vfs_getattr(vfs_t *vfs, const char *path, vfs_stat_t *out);
int vfs_readdir(vfs_t *vfs, const char *path,
                vfs_readdir_cb_t cb, void *ctx);
int vfs_read(vfs_t *vfs, const char *path, size_t offset, size_t size,
             uint8_t *buf, size_t *out_len);

int vfs_create_file(vfs_t *vfs, const char *path,
                    const uint8_t *content, size_t content_len);
int vfs_update_file(vfs_t *vfs, const char *path,
                    const uint8_t *content, size_t content_len);
int vfs_delete_file(vfs_t *vfs, const char *path);
int vfs_mkdir(vfs_t *vfs, const char *path);
int vfs_rmdir(vfs_t *vfs, const char *path);
int vfs_rename(vfs_t *vfs, const char *oldpath, const char *newpath);
int vfs_symlink(vfs_t *vfs, const char *path, const char *target);
int vfs_readlink(vfs_t *vfs, const char *path, char *buf, size_t bufsz);
int vfs_set_times(vfs_t *vfs, const char *path,
                  const struct timespec *mtime,
                  const struct timespec *atime);

int vfs_save_snapshot(vfs_t *vfs);
int vfs_reset_to_snapshot(vfs_t *vfs);

#endif /* VFS_H */
