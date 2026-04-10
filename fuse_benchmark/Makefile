CC         = gcc
CFLAGS     = -Wall -Wextra -O2
FUSE_FLAGS = $(shell pkg-config --cflags --libs fuse3)

MOUNTPOINT = /tmp/benchmount

.PHONY: all clean mount umount bench

all: fuse_bench_fs bench_heavy

fuse_bench_fs: fuse_bench_fs.c
	$(CC) $(CFLAGS) -o $@ $< $(FUSE_FLAGS)

bench_heavy: bench_heavy.c
	$(CC) $(CFLAGS) -o $@ $<

mount: fuse_bench_fs
	mkdir -p $(MOUNTPOINT)
	./fuse_bench_fs $(MOUNTPOINT)

umount:
	fusermount3 -u $(MOUNTPOINT)

bench: all
	bash run_bench.sh

clean:
	rm -f fuse_bench_fs bench_heavy
