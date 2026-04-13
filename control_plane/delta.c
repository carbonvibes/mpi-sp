/*
 * delta.c — fs_delta_t implementation: lifecycle, convenience constructors,
 *            serialization, deserialization, checksum, and dump utilities.
 *
 * Wire format per op:
 *   kind(1) | path_len(2) | path(path_len) |
 *   size(4) | data_len(4) | data(data_len) |
 *   mtime_sec(8) | mtime_nsec(8) | atime_sec(8) | atime_nsec(8)
 *
 * Total fixed overhead per op: 1+2+4+4+8+8+8+8 = 43 bytes.
 */

#include "delta.h"

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* -------------------------------------------------------------------------
 * Big-endian read/write helpers
 * ---------------------------------------------------------------------- */

static uint32_t r32be(const uint8_t *p)
{
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
           ((uint32_t)p[2] <<  8) |  (uint32_t)p[3];
}

static uint16_t r16be(const uint8_t *p)
{
    return (uint16_t)(((uint16_t)p[0] << 8) | p[1]);
}

static int64_t r64be(const uint8_t *p)
{
    uint64_t v = 0;
    for (int i = 0; i < 8; i++) v = (v << 8) | p[i];
    return (int64_t)v;
}

static void w32be(uint8_t *p, uint32_t v)
{
    p[0] = (uint8_t)(v >> 24);
    p[1] = (uint8_t)(v >> 16);
    p[2] = (uint8_t)(v >>  8);
    p[3] = (uint8_t) v;
}

static void w16be(uint8_t *p, uint16_t v)
{
    p[0] = (uint8_t)(v >> 8);
    p[1] = (uint8_t) v;
}

static void w64be(uint8_t *p, int64_t v)
{
    uint64_t u = (uint64_t)v;
    for (int i = 7; i >= 0; i--) { p[i] = (uint8_t)(u & 0xff); u >>= 8; }
}

/* -------------------------------------------------------------------------
 * Lifecycle
 * ---------------------------------------------------------------------- */

fs_delta_t *delta_create(void)
{
    return calloc(1, sizeof(fs_delta_t));
}

void delta_free(fs_delta_t *d)
{
    if (!d) return;
    for (size_t i = 0; i < d->n_ops; i++) {
        free(d->ops[i].path);
        free(d->ops[i].content);
    }
    free(d->ops);
    free(d);
}

int delta_add_op(fs_delta_t *d, const fs_op_t *op)
{
    if (d->n_ops == d->cap) {
        size_t newcap = d->cap ? d->cap * 2 : 4;
        fs_op_t *p = realloc(d->ops, newcap * sizeof(*p));
        if (!p) return -ENOMEM;
        d->ops = p;
        d->cap = newcap;
    }

    fs_op_t *dst = &d->ops[d->n_ops];
    dst->kind        = op->kind;
    dst->content_len = op->content_len;
    dst->mtime       = op->mtime;
    dst->atime       = op->atime;
    dst->content     = NULL;
    dst->path        = NULL;

    if (op->path) {
        dst->path = strdup(op->path);
        if (!dst->path) return -ENOMEM;
    }

    if (op->content && op->content_len > 0) {
        dst->content = malloc(op->content_len);
        if (!dst->content) { free(dst->path); dst->path = NULL; return -ENOMEM; }
        memcpy(dst->content, op->content, op->content_len);
    }

    d->n_ops++;
    return 0;
}

/* -------------------------------------------------------------------------
 * Convenience constructors
 * ---------------------------------------------------------------------- */

int delta_add_create_file(fs_delta_t *d, const char *path,
                          const uint8_t *content, size_t content_len)
{
    fs_op_t op = {
        .kind        = FS_OP_CREATE_FILE,
        .path        = (char *)path,
        .content     = (uint8_t *)content,
        .content_len = content_len,
    };
    return delta_add_op(d, &op);
}

int delta_add_update_file(fs_delta_t *d, const char *path,
                          const uint8_t *content, size_t content_len)
{
    fs_op_t op = {
        .kind        = FS_OP_UPDATE_FILE,
        .path        = (char *)path,
        .content     = (uint8_t *)content,
        .content_len = content_len,
    };
    return delta_add_op(d, &op);
}

int delta_add_delete_file(fs_delta_t *d, const char *path)
{
    fs_op_t op = { .kind = FS_OP_DELETE_FILE, .path = (char *)path };
    return delta_add_op(d, &op);
}

int delta_add_mkdir(fs_delta_t *d, const char *path)
{
    fs_op_t op = { .kind = FS_OP_MKDIR, .path = (char *)path };
    return delta_add_op(d, &op);
}

int delta_add_rmdir(fs_delta_t *d, const char *path)
{
    fs_op_t op = { .kind = FS_OP_RMDIR, .path = (char *)path };
    return delta_add_op(d, &op);
}

int delta_add_set_times(fs_delta_t *d, const char *path,
                        const struct timespec *mtime,
                        const struct timespec *atime)
{
    struct timespec zero = {0, 0};
    fs_op_t op = {
        .kind  = FS_OP_SET_TIMES,
        .path  = (char *)path,
        .mtime = mtime ? *mtime : zero,
        .atime = atime ? *atime : zero,
    };
    return delta_add_op(d, &op);
}

int delta_add_truncate(fs_delta_t *d, const char *path, size_t new_size)
{
    fs_op_t op = {
        .kind        = FS_OP_TRUNCATE,
        .path        = (char *)path,
        .content_len = new_size,   /* content_len holds new size for TRUNCATE */
    };
    return delta_add_op(d, &op);
}

/* -------------------------------------------------------------------------
 * Serialization
 * ---------------------------------------------------------------------- */

uint8_t *delta_serialize(const fs_delta_t *d, size_t *out_len)
{
    *out_len = 0;
    if (!d || d->n_ops == 0) return NULL;

    /* Calculate total buffer size. */
    size_t total = 8; /* magic(4) + n_ops(4) */
    for (size_t i = 0; i < d->n_ops; i++) {
        const fs_op_t *op = &d->ops[i];
        size_t path_len = op->path ? strlen(op->path) : 0;
        if (path_len > 0xFFFFu) return NULL;  /* path too long for u16 */

        /* data_len: only for CREATE_FILE and UPDATE_FILE */
        size_t data_len = (op->kind == FS_OP_CREATE_FILE ||
                           op->kind == FS_OP_UPDATE_FILE)
                          ? op->content_len : 0;

        total += DELTA_OP_FIXED + path_len + data_len;
    }

    uint8_t *buf = calloc(1, total);  /* calloc zeros; unused fields stay 0 */
    if (!buf) return NULL;

    /* Header */
    w32be(buf + 0, DELTA_MAGIC);
    w32be(buf + 4, (uint32_t)d->n_ops);

    size_t pos = 8;
    for (size_t i = 0; i < d->n_ops; i++) {
        const fs_op_t *op = &d->ops[i];
        size_t path_len = op->path ? strlen(op->path) : 0;

        /* Semantic size: content_len for data ops, new_size for TRUNCATE, 0 otherwise */
        uint32_t size_field = (uint32_t)op->content_len;

        /* Data bytes: only for CREATE_FILE / UPDATE_FILE */
        size_t data_len = (op->kind == FS_OP_CREATE_FILE ||
                           op->kind == FS_OP_UPDATE_FILE)
                          ? op->content_len : 0;

        buf[pos++] = (uint8_t)op->kind;

        w16be(buf + pos, (uint16_t)path_len); pos += 2;
        if (path_len) { memcpy(buf + pos, op->path, path_len); pos += path_len; }

        w32be(buf + pos, size_field);         pos += 4;
        w32be(buf + pos, (uint32_t)data_len); pos += 4;
        if (data_len && op->content) {
            memcpy(buf + pos, op->content, data_len);
        }
        pos += data_len;

        w64be(buf + pos, (int64_t)op->mtime.tv_sec);  pos += 8;
        w64be(buf + pos, (int64_t)op->mtime.tv_nsec); pos += 8;
        w64be(buf + pos, (int64_t)op->atime.tv_sec);  pos += 8;
        w64be(buf + pos, (int64_t)op->atime.tv_nsec); pos += 8;
    }

    *out_len = total;
    return buf;
}

/* -------------------------------------------------------------------------
 * Deserialization
 *
 * NEED(n): verify n bytes remain at pos, fail with -EINVAL if not.
 * Uses goto-based cleanup with per-op path/content pointers tracked locally.
 * ---------------------------------------------------------------------- */

#define NEED(n) \
    do { \
        if ((size_t)(n) > len - pos) { err = -EINVAL; goto fail_op; } \
    } while (0)

fs_delta_t *delta_deserialize(const uint8_t *buf, size_t len, int *err_out)
{
    *err_out = 0;

    /* Minimum: magic(4) + n_ops(4) */
    if (len < 8) { *err_out = -EINVAL; return NULL; }
    if (r32be(buf) != DELTA_MAGIC) { *err_out = -EINVAL; return NULL; }

    uint32_t n_ops = r32be(buf + 4);
    if (n_ops == 0 || n_ops > 65535u) { *err_out = -EINVAL; return NULL; }

    fs_delta_t *d = delta_create();
    if (!d) { *err_out = -ENOMEM; return NULL; }

    size_t pos = 8;

    for (uint32_t i = 0; i < n_ops; i++) {
        int      err     = 0;
        char    *path    = NULL;
        uint8_t *content = NULL;

        /* kind */
        NEED(1);
        uint8_t kind_raw = buf[pos++];
        if (kind_raw < FS_OP_KIND_MIN || kind_raw > FS_OP_KIND_MAX) {
            err = -EINVAL; goto fail_op;
        }

        /* path_len + path */
        NEED(2);
        uint16_t path_len = r16be(buf + pos); pos += 2;
        if (path_len == 0) { err = -EINVAL; goto fail_op; }
        NEED(path_len);
        if (buf[pos] != '/') { err = -EINVAL; goto fail_op; }
        path = malloc((size_t)path_len + 1);
        if (!path) { err = -ENOMEM; goto fail_op; }
        memcpy(path, buf + pos, path_len);
        path[path_len] = '\0';
        pos += path_len;

        /* size (semantic) + data_len (actual bytes following) */
        NEED(4);
        uint32_t size_field = r32be(buf + pos); pos += 4;
        NEED(4);
        uint32_t data_len = r32be(buf + pos); pos += 4;
        NEED(data_len);

        if (data_len > 0) {
            content = malloc(data_len);
            if (!content) { err = -ENOMEM; goto fail_op; }
            memcpy(content, buf + pos, data_len);
        }
        pos += data_len;

        /* timestamps */
        NEED(32);
        int64_t mtime_sec  = r64be(buf + pos); pos += 8;
        int64_t mtime_nsec = r64be(buf + pos); pos += 8;
        int64_t atime_sec  = r64be(buf + pos); pos += 8;
        int64_t atime_nsec = r64be(buf + pos); pos += 8;

        /*
         * content_len semantics:
         *   CREATE_FILE / UPDATE_FILE  → data_len (bytes in content buffer)
         *   TRUNCATE                   → size_field (new file size; no data)
         *   others                     → 0
         */
        size_t content_len;
        fs_op_kind_t kind = (fs_op_kind_t)kind_raw;
        if (kind == FS_OP_TRUNCATE)
            content_len = (size_t)size_field;
        else if (kind == FS_OP_CREATE_FILE || kind == FS_OP_UPDATE_FILE)
            content_len = (size_t)data_len;
        else
            content_len = 0;

        /* Grow ops array directly to steal pointers without extra copy. */
        if (d->n_ops == d->cap) {
            size_t newcap = d->cap ? d->cap * 2 : 4;
            fs_op_t *p = realloc(d->ops, newcap * sizeof(*p));
            if (!p) { err = -ENOMEM; goto fail_op; }
            d->ops = p;
            d->cap = newcap;
        }
        d->ops[d->n_ops++] = (fs_op_t){
            .kind        = kind,
            .path        = path,
            .content     = content,
            .content_len = content_len,
            .mtime       = { .tv_sec = (time_t)mtime_sec,  .tv_nsec = mtime_nsec  },
            .atime       = { .tv_sec = (time_t)atime_sec,  .tv_nsec = atime_nsec  },
        };
        continue;

fail_op:
        free(path);
        free(content);
        *err_out = err ? err : -EINVAL;
        delta_free(d);
        return NULL;
    }

    return d;
}

#undef NEED

/* -------------------------------------------------------------------------
 * Checksum  (FNV-1a 64-bit)
 * ---------------------------------------------------------------------- */

uint64_t delta_checksum(const uint8_t *buf, size_t len)
{
    uint64_t h = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < len; i++)
        h = (h ^ buf[i]) * 0x100000001b3ULL;
    return h;
}

/* -------------------------------------------------------------------------
 * Utilities
 * ---------------------------------------------------------------------- */

const char *op_kind_name(fs_op_kind_t kind)
{
    switch (kind) {
        case FS_OP_CREATE_FILE: return "CREATE_FILE";
        case FS_OP_UPDATE_FILE: return "UPDATE_FILE";
        case FS_OP_DELETE_FILE: return "DELETE_FILE";
        case FS_OP_MKDIR:       return "MKDIR";
        case FS_OP_RMDIR:       return "RMDIR";
        case FS_OP_SET_TIMES:   return "SET_TIMES";
        case FS_OP_TRUNCATE:    return "TRUNCATE";
        default:                return "UNKNOWN";
    }
}

void delta_dump(const fs_delta_t *d)
{
    if (!d) { printf("(null delta)\n"); return; }
    printf("delta: %zu op(s)\n", d->n_ops);
    for (size_t i = 0; i < d->n_ops; i++) {
        const fs_op_t *op = &d->ops[i];
        printf("  [%zu] %-12s %s", i, op_kind_name(op->kind),
               op->path ? op->path : "(null)");
        switch (op->kind) {
            case FS_OP_CREATE_FILE:
            case FS_OP_UPDATE_FILE:
                printf("  (%zu bytes)", op->content_len);
                break;
            case FS_OP_TRUNCATE:
                printf("  -> %zu bytes", op->content_len);
                break;
            case FS_OP_SET_TIMES:
                printf("  mtime=%ld.%09ld  atime=%ld.%09ld",
                       (long)op->mtime.tv_sec, op->mtime.tv_nsec,
                       (long)op->atime.tv_sec, op->atime.tv_nsec);
                break;
            default:
                break;
        }
        printf("\n");
    }
}
