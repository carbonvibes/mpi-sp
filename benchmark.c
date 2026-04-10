// benchmark.c for fuse counter filesystem
#include <stdio.h>
#include <time.h>
#include <fcntl.h>
#include <unistd.h>

int main() {
    char buf[32];
    long count = 0;
    time_t start = time(NULL);

    while (time(NULL) - start < 60) {
        int fd = open("/tmp/testmount/counter", O_RDONLY);
        read(fd, buf, sizeof(buf));
        close(fd);
        count++;
    }

    printf("%.2f ops/sec\n", count / 60.0);
    return 0;
}