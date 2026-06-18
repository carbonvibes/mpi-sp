#include <fcntl.h>
#include "unistd.h"
#include <stdio.h>
#include <sys/stat.h>

int main() {
  if (open("canary.txt", O_CREAT | O_WRONLY, S_IRUSR | S_IWUSR) < 0) {
    perror("open");
    return 1;
  }

  return 0;
}
