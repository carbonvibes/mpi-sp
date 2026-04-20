/*
 * control_plane.h — In-process control plane API.
 *
 * The control plane bridges the fuzzer to the live VFS.  Its job is to:
 *
 *  1. Apply a delta (fs_delta_t) to a vfs_t instance with correctness fixups:
 *       - ensure_parents(): create any missing intermediate directories before
 *         a CREATE_FILE or MKDIR op so out-of-order deltas succeed.
 *       - Depth-first RMDIR ordering: RMDIR ops are applied deepest-first so
 *         a parent RMDIR does not fail because a child directory still exists.
 *
 *  2. Support a dry-run mode: apply the delta, print the resulting VFS tree,
 *     then restore to the saved snapshot.  Requires a snapshot to exist.
 *
 *  3. Compute a stable checksum of the current VFS tree for baseline tagging.
 *
 * Transport: in-process shared-library call.  The fuzzer and VFS run in the
 * same process; no IPC is required for Week 4.  (A Unix-socket transport is
 * the obvious extension if process separation is ever needed.)
 */

#ifndef CONTROL_PLANE_H
#define CONTROL_PLANE_H

#include <stdint.h>

#include "../vfs/vfs.h"
#include "delta.h"

/* -------------------------------------------------------------------------
 * Result types
 * ---------------------------------------------------------------------- */

/* Per-op outcome. */
typedef struct {
    int         op_index;   /* index into the delta's ops array */
    int         error;      /* 0 = success, negative errno = failure */
    const char *message;    /* static descriptive string (never NULL) */
} cp_op_result_t;

/* Batch outcome returned by cp_apply_delta(). */
typedef struct {
    int             total_ops;
    int             succeeded;
    int             failed;
    cp_op_result_t *results;   /* array[total_ops]; caller frees via cp_result_free() */
} cp_result_t;

/* Free a cp_result_t. */
void cp_result_free(cp_result_t *r);

/* -------------------------------------------------------------------------
 * Core apply
 * ---------------------------------------------------------------------- */

/*
 * Apply delta d to vfs.
 *
 * Non-RMDIR ops are processed in their original order.  Before each
 * CREATE_FILE or MKDIR op, cp_ensure_parents() is called to create any
 * missing intermediate directories.
 *
 * All RMDIR ops are deferred to a second phase and applied in descending
 * path depth (deepest first) so that child directories are removed before
 * their parents.
 *
 * If dry_run != 0:
 *   The delta is applied as above, the resulting VFS tree is printed to
 *   stdout via cp_dump_vfs(), then the VFS is restored to its saved
 *   snapshot.  A snapshot MUST have been saved (via vfs_save_snapshot)
 *   before calling with dry_run=1; if no snapshot exists the delta is
 *   applied permanently and a warning is printed.
 *
 * Returns a heap-allocated cp_result_t; caller must call cp_result_free().
 * Returns NULL only on catastrophic ENOMEM allocating the result struct.
 */
cp_result_t *cp_apply_delta(vfs_t *vfs, const fs_delta_t *d, int dry_run);

/* -------------------------------------------------------------------------
 * Baseline checksum
 * ---------------------------------------------------------------------- */

/*
 * Compute a 64-bit FNV-1a hash of the current VFS tree.
 *
 * The walk visits nodes in readdir order (insertion order, which is
 * deterministic for a given population sequence) and hashes:
 *   - the absolute path of every node
 *   - file content bytes
 *   - symlink target strings
 *   - mtime and atime seconds + nanoseconds
 *
 * Two VFS instances populated identically (same ops in the same order)
 * will produce the same checksum.  This is sufficient for baseline tagging
 * so that a saved testcase can be reproduced by anyone with the same baseline.
 *
 * NOTE: the hash is NOT sorted — node order matters.  If you need an
 * order-independent checksum in a future revision, sort child names before
 * hashing.
 */
uint64_t cp_vfs_checksum(vfs_t *vfs);

/* -------------------------------------------------------------------------
 * Path enumeration
 * ---------------------------------------------------------------------- */

/*
 * Enumerate all paths in the VFS tree, filtered by node kind.
 *
 * filter:  0 = all paths (files + directories + symlinks)
 *          1 = regular files only
 *          2 = directories only
 *
 * On success, *paths_out is a heap-allocated array of *n_out heap-allocated
 * NUL-terminated absolute-path strings.  Free with cp_enumerate_paths_free().
 * Returns 0 on success.
 */
int cp_enumerate_paths(vfs_t *vfs, int filter,
                       char ***paths_out, size_t *n_out);

/* Free the array returned by cp_enumerate_paths(). */
void cp_enumerate_paths_free(char **paths, size_t n);

/* -------------------------------------------------------------------------
 * Dump
 * ---------------------------------------------------------------------- */

/*
 * Print the current VFS tree to stdout.
 * Produces an indented listing of all paths with sizes and symlink targets.
 * Used by dry-run mode and for manual inspection.
 */
void cp_dump_vfs(vfs_t *vfs);

/* -------------------------------------------------------------------------
 * Helpers (exposed for testing)
 * ---------------------------------------------------------------------- */

/*
 * Ensure all intermediate directories along path exist.
 *
 * For path = "/a/b/c" (whether a file or directory) this function creates
 * "/a" and "/a/b" if they do not already exist.  The path itself is not
 * created; only its parent chain is.
 *
 * EEXIST from vfs_mkdir is silently ignored (directory already present).
 * Any other error is returned immediately.
 *
 * Returns 0 on success, negative errno on VFS allocation failure.
 */
int cp_ensure_parents(vfs_t *vfs, const char *path);

#endif /* CONTROL_PLANE_H */
