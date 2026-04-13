/*
 * fuse_vfs.c — FUSE frontend backed by the in-memory VFS core.
 *
 * Exposes a read-write FUSE mount backed entirely by the in-memory VFS.
 * No data is ever written to disk; all state lives in heap memory.
 *
 * Supported operations:
 *   Read:   getattr, readdir, open, read
 *   Write:  create, write, truncate, mkdir, unlink, rmdir
 *
 * Write semantics:
 *   write()    — read-modify-write on the VFS content buffer; handles
 *                partial writes and appends (gaps filled with zeros).
 *   truncate() — shrink or extend the content buffer (zeros on extension).
 *   create()   — creates an empty file; FUSE calls this for O_CREAT on new paths.
 *
 * Build:    make            (inside fuse_vfs/)
 * Mount:    ./fuse_vfs <mountpoint>
 * Unmount:  fusermount3 -u <mountpoint>
 * Test:     make test
 */

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

/* Global VFS instance.  Initialised in main() before fuse_main() runs. */
static vfs_t *g_vfs;

/* -------------------------------------------------------------------------
 * Stat conversion helper
 * ---------------------------------------------------------------------- */

static void vfs_stat_to_stat(const vfs_stat_t *vs, struct stat *st)
{
    memset(st, 0, sizeof(*st));
    if (vs->kind == VFS_DIR) {
        st->st_mode  = S_IFDIR | 0755;
        st->st_nlink = 2;
    } else if (vs->kind == VFS_SYMLINK) {
        st->st_mode  = S_IFLNK | 0777;
        st->st_nlink = 1;
        st->st_size  = (off_t)vs->size;   /* strlen(target) */
    } else {
        st->st_mode  = S_IFREG | 0644;
        st->st_nlink = 1;
        st->st_size  = (off_t)vs->size;
    }
    st->st_mtim = vs->mtime;
    st->st_atim = vs->atime;
}

/* -------------------------------------------------------------------------
 * FUSE callbacks
 * ---------------------------------------------------------------------- */

static int fvfs_getattr(const char *path, struct stat *st,
                        struct fuse_file_info *fi)
{
    (void)fi;
    vfs_stat_t vs;
    int r = vfs_getattr(g_vfs, path, &vs);
    if (r != 0) return r;   /* already a negative errno */
    vfs_stat_to_stat(&vs, st);
    return 0;
}

/*
 * Bridge: adapts the vfs_readdir_cb_t signature to a FUSE filler call.
 * Passed as the callback to vfs_readdir(); ctx is a readdir_ctx_t *.
 */
typedef struct {
    void            *buf;
    fuse_fill_dir_t  filler;
} readdir_ctx_t;

static int readdir_bridge(void *ctx, const char *name, const vfs_stat_t *vs)
{
    readdir_ctx_t *rc = ctx;
    struct stat st;
    vfs_stat_to_stat(vs, &st);
    /* filler returns 1 when the buffer is full; we ignore it because our
     * directories are small and we use the non-offset readdir mode. */
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

static int fvfs_open(const char *path, struct fuse_file_info *fi)
{
    (void)fi;
    vfs_stat_t vs;
    int r = vfs_getattr(g_vfs, path, &vs);
    if (r != 0) return r;
    if (vs.kind == VFS_DIR) return -EISDIR;
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

/* -------------------------------------------------------------------------
 * Write-path callbacks
 * ---------------------------------------------------------------------- */

/*
 * Create a new empty file.  FUSE calls this (not open) when O_CREAT is used
 * on a path that does not yet exist.
 */
static int fvfs_create(const char *path, mode_t mode, struct fuse_file_info *fi)
{
    (void)mode; (void)fi;
    return vfs_create_file(g_vfs, path, NULL, 0);
}

/*
 * Write size bytes from buf into the file at offset.
 *
 * The VFS stores content as a single flat buffer, so this is a
 * read-modify-write: read the current content, overlay the new bytes,
 * then call vfs_update_file with the merged result.  Gaps introduced by
 * writes past the end of the current content are zero-filled.
 */
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

/*
 * Truncate or extend the file to exactly newsize bytes.
 * Bytes beyond the old size are zero-filled on extension.
 */
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

/*
 * rename: FUSE passes (oldpath, newpath, flags).
 * flags (RENAME_NOREPLACE, RENAME_EXCHANGE) are not supported; we ignore them
 * and fall through to standard POSIX replace semantics.
 */
static int fvfs_rename(const char *oldpath, const char *newpath,
                       unsigned int flags)
{
    (void)flags;
    return vfs_rename(g_vfs, oldpath, newpath);
}

/*
 * symlink: note that FUSE's argument order is (target, linkpath) —
 * the reverse of the intuitive order.  vfs_symlink takes (vfs, linkpath, target).
 */
static int fvfs_symlink(const char *target, const char *linkpath)
{
    return vfs_symlink(g_vfs, linkpath, target);
}

static int fvfs_readlink(const char *path, char *buf, size_t size)
{
    int r = vfs_readlink(g_vfs, path, buf, size);
    return (r >= 0) ? 0 : r;
}

/*
 * Set atime (tv[0]) and mtime (tv[1]) on the node at path.
 * Handles UTIME_NOW (set to current time) and UTIME_OMIT (leave unchanged)
 * as required by POSIX utimensat(2).
 */
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

/* -------------------------------------------------------------------------
 * Initial filesystem layout
 *
 * Populated once in main() before FUSE takes control.  The fuzzer control
 * path (Week 5) will replace this with dynamic mutation batches.
 * ---------------------------------------------------------------------- */

static void populate_vfs(void)
{
    /*
     * /counter — the benchmark (../benchmark.c) opens this path for 60 s.
     * Content is a static string; throughput, not value, is what we measure.
     */
    vfs_create_file(g_vfs, "/counter",
                    (const uint8_t *)"0\n", 2);

    /* A small directory tree used by the integration test and manual checks. */
    vfs_mkdir(g_vfs, "/docs");
    vfs_create_file(g_vfs, "/docs/readme.txt",
                    (const uint8_t *)"fuse_vfs: VFS-backed read-only FUSE mount.\n", 43);

    vfs_mkdir(g_vfs, "/data");
    vfs_create_file(g_vfs, "/data/sample.txt",
                    (const uint8_t *)"hello world\n", 12);
    /* A file with non-text bytes to confirm binary read works. */
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