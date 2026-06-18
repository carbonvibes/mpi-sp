#include <fcntl.h>
#include <stdio.h>
#include <sys/stat.h>
#include <sys/mount.h>

int main() {
  if (mkdir("a", 0755) == -1) {
    perror("mkdir");
    return 1;
  }

  if (mkdir("mnt", 0755) == -1) {
    perror("mkdir");
    return 1;
  }

  if (mount("a", "mnt", NULL, MS_BIND, NULL) == -1) {
    perror("mount");
    return 1;
  }

  return 0;
}
