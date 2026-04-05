/*
 * vfs_test.c — unit tests for the in-memory VFS core.
 *
 * Covers (per Week 2 requirements):
 *   - path parsing and normalization
 *   - successful and failing lookup cases
 *   - partial reads and offset behavior
 *   - create / update / delete sequences
 *   - invalid operations and error codes
 *   - reset-to-baseline behavior
 *   - a randomized mutation-sequence test
 */

#include "vfs.h"

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* -------------------------------------------------------------------------
 * Minimal test framework
 * ---------------------------------------------------------------------- */

static int g_pass  = 0;
static int g_fail  = 0;
static const char *g_suite = "";

#define SUITE(name)  do { g_suite = (name); printf("\n%s\n", name); } while(0)

#define CHECK(cond)                                                         \
    do {                                                                    \
        if (cond) {                                                         \
            g_pass++;                                                       \
        } else {                                                            \
            g_fail++;                                                       \
            fprintf(stderr, "  FAIL  %s  line %d:  %s\n",                  \
                    g_suite, __LINE__, #cond);                              \
        }                                                                   \
    } while (0)

#define CHECK_EQ(a, b)  CHECK((a) == (b))
#define CHECK_NE(a, b)  CHECK((a) != (b))
#define CHECK_NULL(p)   CHECK((p) == NULL)
#define CHECK_NOTNULL(p) CHECK((p) != NULL)

/* -------------------------------------------------------------------------
 * Helpers
 * ---------------------------------------------------------------------- */

static const uint8_t *S(const char *s) { return (const uint8_t *)s; }
static size_t SL(const char *s) { return strlen(s); }

/*
 * readdir accumulator: collect entry names into a flat buffer.
 * ctx must point to a readdir_result.
 */
#define MAX_ENTRIES 64
#define MAX_NAME_LEN 256

typedef struct {
    char names[MAX_ENTRIES][MAX_NAME_LEN];
    int  count;
} readdir_result_t;

static int collect_entries(void *ctx, const char *name, const vfs_stat_t *st)
{
    (void)st;
    readdir_result_t *r = ctx;
    if (r->count >= MAX_ENTRIES) return -1; /* shouldn't happen in tests */
    strncpy(r->names[r->count], name, MAX_NAME_LEN - 1);
    r->names[r->count][MAX_NAME_LEN - 1] = '\0';
    r->count++;
    return 0;
}

static int has_entry(const readdir_result_t *r, const char *name)
{
    for (int i = 0; i < r->count; i++)
        if (strcmp(r->names[i], name) == 0) return 1;
    return 0;
}

/* Read the full content of a VFS file into a freshly malloc'd buffer. */
static char *read_all(vfs_t *vfs, const char *path)
{
    vfs_stat_t st;
    if (vfs_getattr(vfs, path, &st) != 0) return NULL;
    char *buf = malloc(st.size + 1);
    if (!buf) return NULL;
    size_t got = 0;
    if (vfs_read(vfs, path, 0, st.size, (uint8_t *)buf, &got) != 0) {
        free(buf);
        return NULL;
    }
    buf[got] = '\0';
    return buf;
}

/* -------------------------------------------------------------------------
 * Path parsing and normalization
 * ---------------------------------------------------------------------- */

static void test_path_parsing(void)
{
    SUITE("path_parsing");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    /* Create nodes used by trailing-slash and ENAMETOOLONG tests below. */
    CHECK_EQ(vfs_create_file(vfs, "/tfile", S("x"), 1), 0);
    CHECK_EQ(vfs_mkdir(vfs, "/tdir"), 0);

    /* Root always resolves. */
    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/", &st), 0);
    CHECK_EQ(st.kind, VFS_DIR);

    /* Missing leading slash → EINVAL. */
    CHECK_EQ(vfs_getattr(vfs, "", &st), -EINVAL);
    CHECK_EQ(vfs_getattr(vfs, "foo", &st), -EINVAL);
    CHECK_EQ(vfs_getattr(vfs, "foo/bar", &st), -EINVAL);

    /* "." and ".." components → EINVAL. */
    CHECK_EQ(vfs_getattr(vfs, "/.", &st), -EINVAL);
    CHECK_EQ(vfs_getattr(vfs, "/..", &st), -EINVAL);
    CHECK_EQ(vfs_getattr(vfs, "/../bar", &st), -EINVAL);
    CHECK_EQ(vfs_getattr(vfs, "/./bar", &st), -EINVAL);

    /* Double slash → EINVAL. */
    CHECK_EQ(vfs_getattr(vfs, "//foo", &st), -EINVAL);

    /* Trailing slash → EINVAL (requires an existing node to reach the slash). */
    CHECK_EQ(vfs_getattr(vfs, "/tfile/", &st), -EINVAL);
    CHECK_EQ(vfs_getattr(vfs, "/tdir/", &st), -EINVAL);

    /* Name longer than VFS_MAX_NAME (255) → ENAMETOOLONG. */
    char long_path[260];
    long_path[0] = '/';
    memset(long_path + 1, 'x', 256);
    long_path[257] = '\0';
    CHECK_EQ(vfs_getattr(vfs, long_path, &st), -ENAMETOOLONG);

    /* Non-existent path → ENOENT. */
    CHECK_EQ(vfs_getattr(vfs, "/nonexistent", &st), -ENOENT);
    CHECK_EQ(vfs_getattr(vfs, "/a/b/c", &st), -ENOENT);

    /* NULL path → EINVAL. */
    CHECK_EQ(vfs_getattr(vfs, NULL, &st), -EINVAL);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * create_file: success and failure cases
 * ---------------------------------------------------------------------- */

static void test_create_file(void)
{
    SUITE("create_file");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    /* Create a file at root level. */
    CHECK_EQ(vfs_create_file(vfs, "/a", S("hello"), SL("hello")), 0);

    /* getattr sees it as a regular file with the right size. */
    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/a", &st), 0);
    CHECK_EQ(st.kind, VFS_FILE);
    CHECK_EQ(st.size, SL("hello"));

    /* Creating a duplicate returns EEXIST. */
    CHECK_EQ(vfs_create_file(vfs, "/a", S("world"), SL("world")), -EEXIST);

    /* Creating in a non-existent directory returns ENOENT. */
    CHECK_EQ(vfs_create_file(vfs, "/nodir/x", S("x"), 1), -ENOENT);

    /* Path with a file used as a directory component returns ENOTDIR. */
    CHECK_EQ(vfs_create_file(vfs, "/a/child", S("x"), 1), -ENOTDIR);

    /* Trailing slash in control path → EINVAL (empty final component). */
    CHECK_EQ(vfs_create_file(vfs, "/a/", S("x"), 1), -EINVAL);

    /* "." and ".." as final path component → EINVAL. */
    CHECK_EQ(vfs_create_file(vfs, "/.", S("x"), 1), -EINVAL);
    CHECK_EQ(vfs_create_file(vfs, "/..", S("x"), 1), -EINVAL);

    /* Double slash in parent portion → EINVAL (parent path has trailing slash). */
    CHECK_EQ(vfs_mkdir(vfs, "/mdir"), 0);
    CHECK_EQ(vfs_create_file(vfs, "/mdir//f", S("x"), 1), -EINVAL);

    /* Empty content is allowed. */
    CHECK_EQ(vfs_create_file(vfs, "/empty", NULL, 0), 0);
    CHECK_EQ(vfs_getattr(vfs, "/empty", &st), 0);
    CHECK_EQ(st.size, (size_t)0);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * mkdir: success and failure cases
 * ---------------------------------------------------------------------- */

static void test_mkdir(void)
{
    SUITE("mkdir");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    CHECK_EQ(vfs_mkdir(vfs, "/d"), 0);
    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/d", &st), 0);
    CHECK_EQ(st.kind, VFS_DIR);

    /* Duplicate → EEXIST. */
    CHECK_EQ(vfs_mkdir(vfs, "/d"), -EEXIST);

    /* Parent doesn't exist → ENOENT. */
    CHECK_EQ(vfs_mkdir(vfs, "/x/y"), -ENOENT);

    /* Nested mkdir works when parent exists. */
    CHECK_EQ(vfs_mkdir(vfs, "/d/e"), 0);
    CHECK_EQ(vfs_getattr(vfs, "/d/e", &st), 0);
    CHECK_EQ(st.kind, VFS_DIR);

    /* mkdir where a file with the same name exists → EEXIST. */
    CHECK_EQ(vfs_create_file(vfs, "/f", S("x"), 1), 0);
    CHECK_EQ(vfs_mkdir(vfs, "/f"), -EEXIST);

    /* "." and ".." as final component → EINVAL. */
    CHECK_EQ(vfs_mkdir(vfs, "/."), -EINVAL);
    CHECK_EQ(vfs_mkdir(vfs, "/.."), -EINVAL);

    /* Trailing slash → EINVAL. */
    CHECK_EQ(vfs_mkdir(vfs, "/newdir/"), -EINVAL);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * readdir: listing behavior
 * ---------------------------------------------------------------------- */

static void test_readdir(void)
{
    SUITE("readdir");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    /* Empty root: only "." and "..". */
    readdir_result_t res;
    memset(&res, 0, sizeof(res));
    CHECK_EQ(vfs_readdir(vfs, "/", collect_entries, &res), 0);
    CHECK_EQ(res.count, 2);
    CHECK(has_entry(&res, "."));
    CHECK(has_entry(&res, ".."));

    /* Add two files; they appear in the listing. */
    vfs_create_file(vfs, "/a", S("1"), 1);
    vfs_create_file(vfs, "/b", S("2"), 1);

    memset(&res, 0, sizeof(res));
    CHECK_EQ(vfs_readdir(vfs, "/", collect_entries, &res), 0);
    CHECK_EQ(res.count, 4);  /* ".", "..", "a", "b" */
    CHECK(has_entry(&res, "a"));
    CHECK(has_entry(&res, "b"));

    /* Listing a regular file → ENOTDIR. */
    CHECK_EQ(vfs_readdir(vfs, "/a", collect_entries, &res), -ENOTDIR);

    /* Listing a non-existent path → ENOENT. */
    CHECK_EQ(vfs_readdir(vfs, "/nope", collect_entries, &res), -ENOENT);

    /* Nested directory listing. */
    vfs_mkdir(vfs, "/d");
    vfs_create_file(vfs, "/d/x", S("x"), 1);

    memset(&res, 0, sizeof(res));
    CHECK_EQ(vfs_readdir(vfs, "/d", collect_entries, &res), 0);
    CHECK_EQ(res.count, 3);  /* ".", "..", "x" */
    CHECK(has_entry(&res, "x"));

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * read: offsets and partial reads
 * ---------------------------------------------------------------------- */

static void test_read(void)
{
    SUITE("read");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    vfs_create_file(vfs, "/f", S("hello"), SL("hello"));

    uint8_t buf[64];
    size_t  got;

    /* Full read. */
    CHECK_EQ(vfs_read(vfs, "/f", 0, 64, buf, &got), 0);
    CHECK_EQ(got, SL("hello"));
    CHECK_EQ(memcmp(buf, "hello", got), 0);

    /* Partial read from offset 0. */
    CHECK_EQ(vfs_read(vfs, "/f", 0, 2, buf, &got), 0);
    CHECK_EQ(got, (size_t)2);
    CHECK_EQ(memcmp(buf, "he", 2), 0);

    /* Read from middle. */
    CHECK_EQ(vfs_read(vfs, "/f", 2, 3, buf, &got), 0);
    CHECK_EQ(got, (size_t)3);
    CHECK_EQ(memcmp(buf, "llo", 3), 0);

    /* Read from last byte. */
    CHECK_EQ(vfs_read(vfs, "/f", 4, 10, buf, &got), 0);
    CHECK_EQ(got, (size_t)1);
    CHECK_EQ(buf[0], 'o');

    /* Read at exactly end-of-file → 0 bytes, no error. */
    CHECK_EQ(vfs_read(vfs, "/f", SL("hello"), 10, buf, &got), 0);
    CHECK_EQ(got, (size_t)0);

    /* Read beyond end-of-file → 0 bytes, no error. */
    CHECK_EQ(vfs_read(vfs, "/f", 1000, 10, buf, &got), 0);
    CHECK_EQ(got, (size_t)0);

    /* size == 0 → 0 bytes, no error. */
    CHECK_EQ(vfs_read(vfs, "/f", 0, 0, buf, &got), 0);
    CHECK_EQ(got, (size_t)0);

    /* Read from directory → EISDIR. */
    CHECK_EQ(vfs_read(vfs, "/", 0, 10, buf, &got), -EISDIR);

    /* Read from non-existent path → ENOENT. */
    CHECK_EQ(vfs_read(vfs, "/nope", 0, 10, buf, &got), -ENOENT);

    /* Empty file: read returns 0 bytes. */
    vfs_create_file(vfs, "/e", NULL, 0);
    CHECK_EQ(vfs_read(vfs, "/e", 0, 10, buf, &got), 0);
    CHECK_EQ(got, (size_t)0);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * update_file
 * ---------------------------------------------------------------------- */

static void test_update_file(void)
{
    SUITE("update_file");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    vfs_create_file(vfs, "/f", S("original"), SL("original"));

    /* Update to new content. */
    CHECK_EQ(vfs_update_file(vfs, "/f", S("updated"), SL("updated")), 0);
    char *got = read_all(vfs, "/f");
    CHECK_NOTNULL(got);
    CHECK_EQ(strcmp(got, "updated"), 0);
    free(got);

    /* Update to empty content. */
    CHECK_EQ(vfs_update_file(vfs, "/f", NULL, 0), 0);
    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/f", &st), 0);
    CHECK_EQ(st.size, (size_t)0);

    /* Update non-existent → ENOENT. */
    CHECK_EQ(vfs_update_file(vfs, "/nope", S("x"), 1), -ENOENT);

    /* Update a directory → EISDIR. */
    vfs_mkdir(vfs, "/d");
    CHECK_EQ(vfs_update_file(vfs, "/d", S("x"), 1), -EISDIR);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * delete_file
 * ---------------------------------------------------------------------- */

static void test_delete_file(void)
{
    SUITE("delete_file");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    vfs_create_file(vfs, "/f", S("data"), SL("data"));
    CHECK_EQ(vfs_delete_file(vfs, "/f"), 0);

    /* File is gone. */
    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/f", &st), -ENOENT);

    /* Deleting again → ENOENT. */
    CHECK_EQ(vfs_delete_file(vfs, "/f"), -ENOENT);

    /* Trying to delete a directory with delete_file → EISDIR. */
    vfs_mkdir(vfs, "/d");
    CHECK_EQ(vfs_delete_file(vfs, "/d"), -EISDIR);

    /* Deleting non-existent → ENOENT. */
    CHECK_EQ(vfs_delete_file(vfs, "/nope"), -ENOENT);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * rmdir
 * ---------------------------------------------------------------------- */

static void test_rmdir(void)
{
    SUITE("rmdir");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    /* Remove an empty directory. */
    vfs_mkdir(vfs, "/d");
    CHECK_EQ(vfs_rmdir(vfs, "/d"), 0);
    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/d", &st), -ENOENT);

    /* Remove non-existent → ENOENT. */
    CHECK_EQ(vfs_rmdir(vfs, "/d"), -ENOENT);

    /* Remove non-empty → ENOTEMPTY. */
    vfs_mkdir(vfs, "/d");
    vfs_create_file(vfs, "/d/f", S("x"), 1);
    CHECK_EQ(vfs_rmdir(vfs, "/d"), -ENOTEMPTY);

    /* rmdir on a file → ENOTDIR. */
    vfs_create_file(vfs, "/f", S("x"), 1);
    CHECK_EQ(vfs_rmdir(vfs, "/f"), -ENOTDIR);

    /* rmdir "/" → EINVAL (root is protected). */
    CHECK_EQ(vfs_rmdir(vfs, "/"), -EINVAL);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * Nested structure
 * ---------------------------------------------------------------------- */

static void test_nested(void)
{
    SUITE("nested");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    /* Build /a/b/c with a file at /a/b/c/data. */
    CHECK_EQ(vfs_mkdir(vfs, "/a"), 0);
    CHECK_EQ(vfs_mkdir(vfs, "/a/b"), 0);
    CHECK_EQ(vfs_mkdir(vfs, "/a/b/c"), 0);
    CHECK_EQ(vfs_create_file(vfs, "/a/b/c/data", S("deep"), SL("deep")), 0);

    char *got = read_all(vfs, "/a/b/c/data");
    CHECK_NOTNULL(got);
    CHECK_EQ(strcmp(got, "deep"), 0);
    free(got);

    /* readdir at each level. */
    readdir_result_t res;

    memset(&res, 0, sizeof(res));
    vfs_readdir(vfs, "/a", collect_entries, &res);
    CHECK(has_entry(&res, "b"));

    memset(&res, 0, sizeof(res));
    vfs_readdir(vfs, "/a/b", collect_entries, &res);
    CHECK(has_entry(&res, "c"));

    memset(&res, 0, sizeof(res));
    vfs_readdir(vfs, "/a/b/c", collect_entries, &res);
    CHECK(has_entry(&res, "data"));

    /* Attempting to lookup past a file returns ENOTDIR. */
    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/a/b/c/data/x", &st), -ENOTDIR);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * create / update / delete sequence
 * ---------------------------------------------------------------------- */

static void test_mutation_sequence(void)
{
    SUITE("mutation_sequence");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    /* Create three files. */
    vfs_create_file(vfs, "/x", S("X"), 1);
    vfs_create_file(vfs, "/y", S("Y"), 1);
    vfs_create_file(vfs, "/z", S("Z"), 1);

    /* Update /y. */
    CHECK_EQ(vfs_update_file(vfs, "/y", S("YY"), 2), 0);
    char *got = read_all(vfs, "/y");
    CHECK_NOTNULL(got);
    CHECK_EQ(strcmp(got, "YY"), 0);
    free(got);

    /* Delete /z. */
    CHECK_EQ(vfs_delete_file(vfs, "/z"), 0);
    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/z", &st), -ENOENT);

    /* /x and /y still exist unchanged (apart from /y's update). */
    CHECK_EQ(vfs_getattr(vfs, "/x", &st), 0);
    got = read_all(vfs, "/x");
    CHECK_NOTNULL(got);
    CHECK_EQ(strcmp(got, "X"), 0);
    free(got);

    /* Create a directory, put a file in it, verify, then clean up. */
    vfs_mkdir(vfs, "/dir");
    vfs_create_file(vfs, "/dir/child", S("child"), SL("child"));
    got = read_all(vfs, "/dir/child");
    CHECK_NOTNULL(got);
    CHECK_EQ(strcmp(got, "child"), 0);
    free(got);

    vfs_delete_file(vfs, "/dir/child");
    vfs_rmdir(vfs, "/dir");
    CHECK_EQ(vfs_getattr(vfs, "/dir", &st), -ENOENT);

    /* Root still works throughout. */
    CHECK_EQ(vfs_getattr(vfs, "/", &st), 0);
    CHECK_EQ(st.kind, VFS_DIR);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * Snapshot and reset
 * ---------------------------------------------------------------------- */

static void test_snapshot_reset(void)
{
    SUITE("snapshot_reset");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    /* Reset with no snapshot → EINVAL. */
    CHECK_EQ(vfs_reset_to_snapshot(vfs), -EINVAL);

    /* Set up baseline: /a with content "base". */
    vfs_create_file(vfs, "/a", S("base"), SL("base"));
    CHECK_EQ(vfs_save_snapshot(vfs), 0);

    /* Mutate: add /b, change /a. */
    vfs_create_file(vfs, "/b", S("new"), SL("new"));
    vfs_update_file(vfs, "/a", S("changed"), SL("changed"));

    /* Verify mutations took effect. */
    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/b", &st), 0);
    char *got = read_all(vfs, "/a");
    CHECK_NOTNULL(got);
    CHECK_EQ(strcmp(got, "changed"), 0);
    free(got);

    /* Reset to snapshot. */
    CHECK_EQ(vfs_reset_to_snapshot(vfs), 0);

    /* /a is back to "base". */
    got = read_all(vfs, "/a");
    CHECK_NOTNULL(got);
    CHECK_EQ(strcmp(got, "base"), 0);
    free(got);

    /* /b is gone. */
    CHECK_EQ(vfs_getattr(vfs, "/b", &st), -ENOENT);

    /* Snapshot is preserved: reset can be called multiple times. */
    vfs_update_file(vfs, "/a", S("mutated_again"), SL("mutated_again"));
    CHECK_EQ(vfs_reset_to_snapshot(vfs), 0);
    got = read_all(vfs, "/a");
    CHECK_NOTNULL(got);
    CHECK_EQ(strcmp(got, "base"), 0);
    free(got);

    /* Overwrite snapshot with new state. */
    vfs_update_file(vfs, "/a", S("v2"), SL("v2"));
    CHECK_EQ(vfs_save_snapshot(vfs), 0);
    vfs_update_file(vfs, "/a", S("v3"), SL("v3"));
    CHECK_EQ(vfs_reset_to_snapshot(vfs), 0);
    got = read_all(vfs, "/a");
    CHECK_NOTNULL(got);
    CHECK_EQ(strcmp(got, "v2"), 0);
    free(got);

    /* Root survives reset. */
    CHECK_EQ(vfs_getattr(vfs, "/", &st), 0);
    CHECK_EQ(st.kind, VFS_DIR);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * Snapshot with nested structure
 * ---------------------------------------------------------------------- */

static void test_snapshot_nested(void)
{
    SUITE("snapshot_nested");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    /* Build /d/f with content "orig". */
    vfs_mkdir(vfs, "/d");
    vfs_create_file(vfs, "/d/f", S("orig"), SL("orig"));
    CHECK_EQ(vfs_save_snapshot(vfs), 0);

    /* Mutate deeply. */
    vfs_update_file(vfs, "/d/f", S("mutated"), SL("mutated"));
    vfs_create_file(vfs, "/d/g", S("extra"), SL("extra"));
    vfs_mkdir(vfs, "/d/sub");

    /* Reset. */
    CHECK_EQ(vfs_reset_to_snapshot(vfs), 0);

    /* Restored structure. */
    char *got = read_all(vfs, "/d/f");
    CHECK_NOTNULL(got);
    CHECK_EQ(strcmp(got, "orig"), 0);
    free(got);

    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/d/g", &st), -ENOENT);
    CHECK_EQ(vfs_getattr(vfs, "/d/sub", &st), -ENOENT);
    CHECK_EQ(vfs_getattr(vfs, "/d", &st), 0);
    CHECK_EQ(st.kind, VFS_DIR);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * Invariants
 * ---------------------------------------------------------------------- */

static void test_invariants(void)
{
    SUITE("invariants");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    /* Root always has kind VFS_DIR. */
    vfs_stat_t st;
    CHECK_EQ(vfs_getattr(vfs, "/", &st), 0);
    CHECK_EQ(st.kind, VFS_DIR);

    /* Root cannot be removed. */
    CHECK_EQ(vfs_rmdir(vfs, "/"), -EINVAL);

    /* Root is still there after the failed rmdir. */
    CHECK_EQ(vfs_getattr(vfs, "/", &st), 0);

    /* A file and a directory cannot share the same name. */
    vfs_create_file(vfs, "/name", S("x"), 1);
    CHECK_EQ(vfs_mkdir(vfs, "/name"), -EEXIST);

    vfs_mkdir(vfs, "/dirname");
    CHECK_EQ(vfs_create_file(vfs, "/dirname", S("x"), 1), -EEXIST);

    /* Parent must exist for both create_file and mkdir. */
    CHECK_EQ(vfs_create_file(vfs, "/nodir/f", S("x"), 1), -ENOENT);
    CHECK_EQ(vfs_mkdir(vfs, "/nodir/d"), -ENOENT);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * Randomized mutation-sequence test
 *
 * Applies a pseudo-random sequence of create/update/delete/mkdir/rmdir
 * operations, saves a snapshot at a random point, continues mutating,
 * then resets and verifies the state matches what was snapshotted.
 * ---------------------------------------------------------------------- */

/*
 * Compute a simple 32-bit hash of a string (FNV-1a variant).
 * Used only as a deterministic content fingerprint in the test.
 */
static uint32_t fnv1a(const char *s)
{
    uint32_t h = 2166136261u;
    while (*s) {
        h ^= (uint8_t)*s++;
        h *= 16777619u;
    }
    return h;
}

#define N_PATHS 8

static const char *rand_paths[N_PATHS] = {
    "/r0", "/r1", "/r2", "/r3",
    "/r4", "/r5", "/r6", "/r7",
};

static void test_random_sequence(void)
{
    SUITE("random_sequence");

    vfs_t *vfs = vfs_create();
    CHECK_NOTNULL(vfs);

    uint32_t seed = 0xdeadbeef;

    /*
     * Phase 1: Apply 200 random operations.
     * We keep a parallel boolean array tracking which paths are files,
     * which are dirs, and which are absent.
     */
    enum { ABSENT, IS_FILE, IS_DIR } state[N_PATHS] = {0};

    for (int op = 0; op < 200; op++) {
        /* Xorshift32 */
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;

        int    idx    = (int)(seed % N_PATHS);
        int    action = (int)((seed >> 8) % 5); /* 0-4 */
        char   content[16];
        snprintf(content, sizeof(content), "v%u", fnv1a(rand_paths[idx]) ^ seed);

        switch (action) {
        case 0: /* create_file */
            if (state[idx] == ABSENT) {
                int r = vfs_create_file(vfs, rand_paths[idx],
                                        S(content), strlen(content));
                CHECK_EQ(r, 0);
                state[idx] = IS_FILE;
            } else {
                CHECK_NE(vfs_create_file(vfs, rand_paths[idx],
                                         S(content), strlen(content)), 0);
            }
            break;

        case 1: /* update_file */
            if (state[idx] == IS_FILE) {
                CHECK_EQ(vfs_update_file(vfs, rand_paths[idx],
                                          S(content), strlen(content)), 0);
            } else if (state[idx] == IS_DIR) {
                CHECK_EQ(vfs_update_file(vfs, rand_paths[idx],
                                          S(content), strlen(content)), -EISDIR);
            } else {
                CHECK_EQ(vfs_update_file(vfs, rand_paths[idx],
                                          S(content), strlen(content)), -ENOENT);
            }
            break;

        case 2: /* delete_file */
            if (state[idx] == IS_FILE) {
                CHECK_EQ(vfs_delete_file(vfs, rand_paths[idx]), 0);
                state[idx] = ABSENT;
            } else if (state[idx] == IS_DIR) {
                CHECK_EQ(vfs_delete_file(vfs, rand_paths[idx]), -EISDIR);
            } else {
                CHECK_EQ(vfs_delete_file(vfs, rand_paths[idx]), -ENOENT);
            }
            break;

        case 3: /* mkdir */
            if (state[idx] == ABSENT) {
                CHECK_EQ(vfs_mkdir(vfs, rand_paths[idx]), 0);
                state[idx] = IS_DIR;
            } else {
                CHECK_NE(vfs_mkdir(vfs, rand_paths[idx]), 0);
            }
            break;

        case 4: /* rmdir (only if empty dir) */
            if (state[idx] == IS_DIR) {
                /* These rand_paths are all at the root level with no children,
                 * so rmdir should always succeed for an IS_DIR entry here. */
                CHECK_EQ(vfs_rmdir(vfs, rand_paths[idx]), 0);
                state[idx] = ABSENT;
            } else if (state[idx] == IS_FILE) {
                CHECK_EQ(vfs_rmdir(vfs, rand_paths[idx]), -ENOTDIR);
            } else {
                CHECK_EQ(vfs_rmdir(vfs, rand_paths[idx]), -ENOENT);
            }
            break;
        }
    }

    /* Phase 2: Save snapshot, record what currently exists. */
    CHECK_EQ(vfs_save_snapshot(vfs), 0);

    int snapshot_state[N_PATHS];
    for (int i = 0; i < N_PATHS; i++) snapshot_state[i] = state[i];

    /* Phase 3: Apply 100 more random mutations. */
    for (int op = 0; op < 100; op++) {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;

        int idx    = (int)(seed % N_PATHS);
        int action = (int)((seed >> 8) % 3);
        char content[16];
        snprintf(content, sizeof(content), "post%u", seed);

        switch (action) {
        case 0:
            if (state[idx] == ABSENT) {
                vfs_create_file(vfs, rand_paths[idx], S(content), strlen(content));
                state[idx] = IS_FILE;
            }
            break;
        case 1:
            if (state[idx] == IS_FILE) {
                vfs_update_file(vfs, rand_paths[idx], S(content), strlen(content));
            }
            break;
        case 2:
            if (state[idx] == IS_FILE) {
                vfs_delete_file(vfs, rand_paths[idx]);
                state[idx] = ABSENT;
            } else if (state[idx] == IS_DIR) {
                vfs_rmdir(vfs, rand_paths[idx]);
                state[idx] = ABSENT;
            }
            break;
        }
    }

    /* Phase 4: Reset to snapshot and verify structure matches snapshot_state. */
    CHECK_EQ(vfs_reset_to_snapshot(vfs), 0);

    vfs_stat_t st;
    for (int i = 0; i < N_PATHS; i++) {
        int r = vfs_getattr(vfs, rand_paths[i], &st);
        if (snapshot_state[i] == ABSENT) {
            CHECK_EQ(r, -ENOENT);
        } else if (snapshot_state[i] == IS_FILE) {
            CHECK_EQ(r, 0);
            CHECK_EQ(st.kind, VFS_FILE);
        } else { /* IS_DIR */
            CHECK_EQ(r, 0);
            CHECK_EQ(st.kind, VFS_DIR);
        }
    }

    /* Root must survive. */
    CHECK_EQ(vfs_getattr(vfs, "/", &st), 0);
    CHECK_EQ(st.kind, VFS_DIR);

    vfs_destroy(vfs);
}

/* -------------------------------------------------------------------------
 * main
 * ---------------------------------------------------------------------- */

int main(void)
{
    test_path_parsing();
    test_create_file();
    test_mkdir();
    test_readdir();
    test_read();
    test_update_file();
    test_delete_file();
    test_rmdir();
    test_nested();
    test_mutation_sequence();
    test_snapshot_reset();
    test_snapshot_nested();
    test_invariants();
    test_random_sequence();

    printf("\n");
    if (g_fail == 0) {
        printf("All %d checks passed.\n", g_pass);
        return 0;
    } else {
        printf("%d/%d checks FAILED.\n", g_fail, g_pass + g_fail);
        return 1;
    }
}
