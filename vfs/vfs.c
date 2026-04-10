#include "vfs.h"

#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/* Return the current wall-clock time. */
static struct timespec ts_now(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return ts;
}

/* -------------------------------------------------------------------------
 * Internal helpers
 * ---------------------------------------------------------------------- */

/* Maximum allowed length for a single path component (name). */
#define VFS_MAX_NAME 255

/* Allocate a fresh node, assign the next inode number, zero everything. */
static vfs_node_t *node_alloc(vfs_t *vfs, vfs_kind_t kind)
{
    vfs_node_t *n = calloc(1, sizeof(*n));
    if (!n) return NULL;
    n->ino   = vfs->next_ino++;
    n->kind  = kind;
    n->mtime = ts_now();
    n->atime = n->mtime;
    return n;
}

/*
 * Free only the node itself and its own content buffer.
 * Does NOT free children or their subtrees; that is the caller's job.
 */
static void node_free_self(vfs_node_t *n)
{
    if (!n) return;
    free(n->content);
    free(n);
}

/* Recursively free a node and the entire subtree rooted at it. */
static void node_free_deep(vfs_node_t *n)
{
    if (!n) return;
    if (n->kind == VFS_DIR) {
        vfs_dirent_t *d = n->children;
        while (d) {
            vfs_dirent_t *next = d->next;
            node_free_deep(d->node);
            free(d->name);
            free(d);
            d = next;
        }
    }
    node_free_self(n);
}

/*
 * Deep-copy the subtree rooted at src.
 * parent_copy is the parent of the returned copy (NULL for the root copy).
 * Inode numbers are preserved from the source.
 * Returns NULL on allocation failure; the partially-built copy is freed.
 */
static vfs_node_t *node_deepcopy(const vfs_node_t *src, vfs_node_t *parent_copy)
{
    if (!src) return NULL;

    vfs_node_t *dst = calloc(1, sizeof(*dst));
    if (!dst) return NULL;

    dst->ino    = src->ino;
    dst->kind   = src->kind;
    dst->parent = parent_copy;
    dst->mtime  = src->mtime;
    dst->atime  = src->atime;

    if (src->kind == VFS_FILE) {
        if (src->content_len > 0) {
            dst->content = malloc(src->content_len);
            if (!dst->content) { free(dst); return NULL; }
            memcpy(dst->content, src->content, src->content_len);
        }
        dst->content_len = src->content_len;
    } else {
        /* Copy children, keeping the same insertion order. */
        vfs_dirent_t **tail = &dst->children;
        for (const vfs_dirent_t *d = src->children; d; d = d->next) {
            vfs_dirent_t *nd = calloc(1, sizeof(*nd));
            if (!nd) goto oom;

            nd->name = strdup(d->name);
            if (!nd->name) { free(nd); goto oom; }

            nd->node = node_deepcopy(d->node, dst);
            if (!nd->node) { free(nd->name); free(nd); goto oom; }

            *tail = nd;
            tail  = &nd->next;
        }
    }
    return dst;

oom:
    node_free_deep(dst);
    return NULL;
}

/*
 * Resolve an absolute path to the node it names.
 *
 * Rules enforced here:
 *   - path must start with '/'
 *   - components must not be empty (no double slash, no trailing slash)
 *   - "." and ".." components are rejected (EINVAL)
 *
 * On success returns the node pointer.
 * On failure returns NULL and sets errno.
 */
static vfs_node_t *resolve_path(vfs_t *vfs, const char *path)
{
    if (!path || path[0] != '/') { errno = EINVAL; return NULL; }

    vfs_node_t *cur = vfs->root;
    const char *p   = path + 1;

    /* Bare "/" → root. */
    if (*p == '\0') return cur;

    while (*p) {
        const char *start = p;
        while (*p && *p != '/') p++;
        size_t len = (size_t)(p - start);

        /* Empty component: double slash or trailing slash. */
        if (len == 0) { errno = EINVAL; return NULL; }

        /* Reject "." and "..". */
        if (len == 1 && start[0] == '.') { errno = EINVAL; return NULL; }
        if (len == 2 && start[0] == '.' && start[1] == '.') {
            errno = EINVAL;
            return NULL;
        }

        if (len > VFS_MAX_NAME) { errno = ENAMETOOLONG; return NULL; }

        /* Current node must be a directory to descend into it. */
        if (cur->kind != VFS_DIR) { errno = ENOTDIR; return NULL; }

        /* Look up this component in the current directory. */
        vfs_node_t *child = NULL;
        for (vfs_dirent_t *d = cur->children; d; d = d->next) {
            if (strlen(d->name) == len && memcmp(d->name, start, len) == 0) {
                child = d->node;
                break;
            }
        }
        if (!child) { errno = ENOENT; return NULL; }
        cur = child;

        /* Step over the '/' separator, if present. */
        if (*p == '/') {
            p++;
            /* Trailing slash: nothing follows — reject. */
            if (*p == '\0') { errno = EINVAL; return NULL; }
        }
    }

    return cur;
}

/*
 * Split path into a parent path and a final component name.
 *
 * Resolves the parent directory and writes the final component into
 * name_buf (at most name_buf_len-1 chars, NUL-terminated).
 *
 * Returns the parent directory node on success.
 * Returns NULL and sets errno on failure.
 */
static vfs_node_t *resolve_parent(vfs_t *vfs, const char *path,
                                   char *name_buf)
{
    if (!path || path[0] != '/') { errno = EINVAL; return NULL; }

    const char *last_slash = strrchr(path, '/');
    const char *name       = last_slash + 1;
    size_t      name_len   = strlen(name);

    /* Trailing slash or bare "/". */
    if (name_len == 0) { errno = EINVAL; return NULL; }

    if (name_len > VFS_MAX_NAME) { errno = ENAMETOOLONG; return NULL; }

    /* Reject "." and ".." as the final component. */
    if (name_len == 1 && name[0] == '.') { errno = EINVAL; return NULL; }
    if (name_len == 2 && name[0] == '.' && name[1] == '.') {
        errno = EINVAL;
        return NULL;
    }

    memcpy(name_buf, name, name_len);
    name_buf[name_len] = '\0';

    /* Resolve the parent portion of the path. */
    size_t parent_len = (size_t)(last_slash - path);

    if (parent_len == 0) {
        /* Parent is root ("/foo" case). */
        return resolve_path(vfs, "/");
    }

    char *parent_path = malloc(parent_len + 1);
    if (!parent_path) { errno = ENOMEM; return NULL; }
    memcpy(parent_path, path, parent_len);
    parent_path[parent_len] = '\0';

    vfs_node_t *parent = resolve_path(vfs, parent_path);
    int saved_errno = errno;
    free(parent_path);
    errno = saved_errno;
    return parent;
}

/* -------------------------------------------------------------------------
 * Directory helpers
 * ---------------------------------------------------------------------- */

/* Append a (name, node) entry to a directory. The name is strdup'd. */
static int dir_add_child(vfs_node_t *dir, const char *name, vfs_node_t *child)
{
    vfs_dirent_t *d = calloc(1, sizeof(*d));
    if (!d) return -ENOMEM;

    d->name = strdup(name);
    if (!d->name) { free(d); return -ENOMEM; }
    d->node = child;

    /* Append to maintain insertion order. */
    vfs_dirent_t **tail = &dir->children;
    while (*tail) tail = &(*tail)->next;
    *tail = d;
    return 0;
}

/*
 * Remove the entry named name from dir.
 * The dirent struct and its name are freed; the node pointer is NOT freed.
 * Returns 0 on success, -ENOENT if name is not present.
 */
static int dir_remove_child(vfs_node_t *dir, const char *name)
{
    for (vfs_dirent_t **p = &dir->children; *p; p = &(*p)->next) {
        if (strcmp((*p)->name, name) == 0) {
            vfs_dirent_t *d = *p;
            *p = d->next;
            free(d->name);
            free(d);
            return 0;
        }
    }
    return -ENOENT;
}

/* Return the child node named name, or NULL if absent. */
static vfs_node_t *dir_lookup_child(const vfs_node_t *dir, const char *name)
{
    for (const vfs_dirent_t *d = dir->children; d; d = d->next)
        if (strcmp(d->name, name) == 0) return d->node;
    return NULL;
}

/* -------------------------------------------------------------------------
 * Stat conversion
 * ---------------------------------------------------------------------- */

static void fill_stat(const vfs_node_t *n, vfs_stat_t *st)
{
    st->ino   = n->ino;
    st->kind  = n->kind;
    st->size  = (n->kind == VFS_FILE) ? n->content_len : 0;
    st->mtime = n->mtime;
    st->atime = n->atime;
}

/* -------------------------------------------------------------------------
 * Lifecycle
 * ---------------------------------------------------------------------- */

vfs_t *vfs_create(void)
{
    vfs_t *vfs = calloc(1, sizeof(*vfs));
    if (!vfs) return NULL;

    vfs->next_ino = 1;
    vfs->root = node_alloc(vfs, VFS_DIR);
    if (!vfs->root) { free(vfs); return NULL; }

    return vfs;
}

void vfs_destroy(vfs_t *vfs)
{
    if (!vfs) return;
    node_free_deep(vfs->root);
    node_free_deep(vfs->snapshot);
    free(vfs);
}

/* -------------------------------------------------------------------------
 * Read-only operations
 * ---------------------------------------------------------------------- */

int vfs_getattr(vfs_t *vfs, const char *path, vfs_stat_t *out)
{
    errno = 0;
    vfs_node_t *n = resolve_path(vfs, path);
    if (!n) return -errno;
    fill_stat(n, out);
    return 0;
}

int vfs_readdir(vfs_t *vfs, const char *path,
                vfs_readdir_cb_t cb, void *ctx)
{
    errno = 0;
    vfs_node_t *n = resolve_path(vfs, path);
    if (!n) return -errno;
    if (n->kind != VFS_DIR) return -ENOTDIR;

    vfs_stat_t st;
    int ret;

    /* "." */
    fill_stat(n, &st);
    ret = cb(ctx, ".", &st);
    if (ret) return ret;

    /* ".." — root's parent is itself */
    vfs_node_t *parent_node = n->parent ? n->parent : n;
    fill_stat(parent_node, &st);
    ret = cb(ctx, "..", &st);
    if (ret) return ret;

    /* Real children. */
    for (const vfs_dirent_t *d = n->children; d; d = d->next) {
        fill_stat(d->node, &st);
        ret = cb(ctx, d->name, &st);
        if (ret) return ret;
    }

    return 0;
}

int vfs_read(vfs_t *vfs, const char *path, size_t offset, size_t size,
             uint8_t *buf, size_t *out_len)
{
    errno = 0;
    vfs_node_t *n = resolve_path(vfs, path);
    if (!n) return -errno;
    if (n->kind == VFS_DIR) return -EISDIR;

    if (offset >= n->content_len || size == 0) {
        *out_len = 0;
        return 0;
    }

    size_t available = n->content_len - offset;
    size_t to_copy   = (size < available) ? size : available;
    memcpy(buf, n->content + offset, to_copy);
    *out_len = to_copy;
    return 0;
}

/* -------------------------------------------------------------------------
 * Control-path mutating operations
 * ---------------------------------------------------------------------- */

int vfs_create_file(vfs_t *vfs, const char *path,
                    const uint8_t *content, size_t content_len)
{
    char name[VFS_MAX_NAME + 1];
    errno = 0;
    vfs_node_t *parent = resolve_parent(vfs, path, name);
    if (!parent) return -errno;
    if (parent->kind != VFS_DIR) return -ENOTDIR;
    if (dir_lookup_child(parent, name)) return -EEXIST;

    vfs_node_t *node = node_alloc(vfs, VFS_FILE);
    if (!node) return -ENOMEM;

    if (content_len > 0) {
        node->content = malloc(content_len);
        if (!node->content) { node_free_self(node); return -ENOMEM; }
        memcpy(node->content, content, content_len);
    }
    node->content_len = content_len;
    node->parent      = parent;

    int r = dir_add_child(parent, name, node);
    if (r < 0) { node_free_self(node); return r; }
    return 0;
}

int vfs_update_file(vfs_t *vfs, const char *path,
                    const uint8_t *content, size_t content_len)
{
    errno = 0;
    vfs_node_t *n = resolve_path(vfs, path);
    if (!n) return -errno;
    if (n->kind == VFS_DIR) return -EISDIR;

    uint8_t *new_content = NULL;
    if (content_len > 0) {
        new_content = malloc(content_len);
        if (!new_content) return -ENOMEM;
        memcpy(new_content, content, content_len);
    }

    free(n->content);
    n->content     = new_content;
    n->content_len = content_len;
    n->mtime       = ts_now();
    return 0;
}

int vfs_delete_file(vfs_t *vfs, const char *path)
{
    char name[VFS_MAX_NAME + 1];
    errno = 0;
    vfs_node_t *parent = resolve_parent(vfs, path, name);
    if (!parent) return -errno;

    vfs_node_t *n = dir_lookup_child(parent, name);
    if (!n) return -ENOENT;
    if (n->kind == VFS_DIR) return -EISDIR;

    dir_remove_child(parent, name);
    node_free_deep(n);
    return 0;
}

int vfs_mkdir(vfs_t *vfs, const char *path)
{
    char name[VFS_MAX_NAME + 1];
    errno = 0;
    vfs_node_t *parent = resolve_parent(vfs, path, name);
    if (!parent) return -errno;
    if (parent->kind != VFS_DIR) return -ENOTDIR;
    if (dir_lookup_child(parent, name)) return -EEXIST;

    vfs_node_t *node = node_alloc(vfs, VFS_DIR);
    if (!node) return -ENOMEM;
    node->parent = parent;

    int r = dir_add_child(parent, name, node);
    if (r < 0) { node_free_self(node); return r; }
    return 0;
}

int vfs_rmdir(vfs_t *vfs, const char *path)
{
    if (strcmp(path, "/") == 0) return -EINVAL;

    char name[VFS_MAX_NAME + 1];
    errno = 0;
    vfs_node_t *parent = resolve_parent(vfs, path, name);
    if (!parent) return -errno;

    vfs_node_t *n = dir_lookup_child(parent, name);
    if (!n) return -ENOENT;
    if (n->kind != VFS_DIR) return -ENOTDIR;
    if (n->children) return -ENOTEMPTY;

    dir_remove_child(parent, name);
    node_free_deep(n);
    return 0;
}

int vfs_set_times(vfs_t *vfs, const char *path,
                  const struct timespec *mtime,
                  const struct timespec *atime)
{
    errno = 0;
    vfs_node_t *n = resolve_path(vfs, path);
    if (!n) return -errno;
    if (mtime) n->mtime = *mtime;
    if (atime) n->atime = *atime;
    return 0;
}

/* -------------------------------------------------------------------------
 * Snapshot and reset
 * ---------------------------------------------------------------------- */

int vfs_save_snapshot(vfs_t *vfs)
{
    vfs_node_t *copy = node_deepcopy(vfs->root, NULL);
    if (!copy) return -ENOMEM;

    node_free_deep(vfs->snapshot);
    vfs->snapshot = copy;
    return 0;
}

int vfs_reset_to_snapshot(vfs_t *vfs)
{
    if (!vfs->snapshot) return -EINVAL;

    /* Deep-copy the snapshot so the snapshot itself is never mutated. */
    vfs_node_t *copy = node_deepcopy(vfs->snapshot, NULL);
    if (!copy) return -ENOMEM;

    node_free_deep(vfs->root);
    vfs->root = copy;
    return 0;
}
