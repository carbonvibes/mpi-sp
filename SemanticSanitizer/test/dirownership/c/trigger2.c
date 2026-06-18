#include "unistd.h"
#include <stdio.h>
#include <sys/stat.h>
#include <sys/mount.h>

int main() {
  if (mkdir("a", 0755) == -1) {
    perror("mkdir");
    return 1;
  }
  printf("mkdir a\n");

  if (mkdir("mnt", 0755) == -1) {
    perror("mkdir");
    return 1;
  }
  printf("mkdir mnt\n");

  if (mkdir("a/b", 0755) == -1) {
    perror("mkdir");
    return 1;
  }
  printf("mkdir a/b\n");

  if (chown("a", 1000, 1000) == -1) {
    perror("chown");
    return 1;
  }
  printf("chown a\n");

  if (mount("a/b", "mnt", NULL, MS_BIND, NULL) == -1) {
    perror("mount");
    return 1;
  }
  printf("mount a/b to mnt\n");

  return 0;
}
