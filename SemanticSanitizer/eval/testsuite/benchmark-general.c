#include <unistd.h>
#include <fcntl.h>

#include "benchmark.h"

const char *filename = "test.txt";

int work() {
  int fd = open(filename, O_CREAT | O_WRONLY | O_TRUNC, 0600);
  if (fd == -1) {
    perror("open");
    return 1;
  }
  close(fd);

  if (unlink(filename) == -1) {
    perror("unlink");
    return 1;
  }
  return 0;
}

int main() {
  printf("General benchmark\n");
  return run_benchmark(work);
}
