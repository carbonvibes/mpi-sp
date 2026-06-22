/* cp_test.c — test suite for the control plane */

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "../vfs/vfs.h"
#include "control_plane.h"
#include "delta.h"



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

    CHECK(d->ops[0].kind == FS_OP_CREATE_FILE);
    CHECK(d->ops[1].kind == FS_OP_UPDATE_FILE);
    CHECK(d->ops[2].kind == FS_OP_DELETE_FILE);
    CHECK(d->ops[3].kind == FS_OP_MKDIR);
    CHECK(d->ops[4].kind == FS_OP_RMDIR);
    CHECK(d->ops[5].kind == FS_OP_SET_TIMES);
    CHECK(d->ops[6].kind == FS_OP_TRUNCATE);

    CHECK(strcmp(d->ops[0].path, "/a.txt") == 0);
    CHECK(d->ops[0].content_len == 3);
    CHECK(d->ops[0].content != NULL);
    CHECK(d->ops[0].content[0] == 0x41);

    CHECK(d->ops[6].content_len == 100);
    CHECK(d->ops[6].content == NULL);

    CHECK(d->ops[5].mtime.tv_sec == 1000);
    CHECK(d->ops[5].atime.tv_nsec == 500);

    delta_free(d);
    delta_free(NULL);  /* must not crash */
}



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
    CHECK(len > 4);  /* header is n_ops u32 */

    int err = 0;
    fs_delta_t *copy = delta_deserialize(buf, len, &err);
    CHECK(err == 0);
    CHECK(copy != NULL);
    CHECK(copy->n_ops == 7);

    CHECK(copy->ops[0].kind == FS_OP_CREATE_FILE);
    CHECK(strcmp(copy->ops[0].path, "/data/file.bin") == 0);
    CHECK(copy->ops[0].content_len == 4);
    CHECK(copy->ops[0].content != NULL);
    CHECK(memcmp(copy->ops[0].content, content, 4) == 0);

    CHECK(copy->ops[1].kind == FS_OP_UPDATE_FILE);
    CHECK(copy->ops[1].content_len == 4);
    CHECK(memcmp(copy->ops[1].content, content, 4) == 0);

    CHECK(copy->ops[2].kind == FS_OP_DELETE_FILE);
    CHECK(strcmp(copy->ops[2].path, "/data/file.bin") == 0);
    CHECK(copy->ops[2].content == NULL);
    CHECK(copy->ops[2].content_len == 0);

    CHECK(copy->ops[3].kind == FS_OP_MKDIR);
    CHECK(strcmp(copy->ops[3].path, "/data/subdir") == 0);

    CHECK(copy->ops[4].kind == FS_OP_RMDIR);

    CHECK(copy->ops[5].kind == FS_OP_SET_TIMES);
    CHECK(copy->ops[5].mtime.tv_sec  == mt.tv_sec);
    CHECK(copy->ops[5].mtime.tv_nsec == mt.tv_nsec);
    CHECK(copy->ops[5].atime.tv_sec  == at.tv_sec);
    CHECK(copy->ops[5].atime.tv_nsec == at.tv_nsec);

    CHECK(copy->ops[6].kind == FS_OP_TRUNCATE);
    CHECK(copy->ops[6].content_len == 512);
    CHECK(copy->ops[6].content == NULL);

    free(buf);
    delta_free(orig);
    delta_free(copy);

    /* empty delta serializes to NULL */
    fs_delta_t *empty = delta_create();
    size_t elen = 99;
    uint8_t *ebuf = delta_serialize(empty, &elen);
    CHECK(ebuf == NULL);
    CHECK(elen == 0);
    delta_free(empty);
}



static void test_delta_deser_errors(void)
{
    printf("  delta_deser_errors\n");

    int err;
    fs_delta_t *d;

    /* buffer too short for 4-byte header */
    uint8_t tiny[] = { 0x00, 0x00, 0x00 };
    d = delta_deserialize(tiny, 3, &err);
    CHECK(d == NULL && err < 0);

    /* n_ops far exceeds buffer */
    {
        uint8_t huge_ops[4] = { 0x00, 0xFF, 0xFF, 0xFF };
        d = delta_deserialize(huge_ops, 4, &err);
        CHECK(d == NULL && err < 0);
    }

    /* zero n_ops */
    uint8_t zero_ops[4] = { 0, 0, 0, 0 };
    d = delta_deserialize(zero_ops, 4, &err);
    CHECK(d == NULL && err < 0);

    /* invalid op kind (0 reserved) */
    {
        fs_delta_t *src = delta_create();
        delta_add_mkdir(src, "/x");
        size_t len = 0;
        uint8_t *buf = delta_serialize(src, &len);
        delta_free(src);
        CHECK(buf != NULL);
        buf[4] = 0;  /* kind byte, header is 4 bytes */
        d = delta_deserialize(buf, len, &err);
        CHECK(d == NULL && err < 0);
        free(buf);
    }

    /* path not starting with '/' */
    {
        fs_delta_t *src = delta_create();
        delta_add_mkdir(src, "/valid");
        size_t len = 0;
        uint8_t *buf = delta_serialize(src, &len);
        delta_free(src);
        CHECK(buf != NULL);
        buf[7] = 'x';  /* path starts at byte 7 (header 4 + kind 1 + path_len 2) */
        d = delta_deserialize(buf, len, &err);
        CHECK(d == NULL && err < 0);
        free(buf);
    }

    /* truncated mid-path */
    {
        fs_delta_t *src = delta_create();
        delta_add_create_file(src, "/longpath/to/file.txt",
                              (const uint8_t *)"abc", 3);
        size_t len = 0;
        uint8_t *buf = delta_serialize(src, &len);
        delta_free(src);
        CHECK(buf != NULL);
        /* 8 = past path_len field, before path data */
        d = delta_deserialize(buf, 8, &err);
        CHECK(d == NULL && err < 0);
        free(buf);
    }
}



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

    /* same content, same checksum */
    CHECK(len1 == len2);
    CHECK(delta_checksum(buf1, len1) == delta_checksum(buf2, len2));

    /* different content, different checksum */
    fs_delta_t *d3 = delta_create();
    delta_add_create_file(d3, "/f.txt", (const uint8_t *)"world", 5);
    size_t len3 = 0;
    uint8_t *buf3 = delta_serialize(d3, &len3);
    CHECK(delta_checksum(buf1, len1) != delta_checksum(buf3, len3));

    free(buf1); free(buf2); free(buf3);
    delta_free(d1); delta_free(d2); delta_free(d3);

    /* empty buffer must not crash */
    uint64_t h = delta_checksum(NULL, 0);
    (void)h;
}



static void test_ensure_parents(void)
{
    printf("  ensure_parents\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);

    CHECK(cp_ensure_parents(v, "/a/b/c.txt") == 0);
    vfs_stat_t st;
    CHECK(vfs_getattr(v, "/a", &st) == 0 && st.kind == VFS_DIR);
    CHECK(vfs_getattr(v, "/a/b", &st) == 0 && st.kind == VFS_DIR);
    CHECK(vfs_getattr(v, "/a/b/c.txt", &st) != 0);  /* file not created */

    /* already exists: EEXIST silenced */
    CHECK(cp_ensure_parents(v, "/a/b/d.txt") == 0);

    CHECK(cp_ensure_parents(v, "/x/y/z/w/v/u.txt") == 0);
    CHECK(vfs_getattr(v, "/x/y/z/w/v", &st) == 0 && st.kind == VFS_DIR);

    /* root: no intermediate dirs */
    CHECK(cp_ensure_parents(v, "/file.txt") == 0);

    /* not absolute */
    CHECK(cp_ensure_parents(v, "relative/path") < 0);

    vfs_destroy(v);
}



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



static void test_apply_ensure_parents(void)
{
    printf("  apply_ensure_parents\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);

    /* CREATE_FILE before its MKDIR: control plane must auto-create /a, /a/b */
    fs_delta_t *d = delta_create();
    delta_add_create_file(d, "/a/b/c.txt", (const uint8_t *)"data", 4);
    delta_add_mkdir(d, "/a/b");  /* out-of-order: parent already auto-created */

    cp_result_t *r = cp_apply_delta(v, d, 0);
    CHECK(r != NULL);
    CHECK(r->succeeded == 2 && r->failed == 0);

    vfs_stat_t st;
    CHECK(vfs_getattr(v, "/a", &st) == 0 && st.kind == VFS_DIR);
    CHECK(vfs_getattr(v, "/a/b", &st) == 0 && st.kind == VFS_DIR);
    CHECK(vfs_getattr(v, "/a/b/c.txt", &st) == 0 && st.kind == VFS_FILE && st.size == 4);

    cp_result_free(r);
    delta_free(d);
    vfs_destroy(v);
}



static void test_apply_rmdir_ordering(void)
{
    printf("  apply_rmdir_ordering\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);

    vfs_mkdir(v, "/a");
    vfs_mkdir(v, "/a/b");
    vfs_mkdir(v, "/a/b/c");

    /* RMDIRs listed shallowest-first; control plane must reorder deepest-first */
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



static void test_apply_errors(void)
{
    printf("  apply_errors\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);
    vfs_mkdir(v, "/populated");
    vfs_create_file(v, "/populated/child.txt", (const uint8_t *)"x", 1);

    /* DELETE_FILE on missing path fails, rest continue */
    {
        fs_delta_t *d = delta_create();
        delta_add_delete_file(d, "/no_such_file.txt");
        delta_add_mkdir(d, "/newdir");
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->failed == 1 && r->succeeded == 1);
        CHECK(r->results[0].error < 0);
        CHECK(r->results[1].error == 0);
        cp_result_free(r);
        delta_free(d);
    }

    /* RMDIR on non-empty dir fails */
    {
        fs_delta_t *d = delta_create();
        delta_add_rmdir(d, "/populated");
        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->failed == 1 && r->succeeded == 0);
        CHECK(r->results[0].error == -ENOTEMPTY);
        cp_result_free(r);
        delta_free(d);
    }

    /* UPDATE_FILE on a dir fails */
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



static void test_apply_truncate(void)
{
    printf("  apply_truncate\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);
    const uint8_t orig[] = { 'H','e','l','l','o',' ','W','o','r','l','d' };
    vfs_create_file(v, "/t.txt", orig, sizeof orig);

    vfs_stat_t st;
    CHECK(vfs_getattr(v, "/t.txt", &st) == 0 && st.size == 11);

    /* shrink to 5: "Hello" */
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

    /* extend to 10: bytes 5-9 zero-filled */
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
        CHECK(buf[5] == 0 && buf[9] == 0);
        cp_result_free(r);
        delta_free(d);
    }

    /* truncate to 0 */
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



static void test_apply_dry_run(void)
{
    printf("  apply_dry_run\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);
    vfs_create_file(v, "/baseline.txt", (const uint8_t *)"base", 4);
    vfs_save_snapshot(v);  /* required before dry_run */

    fs_delta_t *d = delta_create();
    delta_add_create_file(d, "/new.txt", (const uint8_t *)"new", 3);

    cp_result_t *r = cp_apply_delta(v, d, 1 /* dry_run */);
    CHECK(r != NULL);
    CHECK(r->succeeded == 1 && r->failed == 0);

    /* dry_run restores: /new.txt gone, baseline intact */
    vfs_stat_t st;
    CHECK(vfs_getattr(v, "/new.txt", &st) != 0);
    CHECK(vfs_getattr(v, "/baseline.txt", &st) == 0 && st.size == 4);

    cp_result_free(r);
    delta_free(d);
    vfs_destroy(v);
}



static void test_apply_mutate_reset(void)
{
    printf("  apply_mutate_reset\n");

    vfs_t *v = vfs_create();
    CHECK(v != NULL);

    vfs_create_file(v, "/seed.txt", (const uint8_t *)"seed_content", 12);
    vfs_mkdir(v, "/docs");
    vfs_save_snapshot(v);

    for (int iter = 0; iter < 10; iter++) {
        char path[64];
        snprintf(path, sizeof path, "/docs/iter%d.txt", iter);
        const uint8_t content[] = "iteration data";
        fs_delta_t *d = delta_create();
        delta_add_create_file(d, path, content, sizeof content - 1);
        delta_add_update_file(d, "/seed.txt",
                              (const uint8_t *)"mutated", 7);

        cp_result_t *r = cp_apply_delta(v, d, 0);
        CHECK(r->succeeded == 2 && r->failed == 0);

        vfs_stat_t st;
        CHECK(vfs_getattr(v, path, &st) == 0 && st.size == (sizeof content - 1));
        CHECK(vfs_getattr(v, "/seed.txt", &st) == 0 && st.size == 7);

        cp_result_free(r);
        delta_free(d);

        CHECK(vfs_reset_to_snapshot(v) == 0);

        CHECK(vfs_getattr(v, path, &st) != 0);           /* iter file gone */
        CHECK(vfs_getattr(v, "/seed.txt", &st) == 0);
        CHECK(st.size == 12);                              /* original content */
        CHECK(vfs_getattr(v, "/docs", &st) == 0 && st.kind == VFS_DIR);
    }

    vfs_destroy(v);
}



static void test_vfs_checksum(void)
{
    printf("  vfs_checksum\n");

    /* checksum includes timestamps, so pin them or two VFSes won't match */
    struct timespec t = { .tv_sec = 1234567890, .tv_nsec = 0 };

    /* same state, same checksum */
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

    /* stable across calls */
    CHECK(cp_vfs_checksum(v1) == c1);

    /* different content */
    vfs_t *v3 = vfs_create();
    vfs_create_file(v3, "/f.txt", (const uint8_t *)"xyz", 3);
    vfs_set_times(v3, "/f.txt", &t, &t);
    vfs_mkdir(v3, "/dir");
    vfs_set_times(v3, "/dir", &t, &t);
    CHECK(cp_vfs_checksum(v3) != c1);

    /* different name */
    vfs_t *v4 = vfs_create();
    vfs_create_file(v4, "/g.txt", (const uint8_t *)"abc", 3);
    vfs_set_times(v4, "/g.txt", &t, &t);
    vfs_mkdir(v4, "/dir");
    vfs_set_times(v4, "/dir", &t, &t);
    CHECK(cp_vfs_checksum(v4) != c1);

    /* different timestamps */
    vfs_t *v5 = vfs_create();
    vfs_create_file(v5, "/f.txt", (const uint8_t *)"abc", 3);
    struct timespec t2 = { .tv_sec = 9999, .tv_nsec = 0 };
    vfs_set_times(v5, "/f.txt", &t2, &t2);
    vfs_mkdir(v5, "/dir");
    vfs_set_times(v5, "/dir", &t, &t);
    CHECK(cp_vfs_checksum(v5) != c1);

    /* empty VFS is stable */
    vfs_t *v6 = vfs_create();
    uint64_t h6a = cp_vfs_checksum(v6);
    uint64_t h6b = cp_vfs_checksum(v6);
    CHECK(h6a == h6b);

    vfs_destroy(v1); vfs_destroy(v2); vfs_destroy(v3);
    vfs_destroy(v4); vfs_destroy(v5); vfs_destroy(v6);
}



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
