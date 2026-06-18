#include <sys/time.h>
#include <stdio.h>
#include <unistd.h>
#include <sys/syscall.h>

int main() {
  struct timeval tv;
  syscall(SYS_gettimeofday, &tv, NULL);
  printf("Seconds: %ld, Microseconds: %ld\n", tv.tv_sec, tv.tv_usec);
  return 0;
}
