CC     = gcc
CFLAGS = -Wall -Wextra -O2

FUSE_FLAGS = $(shell pkg-config --cflags --libs fuse3)

.PHONY: all test clean

all: vfs_test

vfs.o: vfs.c vfs.h
	$(CC) $(CFLAGS) -c -o $@ vfs.c

vfs_test: vfs_test.c vfs.o vfs.h
	$(CC) $(CFLAGS) -o $@ vfs_test.c vfs.o

test: vfs_test
	./vfs_test

clean:
	rm -f vfs.o vfs_test
