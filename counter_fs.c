#define FUSE_USE_VERSION 31
#include <fuse3/fuse.h>
#include <string.h>
#include <errno.h>
#include <stdio.h>
#include <stdint.h>

static uint64_t read_counter = 0;

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
        st->st_size = 32; // enough size ig
        return 0;
    }

    return -ENOENT;
}
static int counter_read(const char *path, char *buf, size_t size,
                         off_t offset, struct fuse_file_info *fi) {
    (void)fi;
    if (strcmp(path, "/counter") != 0)
        return -ENOENT;

    char content[32];
    int len = snprintf(content, sizeof(content), "%llu\n",
                       (unsigned long long)read_counter++);

    if (offset >= len)
        return 0;

    size_t to_copy = len - offset;
    if (to_copy > size) to_copy = size;
    memcpy(buf, content + offset, to_copy);
    return to_copy;
}

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
    .read    = counter_read,
    .readdir = counter_readdir,
};

int main(int argc, char *argv[]) {
    return fuse_main(argc, argv, &ops, NULL);
}