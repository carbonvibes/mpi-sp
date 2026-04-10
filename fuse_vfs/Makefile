CC         = gcc
CFLAGS     = -Wall -Wextra -O2 -I..
FUSE_FLAGS = $(shell pkg-config --cflags --libs fuse3)

VFS_SRC = ../vfs/vfs.c
VFS_HDR = ../vfs/vfs.h

.PHONY: all test clean

all: fuse_vfs

fuse_vfs: fuse_vfs.c $(VFS_SRC) $(VFS_HDR)
	$(CC) $(CFLAGS) -o $@ fuse_vfs.c $(VFS_SRC) $(FUSE_FLAGS)

# Mount, run integration checks, unmount.
test: fuse_vfs
	bash test_mount.sh

clean:
	rm -f fuse_vfs
