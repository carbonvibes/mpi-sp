/*
 * delta.h — Filesystem delta data structures and serialization.
 *
 * A delta (fs_delta_t) is an ordered list of typed filesystem operations
 * (fs_op_t).  It is the canonical testcase representation: the fuzzer
 * produces deltas via Rust mutator stages, and the control plane applies
 * them to the live VFS via cp_apply_delta().
 *
 * Wire format (compact binary):
 *
 *   [n_ops  u32 BE]            — number of ops; 0 is invalid
 *   per op:
 *     [kind      u8   ]        — fs_op_kind_t value (1–7); 0 reserved
 *     [path_len  u16 BE]       — byte count of path; path must start with '/'
 *     [path      bytes]        — not NUL-terminated
 *     [size      u32 BE]       — semantic size:
 *                                  CREATE_FILE/UPDATE_FILE: content length
 *                                  TRUNCATE:               new file size
 *                                  others:                 0
 *     [data_len  u32 BE]       — actual data bytes that follow:
 *                                  CREATE_FILE/UPDATE_FILE: == size
 *                                  all others:             0
 *     [data      bytes]        — data_len bytes of content
 *     [has_ts    u8   ]        — 1 if timestamps follow; 0 otherwise
 *                                  only SET_TIMES ops set this to 1
 *     if has_ts:
 *       [mtime_sec  s64 BE]
 *       [mtime_nsec s64 BE]
 *       [atime_sec  s64 BE]
 *       [atime_nsec s64 BE]
 *
 * The Rust mutator stages always re-serialize from a valid fs_delta_t so
 * the format is never byte-flipped by the fuzzer.  Timestamps are therefore
 * only stored when they carry real data (SET_TIMES ops), saving 32 bytes per
 * op on all other kinds.
 */

#ifndef DELTA_H
#define DELTA_H

#include <stddef.h>
#include <stdint.h>
#include <time.h>

/* -------------------------------------------------------------------------
 * Op kinds  (wire byte value matches enum value)
 * ---------------------------------------------------------------------- */

typedef enum {
    FS_OP_CREATE_FILE = 1,   /* create file with content */
    FS_OP_UPDATE_FILE = 2,   /* replace entire file content */
    FS_OP_DELETE_FILE = 3,   /* unlink file or symlink */
    FS_OP_MKDIR       = 4,   /* create directory */
    FS_OP_RMDIR       = 5,   /* remove empty directory */
    FS_OP_SET_TIMES   = 6,   /* set mtime and/or atime */
    FS_OP_TRUNCATE    = 7,   /* resize file (zeros on extend) */
} fs_op_kind_t;

#define FS_OP_KIND_MIN 1
#define FS_OP_KIND_MAX 7

/* -------------------------------------------------------------------------
 * fs_op_t — a single filesystem operation
 * ---------------------------------------------------------------------- */

typedef struct {
    fs_op_kind_t     kind;
    char            *path;        /* NUL-terminated absolute path; heap-allocated */
    uint8_t         *content;     /* CREATE_FILE, UPDATE_FILE: heap-allocated data;
                                     NULL for all other kinds */
    size_t           content_len; /* CREATE_FILE/UPDATE_FILE: byte count of content;
                                     TRUNCATE:                new file size;
                                     others:                  0 */
    struct timespec  mtime;       /* SET_TIMES: desired mtime; zero for other kinds */
    struct timespec  atime;       /* SET_TIMES: desired atime; zero for other kinds */
} fs_op_t;

/* -------------------------------------------------------------------------
 * fs_delta_t — ordered list of ops
 * ---------------------------------------------------------------------- */

typedef struct {
    fs_op_t *ops;    /* heap-allocated array */
    size_t   n_ops;  /* number of valid entries */
    size_t   cap;    /* allocated capacity (internal) */
} fs_delta_t;

/* Per-op minimum fixed overhead in the wire format (bytes),
 * excluding path, data, and conditional timestamps. */
#define DELTA_OP_FIXED 12u   /* 1(kind)+2(path_len)+4(size)+4(data_len)+1(has_ts) */

/* Timestamp block size — only present when has_ts == 1 (SET_TIMES ops). */
#define DELTA_TS_SIZE  32u   /* 4 × s64: mtime_sec, mtime_nsec, atime_sec, atime_nsec */

/* -------------------------------------------------------------------------
 * Lifecycle
 * ---------------------------------------------------------------------- */

/* Allocate an empty delta.  Returns NULL on ENOMEM. */
fs_delta_t *delta_create(void);

/* Deep-free all memory held by the delta. */
void delta_free(fs_delta_t *d);

/* Append a deep copy of *op to d.  Returns 0 or -ENOMEM. */
int delta_add_op(fs_delta_t *d, const fs_op_t *op);

/* -------------------------------------------------------------------------
 * Convenience constructors
 * ---------------------------------------------------------------------- */

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

/* -------------------------------------------------------------------------
 * Serialization
 * ---------------------------------------------------------------------- */

/*
 * Serialize d to a freshly-allocated byte buffer.
 * *out_len is set to the buffer length.
 * Returns NULL (and *out_len = 0) on ENOMEM or if d is empty.
 * Caller must free() the returned buffer.
 */
uint8_t *delta_serialize(const fs_delta_t *d, size_t *out_len);

/*
 * Deserialize buf[0..len-1] into a freshly-allocated delta.
 * Returns NULL on any parse or allocation error; *err_out is then a negative
 * errno value.  On success *err_out is 0.
 */
fs_delta_t *delta_deserialize(const uint8_t *buf, size_t len, int *err_out);

/* -------------------------------------------------------------------------
 * Checksum
 * ---------------------------------------------------------------------- */

/*
 * Compute a 64-bit FNV-1a checksum of a raw byte buffer.
 * Intended use: delta_checksum(buf, len) on a serialized delta for
 * baseline-reproducibility tagging.
 */
uint64_t delta_checksum(const uint8_t *buf, size_t len);

/* -------------------------------------------------------------------------
 * Utilities
 * ---------------------------------------------------------------------- */

/* Return a static string name for an op kind (e.g. "CREATE_FILE"). */
const char *op_kind_name(fs_op_kind_t kind);

/* Print a human-readable listing of all ops to stdout. */
void delta_dump(const fs_delta_t *d);

#endif /* DELTA_H */
