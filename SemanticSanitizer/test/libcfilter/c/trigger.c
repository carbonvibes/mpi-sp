
#include <stdio.h>

int main() {
  char buf[16];
  char *read = gets(buf);
  printf("read: %s\n", read);
  return 0;
}
