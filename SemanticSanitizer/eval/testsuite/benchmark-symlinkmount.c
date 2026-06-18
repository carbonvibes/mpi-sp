#include <errno.h>
#include <stdio.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mount.h>
#include <sys/stat.h>

#include "benchmark.h"

const char *mountpoint = "/tmp/benchmark-symlinkmount";

int work() {
  if (mount("tmpfs", mountpoint, "tmpfs", 0, NULL) == -1) {
    perror("mount");
    return 1;
  }

  if (umount(mountpoint) == -1) {
    perror("umount");
    return 1;
  }

  return 0;
}

int main() {
  printf("Symlinkmount benchmark\n");
  if (mkdir(mountpoint, 0755) == -1 && errno != EEXIST) {
    perror("mkdir");
    return 1;
  }

  return run_benchmark(work);
}
