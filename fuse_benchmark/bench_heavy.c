#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <time.h>
#include <sys/stat.h>
#include <errno.h>

#define FILE_COUNT  2000
#define FILE_SIZE   4096

static char g_payload[FILE_SIZE];

static long now_ms(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long)(ts.tv_sec * 1000 + ts.tv_nsec / 1000000);
}

static void die(const char *msg)
{
    perror(msg);
    exit(1);
}

int main(int argc, char *argv[])
{
    if (argc != 2) {
        fprintf(stderr, "usage: %s <directory>\n", argv[0]);
        return 1;
    }

    const char *dir = argv[1];
    memset(g_payload, 0xAB, FILE_SIZE);

    char path[512], newpath[512];
    long t0, t1, total_start;

    total_start = now_ms();

    t0 = now_ms();
    for (int i = 0; i < FILE_COUNT; i++) {
        snprintf(path, sizeof(path), "%s/file_%04d.dat", dir, i);
        int fd = open(path, O_CREAT | O_WRONLY | O_TRUNC, 0644);
        if (fd < 0) die("open(create)");
        close(fd);
    }
    t1 = now_ms();
    printf("  create  %d files:         %4ld ms\n", FILE_COUNT, t1 - t0);

    t0 = now_ms();
    for (int i = 0; i < FILE_COUNT; i++) {
        snprintf(path, sizeof(path), "%s/file_%04d.dat", dir, i);
        int fd = open(path, O_WRONLY);
        if (fd < 0) die("open(write)");
        ssize_t w = write(fd, g_payload, FILE_SIZE);
        if (w != FILE_SIZE) die("write");
        close(fd);
    }
    t1 = now_ms();
    printf("  write   %d x %d B:    %4ld ms\n", FILE_COUNT, FILE_SIZE, t1 - t0);

    char *rbuf = malloc(FILE_SIZE);
    if (!rbuf) die("malloc");

    t0 = now_ms();
    for (int i = 0; i < FILE_COUNT; i++) {
        snprintf(path, sizeof(path), "%s/file_%04d.dat", dir, i);
        int fd = open(path, O_RDONLY);
        if (fd < 0) die("open(read)");
        ssize_t r = read(fd, rbuf, FILE_SIZE);
        if (r < 0) die("read");
        close(fd);
    }
    t1 = now_ms();
    free(rbuf);
    printf("  read    %d files:         %4ld ms\n", FILE_COUNT, t1 - t0);

    t0 = now_ms();
    for (int i = 0; i < FILE_COUNT; i++) {
        snprintf(path,    sizeof(path),    "%s/file_%04d.dat",    dir, i);
        snprintf(newpath, sizeof(newpath), "%s/renamed_%04d.dat", dir, i);
        if (rename(path, newpath) < 0) die("rename");
    }
    t1 = now_ms();
    printf("  rename  %d files:         %4ld ms\n", FILE_COUNT, t1 - t0);

    t0 = now_ms();
    for (int i = 0; i < FILE_COUNT; i++) {
        snprintf(path, sizeof(path), "%s/renamed_%04d.dat", dir, i);
        if (unlink(path) < 0) die("unlink");
    }
    t1 = now_ms();
    printf("  delete  %d files:         %4ld ms\n", FILE_COUNT, t1 - t0);

    printf("  ─────────────────────────────────────\n");
    printf("  total                     %4ld ms\n", t1 - total_start);

    printf("\n  [sustained 60s] create→write→read→rename→delete ...\n");

    snprintf(path,    sizeof(path),    "%s/sustained.dat",         dir);
    snprintf(newpath, sizeof(newpath), "%s/sustained_renamed.dat", dir);

    long count = 0;
    long sustained_start = now_ms();

    while (now_ms() - sustained_start < 60000) {
        int fd = open(path, O_CREAT | O_WRONLY | O_TRUNC, 0644);
        if (fd < 0) die("sustained open(create)");
        if (write(fd, g_payload, FILE_SIZE) != FILE_SIZE) die("sustained write");
        close(fd);
        fd = open(path, O_RDONLY);
        if (fd < 0) die("sustained open(read)");
        char tmp[FILE_SIZE];
        if (read(fd, tmp, FILE_SIZE) < 0) die("sustained read");
        close(fd);
        if (rename(path, newpath) < 0) die("sustained rename");
        if (unlink(newpath) < 0) die("sustained unlink");

        count++;
    }

    printf("  sustained cycles/sec:     %.2f\n", count / 60.0);

    return 0;
}
