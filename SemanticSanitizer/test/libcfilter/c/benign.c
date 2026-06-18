
#include <stdio.h>

int main() {
  char buf[16];
  char *read = fgets(buf, 0, stdin);
  printf("read: %s\n", read);
  return 0;
}
