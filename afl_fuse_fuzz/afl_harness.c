#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#ifndef FUSE_INPUT_PATH
#define FUSE_INPUT_PATH "./mnt/input"
#endif

void fuzz_foobar_from_path(const char *path);

#ifndef __AFL_FUZZ_TESTCASE_LEN
static ssize_t       __afl_len;
static unsigned char __afl_buf[1 << 20];
#define __AFL_FUZZ_TESTCASE_LEN  __afl_len
#define __AFL_FUZZ_TESTCASE_BUF  __afl_buf
#define __AFL_FUZZ_INIT()        /* nothing */
#define __AFL_INIT()             /* nothing */
#define __AFL_LOOP(n)            ((__afl_len = read(STDIN_FILENO, __afl_buf, \
                                   sizeof(__afl_buf))) > 0)
#endif

__AFL_FUZZ_INIT();

static void write_to_fuse(const unsigned char *data, size_t len)
{
    int fd = open(FUSE_INPUT_PATH, O_WRONLY);
    if (fd < 0) {
        fprintf(stderr, "write_to_fuse open(%s): %s\n",
                FUSE_INPUT_PATH, strerror(errno));
        return;
    }
    /* ftruncate instead of O_TRUNC: FUSE3 doesn't always call truncate for O_TRUNC,
     * leaving g_size stale and making short inputs inherit bytes from longer ones. */
    if (ftruncate(fd, 0) < 0)
        fprintf(stderr, "write_to_fuse ftruncate: %s\n", strerror(errno));
    ssize_t w = write(fd, data, len);
    (void)w;
    close(fd);
}

int main(void)
{
    __AFL_INIT();

    unsigned char *buf = __AFL_FUZZ_TESTCASE_BUF;

    while (__AFL_LOOP(10000)) {
        size_t len = (size_t)__AFL_FUZZ_TESTCASE_LEN;
        if (len == 0)
            continue;

        write_to_fuse(buf, len);
        fuzz_foobar_from_path(FUSE_INPUT_PATH);
    }

    return 0;
}
