CC ?= gcc
CFLAGS ?= -Wall -Wextra -O2
BUILDDIR ?= build

SRCS := $(wildcard benchmark-*.c)
BINS := $(SRCS:.c=)
BUILD_BINS := $(addprefix $(BUILDDIR)/,$(BINS))

all: $(BUILD_BINS)

$(BUILDDIR):
	mkdir -p $@

$(BUILDDIR)/%: %.c benchmark.h | $(BUILDDIR)
	$(CC) $(CFLAGS) -o $@ $<

clean:
	rm -rf $(BUILDDIR)

.PHONY: all clean
