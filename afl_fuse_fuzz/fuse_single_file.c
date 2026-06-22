/*
 * Minimal FUSE3 fs with one file "input" in a fixed in-memory buffer.
 * Mount:   ./fuse_single_file <mountpoint> [-s]
 * Unmount: fusermount3 -u <mountpoint>
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

#define INPUT_FILENAME "input"
#define MAX_FILE_SIZE  65536

static uint8_t g_buf[MAX_FILE_SIZE];
static size_t  g_size = 0;

static int sf_getattr(const char *path, struct stat *st,
                      struct fuse_file_info *fi)
{
    (void)fi;
    memset(st, 0, sizeof(*st));

    if (strcmp(path, "/") == 0) {
        st->st_mode  = S_IFDIR | 0755;
        st->st_nlink = 2;
        return 0;
    }

    if (strcmp(path, "/" INPUT_FILENAME) == 0) {
        st->st_mode  = S_IFREG | 0666;
        st->st_nlink = 1;
        st->st_size  = (off_t)g_size;
        return 0;
    }

    return -ENOENT;
}

static int sf_readdir(const char *path, void *buf, fuse_fill_dir_t filler,
                      off_t offset, struct fuse_file_info *fi,
                      enum fuse_readdir_flags flags)
{
    (void)offset; (void)fi; (void)flags;
    if (strcmp(path, "/") != 0)
        return -ENOENT;

    filler(buf, ".",            NULL, 0, 0);
    filler(buf, "..",           NULL, 0, 0);
    filler(buf, INPUT_FILENAME, NULL, 0, 0);
    return 0;
}

static int sf_open(const char *path, struct fuse_file_info *fi)
{
    (void)fi;
    if (strcmp(path, "/" INPUT_FILENAME) != 0)
        return -ENOENT;
    return 0;
}

static int sf_read(const char *path, char *buf, size_t size, off_t offset,
                   struct fuse_file_info *fi)
{
    (void)fi;
    if (strcmp(path, "/" INPUT_FILENAME) != 0)
        return -ENOENT;

    if ((size_t)offset >= g_size)
        return 0;
    if ((size_t)offset + size > g_size)
        size = g_size - (size_t)offset;

    memcpy(buf, g_buf + offset, size);
    return (int)size;
}

static int sf_write(const char *path, const char *buf, size_t size, off_t offset,
                    struct fuse_file_info *fi)
{
    (void)fi;
    if (strcmp(path, "/" INPUT_FILENAME) != 0)
        return -ENOENT;

    if ((size_t)offset + size > MAX_FILE_SIZE)
        size = MAX_FILE_SIZE - (size_t)offset;

    memcpy(g_buf + offset, buf, size);
    /* cap g_size to written extent so a short write after a long one drops stale tail */
    g_size = (size_t)offset + size;

    return (int)size;
}

static int sf_truncate(const char *path, off_t size,
                       struct fuse_file_info *fi)
{
    (void)fi;
    if (strcmp(path, "/" INPUT_FILENAME) != 0)
        return -ENOENT;
    if ((size_t)size > MAX_FILE_SIZE)
        return -EFBIG;

    if ((size_t)size > g_size)
        memset(g_buf + g_size, 0, (size_t)size - g_size);

    g_size = (size_t)size;
    return 0;
}

static const struct fuse_operations sf_ops = {
    .getattr  = sf_getattr,
    .readdir  = sf_readdir,
    .open     = sf_open,
    .read     = sf_read,
    .write    = sf_write,
    .truncate = sf_truncate,
};

int main(int argc, char *argv[])
{
    return fuse_main(argc, argv, &sf_ops, NULL);
}
