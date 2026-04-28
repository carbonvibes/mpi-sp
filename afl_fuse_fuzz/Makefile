# AFL++ FUSE fuzzing harness for foobar_target.c
#
# Quickstart:
#   sudo apt install afl++          # install AFL++ (first time only)
#   make                            # build both binaries
#   ./run_fuzzing.sh                # mount FUSE and start afl-fuzz
#
# To smoke-test FUSE wiring without AFL++:
#   make smoke

AFL_CC    ?= afl-clang-fast
CC        ?= gcc
TARGET_SRC = ../demo/foobar_target.c
MOUNT_DIR  = ./mnt

FUSE_CFLAGS := $(shell pkg-config --cflags fuse3)
FUSE_LIBS   := $(shell pkg-config --libs   fuse3)

.PHONY: all smoke clean mount umount

all: fuse_single_file afl_harness

# ── FUSE daemon (plain gcc, no instrumentation needed) ──────────────────────
fuse_single_file: fuse_single_file.c
	$(CC) -Wall -O2 $(FUSE_CFLAGS) -o $@ $< $(FUSE_LIBS)

# ── AFL++ harness: links foobar_target.c so fuzz_foobar_from_path is resolved ─
afl_harness: afl_harness.c $(TARGET_SRC)
	$(AFL_CC) -Wall -O2 \
	    -DFUSE_INPUT_PATH=\"$(MOUNT_DIR)/input\" \
	    -o $@ $^

# ── smoke test: build with plain gcc, pipe a seed through FUSE manually ─────
afl_harness_plain: afl_harness.c $(TARGET_SRC)
	$(CC) -Wall -O2 \
	    -DFUSE_INPUT_PATH=\"$(MOUNT_DIR)/input\" \
	    -o $@ $^

smoke: fuse_single_file afl_harness_plain $(MOUNT_DIR)
	./fuse_single_file -s $(MOUNT_DIR) &
	sleep 0.4
	@echo "foobar" | ./afl_harness_plain && echo "smoke: no crash (expected)" || echo "smoke: crashed on foobar (expected)"
	fusermount3 -u $(MOUNT_DIR)

$(MOUNT_DIR):
	mkdir -p $(MOUNT_DIR)

mount: fuse_single_file $(MOUNT_DIR)
	./fuse_single_file -s $(MOUNT_DIR) &
	sleep 0.3

umount:
	fusermount3 -u $(MOUNT_DIR) 2>/dev/null || true

clean: umount
	rm -f fuse_single_file afl_harness afl_harness_plain
