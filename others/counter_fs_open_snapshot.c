#define FUSE_USE_VERSION 31
#include <fuse3/fuse.h>
#include <string.h>
#include <errno.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <fcntl.h>

// This version increments once per open and keeps a stable snapshot per handle.
static uint64_t read_counter = 0;

struct counter_file_handle {
    char content[32];
    int len;
};

static int counter_getattr(const char *path, struct stat *st,
                            struct fuse_file_info *fi) {
    (void)fi;
    memset(st, 0, sizeof(struct stat));

    if (strcmp(path, "/") == 0) {
        st->st_mode = S_IFDIR | 0755;
        st->st_nlink = 2;
        return 0;
    }

    if (strcmp(path, "/counter") == 0) {
        st->st_mode = S_IFREG | 0444;
        st->st_nlink = 1;
        st->st_size = 32; // enough for a number as string
        return 0;
    }

    return -ENOENT;
}

// Called when someone opens a file
static int counter_open(const char *path, struct fuse_file_info *fi) {
    if (strcmp(path, "/counter") != 0)
        return -ENOENT;

    if ((fi->flags & O_ACCMODE) != O_RDONLY)
        return -EACCES;

    struct counter_file_handle *handle = malloc(sizeof(*handle));
    if (handle == NULL)
        return -ENOMEM;

    handle->len = snprintf(handle->content, sizeof(handle->content), "%llu\n",
                           (unsigned long long)read_counter++);
    fi->fh = (uint64_t)handle;
    return 0;
}

// Called when someone reads a file
static int counter_read(const char *path, char *buf, size_t size,
                         off_t offset, struct fuse_file_info *fi) {
    if (strcmp(path, "/counter") != 0)
        return -ENOENT;

    struct counter_file_handle *handle = (struct counter_file_handle *)fi->fh;
    if (handle == NULL)
        return -EIO;

    if (offset >= handle->len)
        return 0;

    size_t to_copy = handle->len - offset;
    if (to_copy > size) to_copy = size;
    memcpy(buf, handle->content + offset, to_copy);
    return to_copy;
}

// Called when someone closes a file
static int counter_release(const char *path, struct fuse_file_info *fi) {
    (void)path;
    free((void *)fi->fh);
    fi->fh = 0;
    return 0;
}

// Called when someone lists a directory
static int counter_readdir(const char *path, void *buf, fuse_fill_dir_t filler,
                            off_t offset, struct fuse_file_info *fi,
                            enum fuse_readdir_flags flags) {
    (void)offset;
    (void)fi;
    (void)flags;
    if (strcmp(path, "/") != 0)
        return -ENOENT;

    filler(buf, ".", NULL, 0, 0);
    filler(buf, "..", NULL, 0, 0);
    filler(buf, "counter", NULL, 0, 0);
    return 0;
}

static struct fuse_operations ops = {
    .getattr = counter_getattr,
    .open    = counter_open,
    .read    = counter_read,
    .release = counter_release,
    .readdir = counter_readdir,
};

int main(int argc, char *argv[]) {
    return fuse_main(argc, argv, &ops, NULL);
}
