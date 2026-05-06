#include <stdint.h>
#include <stddef.h>
#include <archive.h>
#include <archive_entry.h>

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
