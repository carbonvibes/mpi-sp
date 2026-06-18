#ifndef BENCHMARK_H
#define BENCHMARK_H

#include <stdio.h>
#include <time.h>

#define ITERATIONS 3

typedef int (*work_fn)(void);

static int warmup(work_fn work) {
  time_t warmup_start = time(NULL);

  while (time(NULL) - warmup_start < 120) {
    if (work() != 0) {
      return 1;
    }
  }
  return 0;
}

static int bench(work_fn work) {
  time_t start = time(NULL);
  int count = 0;

  while (time(NULL) - start < 60) {
    if (work() != 0) {
      return 1;
    }
    count++;
  }

  printf("Iterations: %d\n", count);
  return 0;
}

static int run_benchmark(work_fn work) {
  printf("Warmup...\n");
  if (warmup(work) != 0) {
    return 1;
  }
  for (int i = 0; i < ITERATIONS; i++) {
    printf("Benchmark %d:\n", i + 1);
    if (bench(work) != 0) {
      return 1;
    }
  }
  return 0;
}

#endif
