# control_plane/Makefile
#
# Builds:
#   libdelta.a          — delta data structure and serialization
#   libcontrol_plane.a  — full control plane (depends on libdelta.a + VFS)
#   cp_test             — test binary (links everything together)
#
# Usage:
#   make            — build all
#   make test       — build and run cp_test
#   make clean      — remove build artifacts

CC      = gcc
CFLAGS  = -std=c11 -Wall -Wextra -Wpedantic -g \
           -D_DEFAULT_SOURCE \
           -I..    # for ../vfs/vfs.h
LDFLAGS =

VFS_SRC  = ../vfs/vfs.c
VFS_OBJ  = vfs.o

DELTA_SRC  = delta.c
DELTA_OBJ  = delta.o

CP_SRC  = control_plane.c
CP_OBJ  = control_plane.o

TEST_SRC = cp_test.c
TEST_OBJ = cp_test.o
TEST_BIN = cp_test

.PHONY: all test clean

all: libcontrol_plane.a $(TEST_BIN)

# VFS object (compiled locally to avoid dependency on ../vfs/Makefile)
$(VFS_OBJ): $(VFS_SRC) ../vfs/vfs.h
	$(CC) $(CFLAGS) -c $< -o $@

$(DELTA_OBJ): $(DELTA_SRC) delta.h
	$(CC) $(CFLAGS) -c $< -o $@

$(CP_OBJ): $(CP_SRC) control_plane.h delta.h ../vfs/vfs.h
	$(CC) $(CFLAGS) -c $< -o $@

$(TEST_OBJ): $(TEST_SRC) control_plane.h delta.h ../vfs/vfs.h
	$(CC) $(CFLAGS) -c $< -o $@

libdelta.a: $(DELTA_OBJ)
	ar rcs $@ $^

libcontrol_plane.a: $(DELTA_OBJ) $(CP_OBJ) $(VFS_OBJ)
	ar rcs $@ $^

$(TEST_BIN): $(TEST_OBJ) $(DELTA_OBJ) $(CP_OBJ) $(VFS_OBJ)
	$(CC) $(CFLAGS) $^ -o $@ $(LDFLAGS)

test: $(TEST_BIN)
	./$(TEST_BIN)

clean:
	rm -f $(VFS_OBJ) $(DELTA_OBJ) $(CP_OBJ) $(TEST_OBJ) \
	      libdelta.a libcontrol_plane.a $(TEST_BIN)
