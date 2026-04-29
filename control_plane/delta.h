#ifndef DELTA_H
#define DELTA_H

#include <stddef.h>
#include <stdint.h>
#include <time.h>

typedef enum {
    FS_OP_CREATE_FILE = 1,
    FS_OP_UPDATE_FILE = 2,
    FS_OP_DELETE_FILE = 3,
    FS_OP_MKDIR       = 4,
    FS_OP_RMDIR       = 5,
    FS_OP_SET_TIMES   = 6,
    FS_OP_TRUNCATE    = 7,
} fs_op_kind_t;

#define FS_OP_KIND_MIN 1
#define FS_OP_KIND_MAX 7

typedef struct {
    fs_op_kind_t     kind;
    char            *path;        /* NUL-terminated absolute path; heap-allocated */
    uint8_t         *content;     /* CREATE_FILE/UPDATE_FILE only */
    size_t           content_len; /* CREATE_FILE/UPDATE_FILE: byte count;
                                     TRUNCATE: new file size; others: 0 */
    struct timespec  mtime;
    struct timespec  atime;
} fs_op_t;

typedef struct {
    fs_op_t *ops;
    size_t   n_ops;
    size_t   cap;
} fs_delta_t;

/* wire format fixed overhead per op (excl. path, data, timestamps) */
#define DELTA_OP_FIXED 12u   /* kind(1)+path_len(2)+size(4)+data_len(4)+has_ts(1) */
#define DELTA_TS_SIZE  32u   /* 4×s64: mtime_sec, mtime_nsec, atime_sec, atime_nsec */

fs_delta_t *delta_create(void);
void delta_free(fs_delta_t *d);
int delta_add_op(fs_delta_t *d, const fs_op_t *op);

int delta_add_create_file(fs_delta_t *d, const char *path,
                          const uint8_t *content, size_t content_len);
int delta_add_update_file(fs_delta_t *d, const char *path,
                          const uint8_t *content, size_t content_len);
int delta_add_delete_file(fs_delta_t *d, const char *path);
int delta_add_mkdir(fs_delta_t *d, const char *path);
int delta_add_rmdir(fs_delta_t *d, const char *path);
int delta_add_set_times(fs_delta_t *d, const char *path,
                        const struct timespec *mtime,
                        const struct timespec *atime);
int delta_add_truncate(fs_delta_t *d, const char *path, size_t new_size);

uint8_t *delta_serialize(const fs_delta_t *d, size_t *out_len);
fs_delta_t *delta_deserialize(const uint8_t *buf, size_t len, int *err_out);

uint64_t delta_checksum(const uint8_t *buf, size_t len);

const char *op_kind_name(fs_op_kind_t kind);
void delta_dump(const fs_delta_t *d);

#endif /* DELTA_H */
