#include <unistd.h>
#include <fcntl.h>

#include "benchmark.h"

const char *filename = "test.txt";

int work() {
  char buf[10];
  int fd = open(filename, O_CREAT | O_WRONLY | O_TRUNC, 0600);
  close(fd);
  unlink(filename);
  return 0;
}

int main() {
  printf("Canary benchmark\n");
  return run_benchmark(work);
}