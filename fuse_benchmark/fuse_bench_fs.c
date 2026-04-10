#define FUSE_USE_VERSION 31

#include <fuse3/fuse.h>
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/statvfs.h>

#define MAX_NODES 8192
#define MAX_PATH  512

typedef struct {
    char     path[MAX_PATH];
    uint8_t *content;
    size_t   size;
    int      is_dir;
    int      used;
} node_t;

static node_t g_nodes[MAX_NODES];

static node_t *node_find(const char *path)
{
    for (int i = 0; i < MAX_NODES; i++)
        if (g_nodes[i].used && strcmp(g_nodes[i].path, path) == 0)
            return &g_nodes[i];
    return NULL;
}

static node_t *node_alloc(void)
{
    for (int i = 0; i < MAX_NODES; i++)
        if (!g_nodes[i].used) return &g_nodes[i];
    return NULL;
}

static void node_free(node_t *n)
{
    free(n->content);
    memset(n, 0, sizeof(*n));
}

/*
 * Return the parent directory path of a given path.
 * "/foo/bar" → "/foo",  "/foo" → "/"
 * Result written into buf (must be MAX_PATH bytes).
 */
static void parent_of(const char *path, char *buf)
{
    const char *last = strrchr(path, '/');
    if (!last || last == path) {
        buf[0] = '/'; buf[1] = '\0';
    } else {
        size_t len = (size_t)(last - path);
        memcpy(buf, path, len);
        buf[len] = '\0';
    }
}

// callbacks

static int fbfs_getattr(const char *path, struct stat *st,
                        struct fuse_file_info *fi)
{
    (void)fi;
    memset(st, 0, sizeof(*st));

    if (strcmp(path, "/") == 0) {
        st->st_mode  = S_IFDIR | 0755;
        st->st_nlink = 2;
        return 0;
    }

    node_t *n = node_find(path);
    if (!n) return -ENOENT;

    if (n->is_dir) {
        st->st_mode  = S_IFDIR | 0755;
        st->st_nlink = 2;
    } else {
        st->st_mode  = S_IFREG | 0644;
        st->st_nlink = 1;
        st->st_size  = (off_t)n->size;
    }
    return 0;
}

static int fbfs_readdir(const char *path, void *buf, fuse_fill_dir_t filler,
                        off_t offset, struct fuse_file_info *fi,
                        enum fuse_readdir_flags flags)
{
    (void)offset; (void)fi; (void)flags;

    filler(buf, ".",  NULL, 0, 0);
    filler(buf, "..", NULL, 0, 0);

    for (int i = 0; i < MAX_NODES; i++) {
        if (!g_nodes[i].used) continue;

        char parent[MAX_PATH];
        parent_of(g_nodes[i].path, parent);
        if (strcmp(parent, path) != 0) continue;

        /* Emit only the final component. */
        const char *name = strrchr(g_nodes[i].path, '/') + 1;
        filler(buf, name, NULL, 0, 0);
    }
    return 0;
}

static int fbfs_open(const char *path, struct fuse_file_info *fi)
{
    (void)fi;
    node_t *n = node_find(path);
    if (!n) return -ENOENT;
    if (n->is_dir) return -EISDIR;
    return 0;
}

static int fbfs_read(const char *path, char *buf, size_t size, off_t offset,
                     struct fuse_file_info *fi)
{
    (void)fi;
    node_t *n = node_find(path);
    if (!n) return -ENOENT;
    if (n->is_dir) return -EISDIR;

    if ((size_t)offset >= n->size) return 0;
    size_t avail = n->size - (size_t)offset;
    size_t to_copy = size < avail ? size : avail;
    memcpy(buf, n->content + offset, to_copy);
    return (int)to_copy;
}

static int fbfs_write(const char *path, const char *buf, size_t size,
                      off_t offset, struct fuse_file_info *fi)
{
    (void)fi;
    node_t *n = node_find(path);
    if (!n) return -ENOENT;
    if (n->is_dir) return -EISDIR;

    size_t new_size = n->size;
    if ((size_t)offset + size > new_size)
        new_size = (size_t)offset + size;

    uint8_t *tmp = realloc(n->content, new_size);
    if (!tmp) return -ENOMEM;

    /* Zero-fill any gap between old end and new offset. */
    if ((size_t)offset > n->size)
        memset(tmp + n->size, 0, (size_t)offset - n->size);

    memcpy(tmp + offset, buf, size);
    n->content = tmp;
    n->size    = new_size;
    return (int)size;
}

static int fbfs_create(const char *path, mode_t mode,
                       struct fuse_file_info *fi)
{
    (void)mode; (void)fi;
    if (node_find(path)) return -EEXIST;

    node_t *n = node_alloc();
    if (!n) return -ENOSPC;

    strncpy(n->path, path, MAX_PATH - 1);
    n->used = 1;
    return 0;
}

static int fbfs_truncate(const char *path, off_t size,
                         struct fuse_file_info *fi)
{
    (void)fi;
    node_t *n = node_find(path);
    if (!n) return -ENOENT;
    if (n->is_dir) return -EISDIR;

    size_t new_size = (size_t)size;
    uint8_t *tmp = realloc(n->content, new_size ? new_size : 1);
    if (!tmp && new_size) return -ENOMEM;

    if (new_size > n->size)
        memset(tmp + n->size, 0, new_size - n->size);

    n->content = new_size ? tmp : NULL;
    n->size    = new_size;
    if (!new_size) free(tmp);
    return 0;
}

static int fbfs_mkdir(const char *path, mode_t mode)
{
    (void)mode;
    if (node_find(path)) return -EEXIST;

    node_t *n = node_alloc();
    if (!n) return -ENOSPC;

    strncpy(n->path, path, MAX_PATH - 1);
    n->is_dir = 1;
    n->used   = 1;
    return 0;
}

static int fbfs_unlink(const char *path)
{
    node_t *n = node_find(path);
    if (!n) return -ENOENT;
    if (n->is_dir) return -EISDIR;
    node_free(n);
    return 0;
}

static int fbfs_rmdir(const char *path)
{
    node_t *n = node_find(path);
    if (!n) return -ENOENT;
    if (!n->is_dir) return -ENOTDIR;

    /* Refuse if any node has this path as its parent. */
    for (int i = 0; i < MAX_NODES; i++) {
        if (!g_nodes[i].used || &g_nodes[i] == n) continue;
        char parent[MAX_PATH];
        parent_of(g_nodes[i].path, parent);
        if (strcmp(parent, path) == 0) return -ENOTEMPTY;
    }

    node_free(n);
    return 0;
}

static int fbfs_rename(const char *from, const char *to, unsigned int flags)
{
    (void)flags;
    node_t *src = node_find(from);
    if (!src) return -ENOENT;

    /* Remove destination if it exists. */
    node_t *dst = node_find(to);
    if (dst) node_free(dst);

    strncpy(src->path, to, MAX_PATH - 1);
    return 0;
}

static int fbfs_statfs(const char *path, struct statvfs *st)
{
    (void)path;
    memset(st, 0, sizeof(*st));
    st->f_bsize  = 4096;
    st->f_frsize = 4096;
    st->f_blocks = 1024UL * 1024;
    st->f_bfree  = 1024UL * 1024;
    st->f_bavail = 1024UL * 1024;
    st->f_files  = MAX_NODES;
    st->f_ffree  = MAX_NODES;
    return 0;
}

static int fbfs_access(const char *path, int mask)
{
    (void)mask;
    if (strcmp(path, "/") == 0) return 0;
    return node_find(path) ? 0 : -ENOENT;
}

static int fbfs_flush(const char *path, struct fuse_file_info *fi)
{
    (void)path; (void)fi; return 0;
}

static int fbfs_release(const char *path, struct fuse_file_info *fi)
{
    (void)path; (void)fi; return 0;
}

static int fbfs_utimens(const char *path, const struct timespec tv[2],
                        struct fuse_file_info *fi)
{
    (void)path; (void)tv; (void)fi; return 0;
}

static int fbfs_chmod(const char *path, mode_t mode, struct fuse_file_info *fi)
{
    (void)path; (void)mode; (void)fi; return 0;
}

static const struct fuse_operations fbfs_ops = {
    .getattr  = fbfs_getattr,
    .readdir  = fbfs_readdir,
    .open     = fbfs_open,
    .read     = fbfs_read,
    .write    = fbfs_write,
    .create   = fbfs_create,
    .truncate = fbfs_truncate,
    .mkdir    = fbfs_mkdir,
    .unlink   = fbfs_unlink,
    .rmdir    = fbfs_rmdir,
    .rename   = fbfs_rename,
    .statfs   = fbfs_statfs,
    .access   = fbfs_access,
    .flush    = fbfs_flush,
    .release  = fbfs_release,
    .utimens  = fbfs_utimens,
    .chmod    = fbfs_chmod,
};

int main(int argc, char *argv[])
{
    return fuse_main(argc, argv, &fbfs_ops, NULL);
}
