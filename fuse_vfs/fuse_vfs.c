#define FUSE_USE_VERSION 31

#include <fuse3/fuse.h>
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include "../vfs/vfs.h"

static vfs_t *g_vfs;

static void vfs_stat_to_stat(const vfs_stat_t *vs, struct stat *st)
{
    memset(st, 0, sizeof(*st));
    if (vs->kind == VFS_DIR) {
        st->st_mode  = S_IFDIR | 0755;
        st->st_nlink = 2;
    } else if (vs->kind == VFS_SYMLINK) {
        st->st_mode  = S_IFLNK | 0777;
        st->st_nlink = 1;
        st->st_size  = (off_t)vs->size;
    } else {
        st->st_mode  = S_IFREG | 0644;
        st->st_nlink = 1;
        st->st_size  = (off_t)vs->size;
    }
    st->st_mtim = vs->mtime;
    st->st_atim = vs->atime;
}

static int fvfs_getattr(const char *path, struct stat *st,
                        struct fuse_file_info *fi)
{
    (void)fi;
    vfs_stat_t vs;
    int r = vfs_getattr(g_vfs, path, &vs);
    if (r != 0) return r;
    vfs_stat_to_stat(&vs, st);
    return 0;
}

typedef struct {
    void            *buf;
    fuse_fill_dir_t  filler;
} readdir_ctx_t;

static int readdir_bridge(void *ctx, const char *name, const vfs_stat_t *vs)
{
    readdir_ctx_t *rc = ctx;
    struct stat st;
    vfs_stat_to_stat(vs, &st);
    /* filler returns 1 when the buffer is full; ignored because directories
     * are small and we use the non-offset readdir mode. */
    rc->filler(rc->buf, name, &st, 0, 0);
    return 0;
}

static int fvfs_readdir(const char *path, void *buf, fuse_fill_dir_t filler,
                        off_t offset, struct fuse_file_info *fi,
                        enum fuse_readdir_flags flags)
{
    (void)offset; (void)fi; (void)flags;
    readdir_ctx_t ctx = { buf, filler };
    return vfs_readdir(g_vfs, path, readdir_bridge, &ctx);
}

static void *fvfs_init(struct fuse_conn_info *conn, struct fuse_config *cfg)
{
    (void)conn;
    /* Zero all cache timeouts so every read/stat goes through FUSE.
     * Without this the kernel page cache serves stale bytes after apply_delta
     * mutates the VFS in-place — the target sees the same content every iteration. */
    cfg->attr_timeout     = 0;
    cfg->entry_timeout    = 0;
    cfg->negative_timeout = 0;
    return NULL;
}

static int fvfs_open(const char *path, struct fuse_file_info *fi)
{
    vfs_stat_t vs;
    int r = vfs_getattr(g_vfs, path, &vs);
    if (r != 0) return r;
    if (vs.kind == VFS_DIR) return -EISDIR;
    fi->direct_io = 1;  /* bypass page cache so each fread() hits fvfs_read() fresh */
    return 0;
}

static int fvfs_read(const char *path, char *buf, size_t size, off_t offset,
                     struct fuse_file_info *fi)
{
    (void)fi;
    size_t got = 0;
    int r = vfs_read(g_vfs, path, (size_t)offset, size, (uint8_t *)buf, &got);
    if (r != 0) return r;
    return (int)got;
}

static int fvfs_create(const char *path, mode_t mode, struct fuse_file_info *fi)
{
    (void)mode; (void)fi;
    return vfs_create_file(g_vfs, path, NULL, 0);
}

static int fvfs_write(const char *path, const char *buf, size_t size,
                      off_t offset, struct fuse_file_info *fi)
{
    (void)fi;
    vfs_stat_t vs;
    int r = vfs_getattr(g_vfs, path, &vs);
    if (r != 0) return r;
    if (vs.kind == VFS_DIR) return -EISDIR;

    size_t off    = (size_t)offset;
    size_t newlen = (off + size > vs.size) ? (off + size) : vs.size;

    uint8_t *tmp = calloc(1, newlen > 0 ? newlen : 1);
    if (!tmp) return -ENOMEM;

    /* Preserve existing bytes. */
    if (vs.size > 0) {
        size_t got;
        vfs_read(g_vfs, path, 0, vs.size, tmp, &got);
    }

    /* Overlay the new bytes at offset. */
    memcpy(tmp + off, buf, size);

    r = vfs_update_file(g_vfs, path, tmp, newlen);
    free(tmp);
    return (r == 0) ? (int)size : r;
}

static int fvfs_truncate(const char *path, off_t newsize,
                         struct fuse_file_info *fi)
{
    (void)fi;
    vfs_stat_t vs;
    int r = vfs_getattr(g_vfs, path, &vs);
    if (r != 0) return r;
    if (vs.kind == VFS_DIR) return -EISDIR;

    size_t sz  = (size_t)newsize;
    uint8_t *tmp = NULL;
    if (sz > 0) {
        tmp = calloc(1, sz);
        if (!tmp) return -ENOMEM;
        size_t copy = (vs.size < sz) ? vs.size : sz;
        if (copy > 0) {
            size_t got;
            vfs_read(g_vfs, path, 0, copy, tmp, &got);
        }
    }

    r = vfs_update_file(g_vfs, path, tmp, sz);
    free(tmp);
    return r;
}

static int fvfs_mkdir(const char *path, mode_t mode)
{
    (void)mode;
    return vfs_mkdir(g_vfs, path);
}

static int fvfs_unlink(const char *path)
{
    return vfs_delete_file(g_vfs, path);
}

static int fvfs_rmdir(const char *path)
{
    return vfs_rmdir(g_vfs, path);
}

static int fvfs_rename(const char *oldpath, const char *newpath,
                       unsigned int flags)
{
    (void)flags;
    return vfs_rename(g_vfs, oldpath, newpath);
}

/* FUSE argument order is (target, linkpath) — opposite of the intuitive order.
 * vfs_symlink takes (vfs, linkpath, target). */
static int fvfs_symlink(const char *target, const char *linkpath)
{
    return vfs_symlink(g_vfs, linkpath, target);
}

static int fvfs_readlink(const char *path, char *buf, size_t size)
{
    int r = vfs_readlink(g_vfs, path, buf, size);
    return (r >= 0) ? 0 : r;
}

static int fvfs_utimens(const char *path, const struct timespec tv[2],
                        struct fuse_file_info *fi)
{
    (void)fi;
    struct timespec now;
    clock_gettime(CLOCK_REALTIME, &now);

    struct timespec atime = (tv[0].tv_nsec == UTIME_NOW)  ? now : tv[0];
    struct timespec mtime = (tv[1].tv_nsec == UTIME_NOW)  ? now : tv[1];

    const struct timespec *ap = (tv[0].tv_nsec == UTIME_OMIT) ? NULL : &atime;
    const struct timespec *mp = (tv[1].tv_nsec == UTIME_OMIT) ? NULL : &mtime;

    return vfs_set_times(g_vfs, path, mp, ap);
}

static const struct fuse_operations fvfs_ops = {
    .init     = fvfs_init,
    /* read path */
    .getattr  = fvfs_getattr,
    .readdir  = fvfs_readdir,
    .open     = fvfs_open,
    .read     = fvfs_read,
    .readlink = fvfs_readlink,
    /* write path */
    .create   = fvfs_create,
    .write    = fvfs_write,
    .truncate = fvfs_truncate,
    .mkdir    = fvfs_mkdir,
    .unlink   = fvfs_unlink,
    .rmdir    = fvfs_rmdir,
    .rename   = fvfs_rename,
    .symlink  = fvfs_symlink,
    .utimens  = fvfs_utimens,
};

static struct fuse   *g_fuse_handle = NULL;
static volatile int   g_mounted     = 0;

void fuse_vfs_lib_init(vfs_t *vfs)
{
    g_vfs = vfs;
}

int fuse_vfs_lib_run(const char *mountpoint)
{
    char *argv0 = "fvfs_lib";
    struct fuse_args args = FUSE_ARGS_INIT(1, &argv0);

    g_fuse_handle = fuse_new(&args, &fvfs_ops, sizeof(fvfs_ops), NULL);
    fuse_opt_free_args(&args);
    if (!g_fuse_handle) {
        fprintf(stderr, "fuse_vfs_lib: fuse_new failed\n");
        return -1;
    }

    if (fuse_mount(g_fuse_handle, mountpoint) != 0) {
        fprintf(stderr, "fuse_vfs_lib: fuse_mount failed on %s\n", mountpoint);
        fuse_destroy(g_fuse_handle);
        g_fuse_handle = NULL;
        return -1;
    }

    g_mounted = 1;
    int ret = fuse_loop(g_fuse_handle);
    g_mounted = 0;

    fuse_unmount(g_fuse_handle);
    fuse_destroy(g_fuse_handle);
    g_fuse_handle = NULL;
    return ret;
}

int fuse_vfs_lib_is_mounted(void)
{
    return g_mounted;
}

void fuse_vfs_lib_stop(void)
{
    if (g_fuse_handle && g_mounted)
        fuse_exit(g_fuse_handle);
}

#ifndef FUSE_VFS_LIBRARY

static void populate_vfs(void)
{
    vfs_create_file(g_vfs, "/counter",
                    (const uint8_t *)"0\n", 2);

    vfs_mkdir(g_vfs, "/docs");
    vfs_create_file(g_vfs, "/docs/readme.txt",
                    (const uint8_t *)"fuse_vfs: VFS-backed read-only FUSE mount.\n", 43);

    vfs_mkdir(g_vfs, "/data");
    vfs_create_file(g_vfs, "/data/sample.txt",
                    (const uint8_t *)"hello world\n", 12);
    vfs_create_file(g_vfs, "/data/binary.bin",
                    (const uint8_t *)"\x00\x01\x02\x03\xff\xfe", 6);
}

int main(int argc, char *argv[])
{
    g_vfs = vfs_create();
    if (!g_vfs) {
        fprintf(stderr, "fuse_vfs: vfs_create failed\n");
        return 1;
    }
    populate_vfs();

    int ret = fuse_main(argc, argv, &fvfs_ops, NULL);
    vfs_destroy(g_vfs);
    return ret;
}

#endif /* FUSE_VFS_LIBRARY */
