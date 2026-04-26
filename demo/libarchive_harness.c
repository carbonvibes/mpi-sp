/*
 * libarchive_harness.c — in-process libarchive fuzzing harness.
 *
 * The Rust fuzzer calls fuzz_libarchive() with the primary content bytes from
 * each FsDelta.  libarchive tries to parse those bytes as any supported archive
 * format (tar, zip, cpio, 7z, xz, bz2, gz, ...) in-process.  Memory errors are
 * caught by AddressSanitizer; hangs are caught by the InProcessExecutor timeout.
 *
 * archive_read_open_memory() avoids any filesystem I/O — the input bytes are
 * fed directly from the FsDelta content buffer.  This makes libarchive the ideal
 * first real-world campaign: our content mutations (bit-flip, perturb, dictionary
 * draws of ELF/binary magic bytes) map directly to what archive parsers consume.
 *
 * Build requirements:
 *   apt install libarchive-dev   (Ubuntu/Debian)
 *   dnf install libarchive-devel (Fedora/RHEL)
 * Link: -larchive
 */

#include <stdint.h>
#include <stddef.h>
#include <archive.h>
#include <archive_entry.h>

/* Read an archive from a filesystem path (used with the FUSE mount).
 * The target opens the file through the kernel → FUSE → VFS path, so the full
 * FsDelta (not just raw bytes) drives what libarchive parses. */
void fuzz_libarchive_from_path(const char *path)
{
    struct archive *a = archive_read_new();
    if (!a) return;

    archive_read_support_filter_all(a);
    archive_read_support_format_all(a);

    if (archive_read_open_filename(a, path, 65536) == ARCHIVE_OK) {
        struct archive_entry *entry;
        while (archive_read_next_header(a, &entry) == ARCHIVE_OK)
            archive_read_data_skip(a);
    }

    archive_read_free(a);
}

void fuzz_libarchive(const uint8_t *data, size_t len)
{
    struct archive *a = archive_read_new();
    if (!a) return;

    archive_read_support_filter_all(a);
    archive_read_support_format_all(a);

    /* Feed content bytes directly — no temp file, no filesystem I/O. */
    if (archive_read_open_memory(a, data, len) == ARCHIVE_OK) {
        struct archive_entry *entry;
        while (archive_read_next_header(a, &entry) == ARCHIVE_OK) {
            /* Drain entry data to exercise decompression and format parsers. */
            archive_read_data_skip(a);
        }
    }

    archive_read_free(a);
}
