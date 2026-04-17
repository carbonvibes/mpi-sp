/*
 * cp_test.c — Test suite for the Week 4 control plane.
 *
 * Covers:
 *   delta_lifecycle       — create, add all 7 op kinds, free
 *   delta_serialize       — serialize → deserialize roundtrip for all kinds
 *   delta_deser_errors    — truncated buf, bad magic, bad kind, bad path
 *   delta_checksum        — same data → same hash; different data → different
 *   ensure_parents        — basic, deep, already-exists, root, bad path
 *   apply_basic           — all 7 op kinds through cp_apply_delta
 *   apply_ensure_parents  — CREATE_FILE before MKDIR parent succeeds
 *   apply_rmdir_ordering  — shallowest-first RMDIR list is reordered to work
 *   apply_errors          — ENOENT, EISDIR, ENOTEMPTY from delta ops
 *   apply_set_times       — SET_TIMES op reaches VFS and is verified
 *   apply_truncate        — TRUNCATE shrink and extend
 *   apply_dry_run         — dry_run=1 shows tree; VFS unchanged after
 *   apply_mutate_reset    — 10 iterations of apply + vfs_reset_to_snapshot
 *   vfs_checksum          — identical tree → same hash; mutation changes hash
 *
 * Build: see control_plane/Makefile
 * Run:   ./cp_test
 */

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "../vfs/vfs.h"
#include "control_plane.h"
#include "delta.h"

/* -------------------------------------------------------------------------
 * Check harness (matches style of vfs_test.c)
 * ---------------------------------------------------------------------- */

static int g_checks   = 0;
static int g_failures = 0;

#define CHECK(expr) \
    do { \
        g_checks++; \
        if (!(expr)) { \
            g_failures++; \
            fprintf(stderr, "FAIL %s:%d  %s\n", __FILE__, __LINE__, #expr); \
        } \
    } while (0)

/* -------------------------------------------------------------------------
 * 1. delta_lifecycle
 * ---------------------------------------------------------------------- */

static void test_delta_lifecycle(void)
{
    printf("  delta_lifecycle\n");

    fs_delta_t *d = delta_create();
    CHECK(d != NULL);
    CHECK(d->n_ops == 0);

    const uint8_t bytes[] = {0x41, 0x42, 0x43};
    struct timespec ts = { .tv_sec = 1000, .tv_nsec = 500 };

    CHECK(delta_add_create_file(d, "/a.txt", bytes, 3) == 0);
    CHECK(delta_add_update_file(d, "/a.txt", bytes, 3) == 0);
    CHECK(delta_add_delete_file(d, "/a.txt") == 0);
    CHECK(delta_add_mkdir(d, "/mydir") == 0);
    CHECK(delta_add_rmdir(d, "/mydir") == 0);
    CHECK(delta_add_set_times(d, "/a.txt", &ts, &ts) == 0);
    CHECK(delta_add_truncate(d, "/a.txt", 100) == 0);

    CHECK(d->n_ops == 7);

    /* Verify kind fields. */
    CHECK(d->ops[0].kind == FS_OP_CREATE_FILE);
    CHECK(d->ops[1].kind == FS_OP_UPDATE_FILE);
    CHECK(d->ops[2].kind == FS_OP_DELETE_FILE);
    CHECK(d->ops[3].kind == FS_OP_MKDIR);
    CHECK(d->ops[4].kind == FS_OP_RMDIR);
    CHECK(d->ops[5].kind == FS_OP_SET_TIMES);
    CHECK(d->ops[6].kind == FS_OP_TRUNCATE);

    /* CREATE_FILE: path and content deep-copied. */
    CHECK(strcmp(d->ops[0].path, "/a.txt") == 0);
    CHECK(d->ops[0].content_len == 3);
    CHECK(d->ops[0].content != NULL);
    CHECK(d->ops[0].content[0] == 0x41);

    /* TRUNCATE: new_size stored in content_len, content is NULL. */
    CHECK(d->ops[6].content_len == 100);
    CHECK(d->ops[6].content == NULL);

    /* SET_TIMES: timestamps copied. */
    CHECK(d->ops[5].mtime.tv_sec == 1000);
    CHECK(d->ops[5].atime.tv_nsec == 500);

    delta_free(d);
    /* Double-free must not crash (calling free(NULL) is OK). */
    delta_free(NULL);
}

/* -------------------------------------------------------------------------
 * 2. delta_serialize — roundtrip for all 7 op kinds
 * ---------------------------------------------------------------------- */

static void test_delta_serialize(void)
{
    printf("  delta_serialize\n");

    fs_delta_t *orig = delta_create();
    CHECK(orig != NULL);

    const uint8_t content[] = {0x01, 0x02, 0x03, 0x04};
    struct timespec mt = { .tv_sec = 9999, .tv_nsec = 123456789 };
    struct timespec at = { .tv_sec = 8888, .tv_nsec = 987654321 };

    delta_add_create_file(orig, "/data/file.bin", content, 4);
    delta_add_update_file(orig, "/data/file.bin", content, 4);
    delta_add_delete_file(orig, "/data/file.bin");
    delta_add_mkdir(orig, "/data/subdir");
    delta_add_rmdir(orig, "/data/subdir");
    delta_add_set_times(orig, "/data/file.bin", &mt, &at);
    delta_add_truncate(orig, "/data/file.bin", 512);

    size_t len = 0;
    uint8_t *buf = delta_serialize(orig, &len);
    CHECK(buf != NULL);
    CHECK(len > 4);  /* at least header (n_ops u32) */

    int err = 0;
    fs_delta_t *copy = delta_deserialize(buf, len, &err);
    CHECK(err == 0);
    CHECK(copy != NULL);
    CHECK(copy->n_ops == 7);

    /* CREATE_FILE roundtrip */
    CHECK(copy->ops[0].kind == FS_OP_CREATE_FILE);
    CHECK(strcmp(copy->ops[0].path, "/data/file.bin") == 0);
    CHECK(copy->ops[0].content_len == 4);
    CHECK(copy->ops[0].content != NULL);
    CHECK(memcmp(copy->ops[0].content, content, 4) == 0);

    /* UPDATE_FILE roundtrip */
    CHECK(copy->ops[1].kind == FS_OP_UPDATE_FILE);
    CHECK(copy->ops[1].content_len == 4);
    CHECK(memcmp(copy->ops[1].content, content, 4) == 0);

    /* DELETE_FILE roundtrip */
    CHECK(copy->ops[2].kind == FS_OP_DELETE_FILE);
    CHECK(strcmp(copy->ops[2].path, "/data/file.bin") == 0);
    CHECK(copy->ops[2].content == NULL);
    CHECK(copy->ops[2].content_len == 0);

    /* MKDIR roundtrip */
    CHECK(copy->ops[3].kind == FS_OP_MKDIR);
    CHECK(strcmp(copy->ops[3].path, "/data/subdir") == 0);

    /* RMDIR roundtrip */
    CHECK(copy->ops[4].kind == FS_OP_RMDIR);

    /* SET_TIMES roundtrip: timestamps preserved */
    CHECK(copy->ops[5].kind == FS_OP_SET_TIMES);
    CHECK(copy->ops[5].mtime.tv_sec  == mt.tv_sec);
    CHECK(copy->ops[5].mtime.tv_nsec == mt.tv_nsec);
    CHECK(copy->ops[5].atime.tv_sec  == at.tv_sec);
    CHECK(copy->ops[5].atime.tv_nsec == at.tv_nsec);

    /* TRUNCATE: new_size is content_len; no content data. */
    CHECK(copy->ops[6].kind == FS_OP_TRUNCATE);
    CHECK(copy->ops[6].content_len == 512);
    CHECK(copy->ops[6].content == NULL);

    free(buf);
    delta_free(orig);
    delta_free(copy);

    /* Empty delta serializes to NULL. */
    fs_delta_t *empty = delta_create();
    size_t elen = 99;
    uint8_t *ebuf = delta_serialize(empty, &elen);
    CHECK(ebuf == NULL);
    CHECK(elen == 0);
    delta_free(empty);
}

/* -------------------------------------------------------------------------
 * 3. delta_deser_errors
 * ---------------------------------------------------------------------- */

static void test_delta_deser_errors(void)
{
    printf("  delta_deser_errors\n");

    int err;
    fs_delta_t *d;

    /* Buffer too short for header (need 4 bytes for n_ops). */
    uint8_t tiny[] = { 0x00, 0x00, 0x00 };
    d = delta_deserialize(tiny, 3, &err);
    CHECK(d == NULL && err < 0);

    /* n_ops claims far more ops than the buffer could possibly hold. */
    {
        /* Header says 0x00FFFFFF ops but buffer is only 4 bytes — too small. */
        uint8_t huge_ops[4] = { 0x00, 0xFF, 0xFF, 0xFF };
        d = delta_deserialize(huge_ops, 4, &err);
        CHECK(d == NULL && err < 0);
    }

    /* Zero n_ops (invalid). */
    uint8_t zero_ops[4] = { 0, 0, 0, 0 };
    d = delta_deserialize(zero_ops, 4, &err);
    CHECK(d == NULL && err < 0);

    /* Invalid op kind (0 is reserved). */
    {
        /* Build a valid 1-op delta then corrupt the kind byte. */
        fs_delta_t *src = delta_create();
        delta_add_mkdir(src, "/x");
        size_t len = 0;
        uint8_t *buf = delta_serialize(src, &len);
        delta_free(src);
        CHECK(buf != NULL);
        buf[4] = 0;  /* kind byte → 0 (reserved/invalid); header is 4 bytes */
        d = delta_deserialize(buf, len, &err);
        CHECK(d == NULL && err < 0);
        free(buf);
    }

    /* Path that does not start with '/'. */
    {
        fs_delta_t *src = delta_create();
        delta_add_mkdir(src, "/valid");
        size_t len = 0;
        uint8_t *buf = delta_serialize(src, &len);
        delta_free(src);
        CHECK(buf != NULL);
        /* Path starts at byte 7 (header 4 + kind 1 + path_len 2). */
        buf[7] = 'x';  /* change '/' to 'x' */
        d = delta_deserialize(buf, len, &err);
        CHECK(d == NULL && err < 0);
        free(buf);
    }

    /* Truncated mid-path. */
    {
        fs_delta_t *src = delta_create();
        delta_add_create_file(src, "/longpath/to/file.txt",
                              (const uint8_t *)"abc", 3);
        size_t len = 0;
        uint8_t *buf = delta_serialize(src, &len);
        delta_free(src);
        CHECK(buf != NULL);
        /* Truncate to just past the path_len field (header 4 + kind 1 + path_len 2 = 7,
         * +1 so path_len bytes are present but path data is missing). */
        d = delta_deserialize(buf, 8, &err);
        CHECK(d == NULL && err < 0);
        free(buf);
    }
}

/* -------------------------------------------------------------------------
 * 4. delta_checksum
 * ---------------------------------------------------------------------- */

static void test_delta_checksum(void)
{
    printf("  delta_checksum\n");

    fs_delta_t *d1 = delta_create();
    delta_add_create_file(d1, "/f.txt", (const uint8_t *)"hello", 5);
    size_t len1 = 0;
    uint8_t *buf1 = delta_serialize(d1, &len1);

    fs_delta_t *d2 = delta_create();
    delta_add_create_file(d2, "/f.txt", (const uint8_t *)"hello", 5);
    size_t len2 = 0;
    uint8_t *buf2 = delta_serialize(d2, &len2);

    /* Same content → same checksum. */
    CHECK(len1 == len2);
    CHECK(delta_checksum(buf1, len1) == delta_checksum(buf2, len2));

    /* Different content → different checksum (very high probability). */
    fs_delta_t *d3 = delta_create();
    delta_add_create_file(d3, "/f.txt", (const uint8_t *)"world", 5);
    size_t len3 = 0;
    uint8_t *buf3 = delta_serialize(d3, &len3);
    CHECK(delta_checksum(buf1, len1) != delta_checksum(buf3, len3));

    free(buf1); free(buf2); free(buf3);
    delta_free(d1); delta_free(d2); delta_free(d3);

    /* Empty buffer checksum does not crash. */
    uint64_t h = delta_checksum(NULL, 0);
    (void)h;  /* value doesn't matter; just verify no crash */
}

/* -------------------------------------------------------------------------
 * 5. ensure_parents
 * ---------------------------------------------------------------------- */

static void test_ensure_parents(void)
{
    printf("  ensure_parents\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);

    /* Basic: ensure /a/b for path /a/b/c.txt */
    CHECK(cp_ensure_parents(v, "/a/b/c.txt") == 0);
    vfs_stat_t st;
    CHECK(vfs_getattr(v, "/a", &st) == 0 && st.kind == VFS_DIR);
    CHECK(vfs_getattr(v, "/a/b", &st) == 0 && st.kind == VFS_DIR);
    CHECK(vfs_getattr(v, "/a/b/c.txt", &st) != 0);  /* file not created */

    /* Already exists: calling again returns 0 (EEXIST silenced). */
    CHECK(cp_ensure_parents(v, "/a/b/d.txt") == 0);

    /* Deep path. */
    CHECK(cp_ensure_parents(v, "/x/y/z/w/v/u.txt") == 0);
    CHECK(vfs_getattr(v, "/x/y/z/w/v", &st) == 0 && st.kind == VFS_DIR);

    /* Root: no intermediate dirs needed. */
    CHECK(cp_ensure_parents(v, "/file.txt") == 0);

    /* Bad path (not absolute). */
    CHECK(cp_ensure_parents(v, "relative/path") < 0);

    vfs_destroy(v);
}

/* -------------------------------------------------------------------------
 * 6. apply_basic — one op of each kind
 * ---------------------------------------------------------------------- */

static void test_apply_basic(void)
{
    printf("  apply_basic\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);
    vfs_stat_t st;

    /* CREATE_FILE */
    {
        fs_delta_t *d = delta_create();
        delta_add_create_file(d, "/hello.txt", (const uint8_t *)"hi", 2);
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r != NULL);
        CHECK(r->succeeded == 1 && r->failed == 0);
        CHECK(vfs_getattr(v, "/hello.txt", &st) == 0 && st.kind == VFS_FILE && st.size == 2);
        cp_result_free(r);
        delta_free(d);
    }

    /* UPDATE_FILE */
    {
        fs_delta_t *d = delta_create();
        delta_add_update_file(d, "/hello.txt", (const uint8_t *)"world", 5);
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 1 && r->failed == 0);
        CHECK(vfs_getattr(v, "/hello.txt", &st) == 0 && st.size == 5);
        cp_result_free(r);
        delta_free(d);
    }

    /* MKDIR */
    {
        fs_delta_t *d = delta_create();
        delta_add_mkdir(d, "/mydir");
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 1 && r->failed == 0);
        CHECK(vfs_getattr(v, "/mydir", &st) == 0 && st.kind == VFS_DIR);
        cp_result_free(r);
        delta_free(d);
    }

    /* DELETE_FILE */
    {
        fs_delta_t *d = delta_create();
        delta_add_delete_file(d, "/hello.txt");
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 1 && r->failed == 0);
        CHECK(vfs_getattr(v, "/hello.txt", &st) != 0);
        cp_result_free(r);
        delta_free(d);
    }

    /* RMDIR */
    {
        fs_delta_t *d = delta_create();
        delta_add_rmdir(d, "/mydir");
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 1 && r->failed == 0);
        CHECK(vfs_getattr(v, "/mydir", &st) != 0);
        cp_result_free(r);
        delta_free(d);
    }

    /* SET_TIMES */
    {
        /* Create a file first. */
        vfs_create_file(v, "/ts.txt", (const uint8_t *)"x", 1);
        struct timespec mt = { .tv_sec = 42, .tv_nsec = 0 };
        struct timespec at = { .tv_sec = 99, .tv_nsec = 0 };
        fs_delta_t *d = delta_create();
        delta_add_set_times(d, "/ts.txt", &mt, &at);
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 1 && r->failed == 0);
        CHECK(vfs_getattr(v, "/ts.txt", &st) == 0);
        CHECK(st.mtime.tv_sec == 42 && st.atime.tv_sec == 99);
        cp_result_free(r);
        delta_free(d);
    }

    /* TRUNCATE */
    {
        /* Truncate /ts.txt from 1 byte to 8 bytes (extend with zeros). */
        fs_delta_t *d = delta_create();
        delta_add_truncate(d, "/ts.txt", 8);
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 1 && r->failed == 0);
        CHECK(vfs_getattr(v, "/ts.txt", &st) == 0 && st.size == 8);
        cp_result_free(r);
        delta_free(d);
    }

    vfs_destroy(v);
}

/* -------------------------------------------------------------------------
 * 7. apply_ensure_parents — CREATE_FILE before MKDIR parent
 * ---------------------------------------------------------------------- */

static void test_apply_ensure_parents(void)
{
    printf("  apply_ensure_parents\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);

    /*
     * Delta: CREATE_FILE /a/b/c.txt BEFORE MKDIR /a/b.
     * The control plane must auto-create /a and /a/b first.
     */
    fs_delta_t *d = delta_create();
    delta_add_create_file(d, "/a/b/c.txt", (const uint8_t *)"data", 4);
    delta_add_mkdir(d, "/a/b");  /* out-of-order: parent already auto-created */

    cp_result_t *r = cp_apply_delta(v, d, 0);
    CHECK(r != NULL);
    /* All ops should succeed. */
    CHECK(r->succeeded == 2 && r->failed == 0);

    vfs_stat_t st;
    CHECK(vfs_getattr(v, "/a", &st) == 0 && st.kind == VFS_DIR);
    CHECK(vfs_getattr(v, "/a/b", &st) == 0 && st.kind == VFS_DIR);
    CHECK(vfs_getattr(v, "/a/b/c.txt", &st) == 0 && st.kind == VFS_FILE && st.size == 4);

    cp_result_free(r);
    delta_free(d);
    vfs_destroy(v);
}

/* -------------------------------------------------------------------------
 * 8. apply_rmdir_ordering — shallowest-first list reordered to succeed
 * ---------------------------------------------------------------------- */

static void test_apply_rmdir_ordering(void)
{
    printf("  apply_rmdir_ordering\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);

    /* Build /a/b/c directory tree. */
    vfs_mkdir(v, "/a");
    vfs_mkdir(v, "/a/b");
    vfs_mkdir(v, "/a/b/c");

    /*
     * Delta lists RMDIRs shallowest-first: /a, /a/b, /a/b/c.
     * If applied in that order: /a fails (ENOTEMPTY), /a/b fails, /a/b/c succeeds.
     * The control plane MUST reorder to: /a/b/c, /a/b, /a.
     */
    fs_delta_t *d = delta_create();
    delta_add_rmdir(d, "/a");
    delta_add_rmdir(d, "/a/b");
    delta_add_rmdir(d, "/a/b/c");

    cp_result_t *r = cp_apply_delta(v, d, 0);
    CHECK(r != NULL);
    CHECK(r->succeeded == 3 && r->failed == 0);

    vfs_stat_t st;
    CHECK(vfs_getattr(v, "/a", &st) != 0);  /* /a gone */

    cp_result_free(r);
    delta_free(d);
    vfs_destroy(v);
}

/* -------------------------------------------------------------------------
 * 9. apply_errors — expected failures reported correctly
 * ---------------------------------------------------------------------- */

static void test_apply_errors(void)
{
    printf("  apply_errors\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);
    vfs_mkdir(v, "/populated");
    vfs_create_file(v, "/populated/child.txt", (const uint8_t *)"x", 1);

    /* DELETE_FILE on non-existent path → fail, rest continue. */
    {
        fs_delta_t *d = delta_create();
        delta_add_delete_file(d, "/no_such_file.txt");
        delta_add_mkdir(d, "/newdir");  /* should still succeed */
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->failed == 1 && r->succeeded == 1);
        CHECK(r->results[0].error < 0);  /* ENOENT */
        CHECK(r->results[1].error == 0);
        cp_result_free(r);
        delta_free(d);
    }

    /* RMDIR on non-empty directory → fail. */
    {
        fs_delta_t *d = delta_create();
        delta_add_rmdir(d, "/populated");
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->failed == 1 && r->succeeded == 0);
        CHECK(r->results[0].error == -ENOTEMPTY);
        cp_result_free(r);
        delta_free(d);
    }

    /* UPDATE_FILE on a directory → fail. */
    {
        fs_delta_t *d = delta_create();
        delta_add_update_file(d, "/populated",
                              (const uint8_t *)"oops", 4);
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->failed == 1);
        CHECK(r->results[0].error == -EISDIR);
        cp_result_free(r);
        delta_free(d);
    }

    vfs_destroy(v);
}

/* -------------------------------------------------------------------------
 * 10. apply_set_times — timestamps reach VFS and are readable
 * ---------------------------------------------------------------------- */

static void test_apply_set_times(void)
{
    printf("  apply_set_times\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);
    vfs_create_file(v, "/timed.txt", (const uint8_t *)".", 1);

    struct timespec mt = { .tv_sec = 1700000000, .tv_nsec = 123456789 };
    struct timespec at = { .tv_sec = 1600000000, .tv_nsec = 987654321 };

    fs_delta_t *d = delta_create();
    delta_add_set_times(d, "/timed.txt", &mt, &at);
    cp_result_t *r = cp_apply_delta(v, d, 0);
    CHECK(r->succeeded == 1 && r->failed == 0);

    vfs_stat_t st;
    CHECK(vfs_getattr(v, "/timed.txt", &st) == 0);
    CHECK(st.mtime.tv_sec  == 1700000000);
    CHECK(st.mtime.tv_nsec == 123456789);
    CHECK(st.atime.tv_sec  == 1600000000);
    CHECK(st.atime.tv_nsec == 987654321);

    cp_result_free(r);
    delta_free(d);
    vfs_destroy(v);
}

/* -------------------------------------------------------------------------
 * 11. apply_truncate — shrink and extend
 * ---------------------------------------------------------------------- */

static void test_apply_truncate(void)
{
    printf("  apply_truncate\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);
    const uint8_t orig[] = { 'H','e','l','l','o',' ','W','o','r','l','d' };
    vfs_create_file(v, "/t.txt", orig, sizeof orig);

    vfs_stat_t st;
    CHECK(vfs_getattr(v, "/t.txt", &st) == 0 && st.size == 11);

    /* Shrink to 5 bytes: content should be "Hello". */
    {
        fs_delta_t *d = delta_create();
        delta_add_truncate(d, "/t.txt", 5);
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 1 && r->failed == 0);
        CHECK(vfs_getattr(v, "/t.txt", &st) == 0 && st.size == 5);
        uint8_t buf[10] = {0};
        size_t got = 0;
        vfs_read(v, "/t.txt", 0, 5, buf, &got);
        CHECK(got == 5 && buf[0] == 'H' && buf[4] == 'o');
        cp_result_free(r);
        delta_free(d);
    }

    /* Extend to 10 bytes: bytes 5-9 must be zero. */
    {
        fs_delta_t *d = delta_create();
        delta_add_truncate(d, "/t.txt", 10);
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 1 && r->failed == 0);
        CHECK(vfs_getattr(v, "/t.txt", &st) == 0 && st.size == 10);
        uint8_t buf[10] = {0xff};
        size_t got = 0;
        vfs_read(v, "/t.txt", 0, 10, buf, &got);
        CHECK(got == 10);
        CHECK(buf[5] == 0 && buf[9] == 0);  /* extension is zero-filled */
        cp_result_free(r);
        delta_free(d);
    }

    /* Truncate to 0 bytes. */
    {
        fs_delta_t *d = delta_create();
        delta_add_truncate(d, "/t.txt", 0);
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 1 && r->failed == 0);
        CHECK(vfs_getattr(v, "/t.txt", &st) == 0 && st.size == 0);
        cp_result_free(r);
        delta_free(d);
    }

    vfs_destroy(v);
}

/* -------------------------------------------------------------------------
 * 12. apply_dry_run — tree printed, VFS unchanged afterwards
 * ---------------------------------------------------------------------- */

static void test_apply_dry_run(void)
{
    printf("  apply_dry_run\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);
    vfs_create_file(v, "/baseline.txt", (const uint8_t *)"base", 4);
    vfs_save_snapshot(v);  /* required before dry_run */

    /* Delta creates a new file. */
    fs_delta_t *d = delta_create();
    delta_add_create_file(d, "/new.txt", (const uint8_t *)"new", 3);

    cp_result_t *r = cp_apply_delta(v, d, 1 /* dry_run */);
    CHECK(r != NULL);
    CHECK(r->succeeded == 1 && r->failed == 0);

    /* After dry_run the VFS is restored: /new.txt must not exist. */
    vfs_stat_t st;
    CHECK(vfs_getattr(v, "/new.txt", &st) != 0);

    /* Baseline file must still be present. */
    CHECK(vfs_getattr(v, "/baseline.txt", &st) == 0 && st.size == 4);

    cp_result_free(r);
    delta_free(d);
    vfs_destroy(v);
}

/* -------------------------------------------------------------------------
 * 13. apply_mutate_reset — 10 iterations, no stale state
 * ---------------------------------------------------------------------- */

static void test_apply_mutate_reset(void)
{
    printf("  apply_mutate_reset\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);

    /* Baseline: one file, one directory. */
    vfs_create_file(v, "/seed.txt", (const uint8_t *)"seed_content", 12);
    vfs_mkdir(v, "/docs");
    vfs_save_snapshot(v);

    for (int iter = 0; iter < 10; iter++) {
        /* Build a delta that creates and modifies things. */
        char path[64];
        snprintf(path, sizeof path, "/docs/iter%d.txt", iter);
        const uint8_t content[] = "iteration data";
        fs_delta_t *d = delta_create();
        delta_add_create_file(d, path, content, sizeof content - 1);
        delta_add_update_file(d, "/seed.txt",
                              (const uint8_t *)"mutated", 7);

        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 2 && r->failed == 0);

        /* Verify mutations are visible. */
        vfs_stat_t st;
        CHECK(vfs_getattr(v, path, &st) == 0 && st.size == (sizeof content - 1));
        CHECK(vfs_getattr(v, "/seed.txt", &st) == 0 && st.size == 7);

        cp_result_free(r);
        delta_free(d);

        /* Reset to baseline for next iteration. */
        CHECK(vfs_reset_to_snapshot(v) == 0);

        /* Verify baseline is restored. */
        CHECK(vfs_getattr(v, path, &st) != 0);           /* iter file gone */
        CHECK(vfs_getattr(v, "/seed.txt", &st) == 0);
        CHECK(st.size == 12);                              /* original content */
        CHECK(vfs_getattr(v, "/docs", &st) == 0 && st.kind == VFS_DIR);
    }

    vfs_destroy(v);
}

/* -------------------------------------------------------------------------
 * 14. vfs_checksum — stability and sensitivity
 * ---------------------------------------------------------------------- */

static void test_vfs_checksum(void)
{
    printf("  vfs_checksum\n");

    /*
     * The checksum includes node timestamps.  Two VFS instances built by
     * separate vfs_create_file / vfs_mkdir calls acquire different
     * clock_gettime values, so they cannot be expected to agree unless we
     * pin timestamps to the same value explicitly.
     */
    struct timespec t = { .tv_sec = 1234567890, .tv_nsec = 0 };

    /* Same VFS state (same content + same timestamps) → same checksum. */
    vfs_t *v1 = vfs_create();
    vfs_create_file(v1, "/f.txt", (const uint8_t *)"abc", 3);
    vfs_set_times(v1, "/f.txt", &t, &t);
    vfs_mkdir(v1, "/dir");
    vfs_set_times(v1, "/dir", &t, &t);

    vfs_t *v2 = vfs_create();
    vfs_create_file(v2, "/f.txt", (const uint8_t *)"abc", 3);
    vfs_set_times(v2, "/f.txt", &t, &t);
    vfs_mkdir(v2, "/dir");
    vfs_set_times(v2, "/dir", &t, &t);

    uint64_t c1 = cp_vfs_checksum(v1);
    uint64_t c2 = cp_vfs_checksum(v2);
    CHECK(c1 == c2);

    /* Same VFS, called twice → stable (checksum does not mutate state). */
    CHECK(cp_vfs_checksum(v1) == c1);

    /* Different file content → different checksum. */
    vfs_t *v3 = vfs_create();
    vfs_create_file(v3, "/f.txt", (const uint8_t *)"xyz", 3);
    vfs_set_times(v3, "/f.txt", &t, &t);
    vfs_mkdir(v3, "/dir");
    vfs_set_times(v3, "/dir", &t, &t);
    CHECK(cp_vfs_checksum(v3) != c1);

    /* Different node name → different checksum. */
    vfs_t *v4 = vfs_create();
    vfs_create_file(v4, "/g.txt", (const uint8_t *)"abc", 3);
    vfs_set_times(v4, "/g.txt", &t, &t);
    vfs_mkdir(v4, "/dir");
    vfs_set_times(v4, "/dir", &t, &t);
    CHECK(cp_vfs_checksum(v4) != c1);

    /* Different timestamps → different checksum. */
    vfs_t *v5 = vfs_create();
    vfs_create_file(v5, "/f.txt", (const uint8_t *)"abc", 3);
    struct timespec t2 = { .tv_sec = 9999, .tv_nsec = 0 };
    vfs_set_times(v5, "/f.txt", &t2, &t2);
    vfs_mkdir(v5, "/dir");
    vfs_set_times(v5, "/dir", &t, &t);
    CHECK(cp_vfs_checksum(v5) != c1);

    /* Empty VFS checksum is valid and stable across calls. */
    vfs_t *v6 = vfs_create();
    uint64_t h6a = cp_vfs_checksum(v6);
    uint64_t h6b = cp_vfs_checksum(v6);
    CHECK(h6a == h6b);

    vfs_destroy(v1); vfs_destroy(v2); vfs_destroy(v3);
    vfs_destroy(v4); vfs_destroy(v5); vfs_destroy(v6);
}

/* -------------------------------------------------------------------------
 * Entry point
 * ---------------------------------------------------------------------- */

int main(void)
{
    printf("control_plane test suite\n");
    printf("========================\n");

    test_delta_lifecycle();
    test_delta_serialize();
    test_delta_deser_errors();
    test_delta_checksum();
    test_ensure_parents();
    test_apply_basic();
    test_apply_ensure_parents();
    test_apply_rmdir_ordering();
    test_apply_errors();
    test_apply_set_times();
    test_apply_truncate();
    test_apply_dry_run();
    test_apply_mutate_reset();
    test_vfs_checksum();

    printf("\n========================\n");
    if (g_failures == 0) {
        printf("ALL %d checks passed\n", g_checks);
    } else {
        printf("%d / %d checks FAILED\n", g_failures, g_checks);
    }

    return (g_failures == 0) ? 0 : 1;
}
