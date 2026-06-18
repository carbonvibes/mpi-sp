#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>

#include "benchmark.h"

const char *filename = "test.txt";

int work() {
  if (chmod(filename, 0644) == -1) {
    perror("chmod");
    return 1;
  }
  return 0;
}

int main() {
  printf("Dirownership benchmark\n");

  int fd = open(filename, O_CREAT | O_WRONLY | O_TRUNC, 0600);
  if (fd == -1) {
    perror("open");
    return 1;
  }
  close(fd);

  int ret = run_benchmark(work);
  unlink(filename);
  return ret;
}
