/*
 * control_plane.c — In-process control plane implementation.
 *
 * See control_plane.h for the full API contract.
 *
 * Apply algorithm:
 *
 *   Phase 1: Walk ops in original order, skip RMDIR.
 *     Before every CREATE_FILE or MKDIR: call cp_ensure_parents().
 *     Apply the op; record result.
 *
 *   Phase 2: Collect all RMDIR ops, sort by path depth descending
 *     (deepest first so children are removed before their parents), apply.
 *
 * Dry-run:
 *   After both phases, print the VFS tree via cp_dump_vfs(), then call
 *   vfs_reset_to_snapshot() to undo.  Requires a saved snapshot.
 */

#include "control_plane.h"

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* -------------------------------------------------------------------------
 * Internal helpers
 * ---------------------------------------------------------------------- */

/* Count path depth as the number of '/' characters (root "/" → depth 1). */
static int path_depth(const char *path)
{
    int d = 0;
    for (; *path; path++)
        if (*path == '/') d++;
    return d;
}

/* Entry for the RMDIR sort pass. */
typedef struct { size_t idx; int depth; } rmdir_entry_t;

static int cmp_rmdir_desc(const void *a, const void *b)
{
    const rmdir_entry_t *ea = a, *eb = b;
    return eb->depth - ea->depth;  /* descending depth */
}

/* -------------------------------------------------------------------------
 * cp_ensure_parents
 * ---------------------------------------------------------------------- */

int cp_ensure_parents(vfs_t *vfs, const char *path)
{
    if (!path || path[0] != '/') return -EINVAL;

    char *tmp = strdup(path);
    if (!tmp) return -ENOMEM;

    /*
     * Walk through each '/' separator.  For each prefix up to (but not
     * including) the final component, call vfs_mkdir.  EEXIST is fine.
     *
     * Example: path = "/a/b/c"
     *   i=2 → tmp[2]='\0' → mkdir("/a") → restore
     *   i=4 → tmp[4]='\0' → mkdir("/a/b") → restore
     *   i=6 = NUL → stop (don't create the final component)
     */
    for (size_t i = 1; tmp[i] != '\0'; i++) {
        if (tmp[i] == '/') {
            tmp[i] = '\0';
            int r = vfs_mkdir(vfs, tmp);
            tmp[i] = '/';
            if (r != 0 && r != -EEXIST) {
                free(tmp);
                return r;
            }
        }
    }

    free(tmp);
    return 0;
}

/* -------------------------------------------------------------------------
 * Apply a single op (no ensure_parents, no ordering fixup)
 * ---------------------------------------------------------------------- */

static int apply_single_op(vfs_t *vfs, const fs_op_t *op)
{
    switch (op->kind) {

    case FS_OP_CREATE_FILE:
        return vfs_create_file(vfs, op->path, op->content, op->content_len);

    case FS_OP_UPDATE_FILE:
        return vfs_update_file(vfs, op->path, op->content, op->content_len);

    case FS_OP_DELETE_FILE:
        return vfs_delete_file(vfs, op->path);

    case FS_OP_MKDIR:
        return vfs_mkdir(vfs, op->path);

    case FS_OP_RMDIR:
        return vfs_rmdir(vfs, op->path);

    case FS_OP_SET_TIMES:
        return vfs_set_times(vfs, op->path, &op->mtime, &op->atime);

    case FS_OP_TRUNCATE: {
        /*
         * Read-modify-write: preserve existing bytes up to the new size,
         * zero-fill on extension.  op->content_len is the new size.
         */
        vfs_stat_t vs;
        int r = vfs_getattr(vfs, op->path, &vs);
        if (r != 0) return r;
        if (vs.kind == VFS_DIR) return -EISDIR;

        size_t new_sz = op->content_len;
        uint8_t *tmp  = NULL;

        if (new_sz > 0) {
            tmp = calloc(1, new_sz);
            if (!tmp) return -ENOMEM;
            if (vs.size > 0) {
                size_t copy = vs.size < new_sz ? vs.size : new_sz;
                size_t got;
                vfs_read(vfs, op->path, 0, copy, tmp, &got);
            }
        }

        r = vfs_update_file(vfs, op->path, tmp, new_sz);
        free(tmp);
        return r;
    }

    default:
        return -EINVAL;
    }
}

/* -------------------------------------------------------------------------
 * cp_result_free
 * ---------------------------------------------------------------------- */

void cp_result_free(cp_result_t *r)
{
    if (!r) return;
    free(r->results);
    free(r);
}

/* -------------------------------------------------------------------------
 * cp_apply_delta
 * ---------------------------------------------------------------------- */

cp_result_t *cp_apply_delta(vfs_t *vfs, const fs_delta_t *d, int dry_run)
{
    cp_result_t *res = calloc(1, sizeof(*res));
    if (!res) return NULL;

    res->total_ops = (int)d->n_ops;
    if (d->n_ops > 0) {
        res->results = calloc(d->n_ops, sizeof(*res->results));
        if (!res->results) { free(res); return NULL; }
        for (size_t i = 0; i < d->n_ops; i++)
            res->results[i].op_index = (int)i;
    }

    /* --- Phase 1: non-RMDIR ops in original order --- */
    for (size_t i = 0; i < d->n_ops; i++) {
        const fs_op_t *op = &d->ops[i];
        if (op->kind == FS_OP_RMDIR) continue;

        cp_op_result_t *rr = &res->results[i];

        /* Ensure intermediate directories exist before create ops. */
        if (op->kind == FS_OP_CREATE_FILE || op->kind == FS_OP_MKDIR) {
            int er = cp_ensure_parents(vfs, op->path);
            if (er != 0) {
                rr->error   = er;
                rr->message = "ensure_parents failed";
                res->failed++;
                continue;
            }
        }

        int r = apply_single_op(vfs, op);
        /*
         * EEXIST on MKDIR is treated as success: the directory already
         * exists (possibly because ensure_parents() created it for a
         * preceding CREATE_FILE op), so the user's intent is satisfied.
         */
        if (r == -EEXIST && op->kind == FS_OP_MKDIR) r = 0;

        if (r == 0) {
            rr->error   = 0;
            rr->message = "ok";
            res->succeeded++;
        } else {
            rr->error   = r;
            rr->message = "vfs error";
            res->failed++;
        }
    }

    /* --- Phase 2: RMDIR ops, deepest first --- */
    size_t n_rmdir = 0;
    for (size_t i = 0; i < d->n_ops; i++)
        if (d->ops[i].kind == FS_OP_RMDIR) n_rmdir++;

    if (n_rmdir > 0) {
        rmdir_entry_t *rmdir_ops = malloc(n_rmdir * sizeof(*rmdir_ops));
        if (!rmdir_ops) {
            /* Out of memory: mark remaining RMDIR ops as failed. */
            for (size_t i = 0; i < d->n_ops; i++) {
                if (d->ops[i].kind == FS_OP_RMDIR) {
                    res->results[i].error   = -ENOMEM;
                    res->results[i].message = "rmdir sort: ENOMEM";
                    res->failed++;
                }
            }
        } else {
            size_t j = 0;
            for (size_t i = 0; i < d->n_ops; i++) {
                if (d->ops[i].kind == FS_OP_RMDIR)
                    rmdir_ops[j++] = (rmdir_entry_t){
                        .idx   = i,
                        .depth = path_depth(d->ops[i].path),
                    };
            }
            qsort(rmdir_ops, n_rmdir, sizeof(*rmdir_ops), cmp_rmdir_desc);

            for (size_t k = 0; k < n_rmdir; k++) {
                size_t i           = rmdir_ops[k].idx;
                cp_op_result_t *rr = &res->results[i];
                int r = vfs_rmdir(vfs, d->ops[i].path);
                if (r == 0) {
                    rr->error   = 0;
                    rr->message = "ok";
                    res->succeeded++;
                } else {
                    rr->error   = r;
                    rr->message = "vfs error";
                    res->failed++;
                }
            }
            free(rmdir_ops);
        }
    }

    /* --- Dry-run: print result then restore --- */
    if (dry_run) {
        printf("\n[dry-run] VFS state after applying delta:\n");
        cp_dump_vfs(vfs);
        printf("\n");

        int sr = vfs_reset_to_snapshot(vfs);
        if (sr != 0) {
            fprintf(stderr, "[dry-run] WARNING: no snapshot saved — "
                            "delta was applied permanently (vfs_reset_to_snapshot: %d)\n", sr);
        }
    }

    return res;
}

/* -------------------------------------------------------------------------
 * cp_vfs_checksum — FNV-1a walk of the VFS tree
 * ---------------------------------------------------------------------- */

/* Forward declaration for recursive helper. */
static void checksum_dir(vfs_t *vfs, const char *path, uint64_t *h);

/* Mix bytes into FNV-1a hash. */
static void fnv_mix(uint64_t *h, const void *data, size_t len)
{
    const uint8_t *p = data;
    for (size_t i = 0; i < len; i++)
        *h = (*h ^ p[i]) * 0x100000001b3ULL;
}

/*
 * Sorted checksum implementation.
 *
 * Children are collected into a flat array, sorted alphabetically by name,
 * then hashed in sorted order.  This makes the checksum insertion-order
 * independent: two VFS instances with the same files in different creation
 * orders produce the same hash.
 */

typedef struct {
    char       *name;        /* heap-allocated entry name              */
    char       *child_path;  /* heap-allocated absolute path           */
    vfs_stat_t  vs;          /* stat snapshot (kind, size, timestamps) */
} cksum_entry_t;

typedef struct {
    cksum_entry_t *entries;
    size_t         n;
    size_t         cap;
    const char    *parent_path;
} cksum_coll_ctx_t;

static int cksum_coll_cb(void *ctx, const char *name, const vfs_stat_t *vs)
{
    cksum_coll_ctx_t *c = ctx;
    if (strcmp(name, ".") == 0 || strcmp(name, "..") == 0) return 0;

    if (c->n >= c->cap) {
        size_t nc = c->cap ? c->cap * 2 : 8;
        cksum_entry_t *p = realloc(c->entries, nc * sizeof(*p));
        if (!p) return 0;   /* skip on OOM; checksum still useful */
        c->entries = p;
        c->cap = nc;
    }

    size_t plen = strlen(c->parent_path);
    size_t nlen = strlen(name);
    char *child = malloc(plen + nlen + 2);
    if (!child) return 0;

    if (strcmp(c->parent_path, "/") == 0)
        snprintf(child, plen + nlen + 2, "/%s", name);
    else
        snprintf(child, plen + nlen + 2, "%s/%s", c->parent_path, name);

    c->entries[c->n++] = (cksum_entry_t){
        .name       = strdup(name),
        .child_path = child,
        .vs         = *vs,
    };
    return 0;
}

static int cmp_cksum_entry(const void *a, const void *b)
{
    return strcmp(((const cksum_entry_t *)a)->name,
                  ((const cksum_entry_t *)b)->name);
}

static void checksum_dir(vfs_t *vfs, const char *path, uint64_t *h)
{
    /* Phase 1: collect all children. */
    cksum_coll_ctx_t coll = { .parent_path = path };
    vfs_readdir(vfs, path, cksum_coll_cb, &coll);

    /* Phase 2: sort alphabetically — insertion-order independent. */
    if (coll.n > 1)
        qsort(coll.entries, coll.n, sizeof(cksum_entry_t), cmp_cksum_entry);

    /* Phase 3: hash in sorted order. */
    for (size_t i = 0; i < coll.n; i++) {
        cksum_entry_t *e = &coll.entries[i];

        fnv_mix(h, e->child_path, strlen(e->child_path));
        fnv_mix(h, &e->vs.mtime.tv_sec,  sizeof(e->vs.mtime.tv_sec));
        fnv_mix(h, &e->vs.mtime.tv_nsec, sizeof(e->vs.mtime.tv_nsec));
        fnv_mix(h, &e->vs.atime.tv_sec,  sizeof(e->vs.atime.tv_sec));
        fnv_mix(h, &e->vs.atime.tv_nsec, sizeof(e->vs.atime.tv_nsec));

        if (e->vs.kind == VFS_FILE && e->vs.size > 0) {
            uint8_t *buf = malloc(e->vs.size);
            if (buf) {
                size_t got = 0;
                vfs_read(vfs, e->child_path, 0, e->vs.size, buf, &got);
                fnv_mix(h, buf, got);
                free(buf);
            }
        } else if (e->vs.kind == VFS_SYMLINK) {
            char target[4096];
            int r = vfs_readlink(vfs, e->child_path, target, sizeof(target) - 1);
            if (r > 0) { target[r] = '\0'; fnv_mix(h, target, (size_t)r); }
        } else if (e->vs.kind == VFS_DIR) {
            checksum_dir(vfs, e->child_path, h);
        }

        free(e->name);
        free(e->child_path);
    }
    free(coll.entries);
}

uint64_t cp_vfs_checksum(vfs_t *vfs)
{
    uint64_t h = 0xcbf29ce484222325ULL;
    /* Hash the root marker so an empty VFS has a non-trivial hash. */
    fnv_mix(&h, "/", 1);
    checksum_dir(vfs, "/", &h);
    return h;
}

/* -------------------------------------------------------------------------
 * cp_enumerate_paths — collect all VFS paths filtered by kind
 * ---------------------------------------------------------------------- */

/* Flat path list built across recursive calls. */
typedef struct {
    char  **paths;
    size_t  n;
    size_t  cap;
    int     filter;   /* 0=all, 1=files, 2=dirs */
} enum_list_t;

/* Per-readdir-call context (stack-allocated, not shared). */
typedef struct {
    vfs_t       *vfs;
    const char  *parent_path;
    enum_list_t *list;
} enum_cb_ctx_t;

/* Forward declaration. */
static void enum_dir(vfs_t *vfs, const char *parent, enum_list_t *list);

static int enum_list_push(enum_list_t *l, const char *path)
{
    if (l->n >= l->cap) {
        size_t nc = l->cap ? l->cap * 2 : 16;
        char **p  = realloc(l->paths, nc * sizeof(char *));
        if (!p) return -ENOMEM;
        l->paths = p;
        l->cap   = nc;
    }
    l->paths[l->n] = strdup(path);
    if (!l->paths[l->n]) return -ENOMEM;
    l->n++;
    return 0;
}

static int enum_readdir_cb(void *raw, const char *name, const vfs_stat_t *vs)
{
    enum_cb_ctx_t *rc = raw;
    if (strcmp(name, ".") == 0 || strcmp(name, "..") == 0) return 0;

    size_t plen = strlen(rc->parent_path);
    size_t nlen = strlen(name);
    char *child = malloc(plen + nlen + 2);
    if (!child) return 0;

    if (strcmp(rc->parent_path, "/") == 0)
        snprintf(child, plen + nlen + 2, "/%s", name);
    else
        snprintf(child, plen + nlen + 2, "%s/%s", rc->parent_path, name);

    int match = (rc->list->filter == 0)
             || (rc->list->filter == 1 && vs->kind == VFS_FILE)
             || (rc->list->filter == 2 && vs->kind == VFS_DIR);
    if (match) enum_list_push(rc->list, child);

    if (vs->kind == VFS_DIR)
        enum_dir(rc->vfs, child, rc->list);

    free(child);
    return 0;
}

static void enum_dir(vfs_t *vfs, const char *parent, enum_list_t *list)
{
    enum_cb_ctx_t ctx = { .vfs = vfs, .parent_path = parent, .list = list };
    vfs_readdir(vfs, parent, enum_readdir_cb, &ctx);
}

int cp_enumerate_paths(vfs_t *vfs, int filter, char ***paths_out, size_t *n_out)
{
    enum_list_t list = { .filter = filter };
    enum_dir(vfs, "/", &list);
    *paths_out = list.paths;
    *n_out     = list.n;
    return 0;
}

void cp_enumerate_paths_free(char **paths, size_t n)
{
    for (size_t i = 0; i < n; i++) free(paths[i]);
    free(paths);
}

/* -------------------------------------------------------------------------
 * cp_dump_vfs — indented tree listing
 * ---------------------------------------------------------------------- */

typedef struct {
    vfs_t      *vfs;
    const char *parent_path;
    int         depth;
} dump_ctx_t;

/* Forward declaration for recursion. */
static int dump_readdir_cb(void *ctx, const char *name, const vfs_stat_t *vs);

static int dump_readdir_cb(void *ctx, const char *name, const vfs_stat_t *vs)
{
    dump_ctx_t *dc = ctx;
    if (strcmp(name, ".") == 0 || strcmp(name, "..") == 0) return 0;

    /* Print indentation. */
    for (int i = 0; i < dc->depth; i++) printf("  ");

    /* Build child path. */
    size_t plen = strlen(dc->parent_path);
    size_t nlen = strlen(name);
    char *child_path = malloc(plen + nlen + 2);
    if (!child_path) { printf("(ENOMEM)\n"); return 0; }

    if (strcmp(dc->parent_path, "/") == 0)
        snprintf(child_path, plen + nlen + 2, "/%s", name);
    else
        snprintf(child_path, plen + nlen + 2, "%s/%s", dc->parent_path, name);

    if (vs->kind == VFS_DIR) {
        printf("[dir]  %s/\n", name);
        dump_ctx_t sub = { .vfs = dc->vfs, .parent_path = child_path, .depth = dc->depth + 1 };
        vfs_readdir(dc->vfs, child_path, dump_readdir_cb, &sub);
    } else if (vs->kind == VFS_SYMLINK) {
        char target[4096] = {0};
        int r = vfs_readlink(dc->vfs, child_path, target, sizeof(target) - 1);
        if (r > 0) target[r] = '\0';
        printf("[lnk]  %s -> %s\n", name, target);
    } else {
        printf("[file] %s  (%zu bytes)\n", name, vs->size);
    }

    free(child_path);
    return 0;
}

void cp_dump_vfs(vfs_t *vfs)
{
    printf("/\n");
    dump_ctx_t ctx = { .vfs = vfs, .parent_path = "/", .depth = 1 };
    vfs_readdir(vfs, "/", dump_readdir_cb, &ctx);
}
