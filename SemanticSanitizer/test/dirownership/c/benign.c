#include "unistd.h"
#include <stdio.h>
#include <sys/stat.h>

int main() {
  if (mkdir("a", 0755) == -1) {
    perror("mkdir");
    return 1;
  }

  if (mkdir("a/b", 0755) == -1) {
    perror("mkdir");
    return 1;
  }

  if (rmdir("a/b") == -1) {
    perror("rmdir");
    return 1;
  }

  return 0;
}
