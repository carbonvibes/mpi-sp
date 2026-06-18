#include <unistd.h>
#include <stdio.h>
#include <string.h>

int main(int argc, char **argv) {
  if (argc > 1 && strcmp(argv[1], "--child") == 0) {
    return 0;
  }

  char *child_argv[] = {"benign_execve", "--child", "benign", NULL};
  char *envp[] = {NULL};

  if (execve("/proc/self/exe", child_argv, envp) < 0) {
    perror("execve");
    return 1;
  }

  return 0;
}
