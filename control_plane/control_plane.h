#ifndef CONTROL_PLANE_H
#define CONTROL_PLANE_H

#include <stdint.h>

#include "../vfs/vfs.h"
#include "delta.h"

typedef struct {
    int         op_index;
    int         error;
    const char *message;
} cp_op_result_t;

typedef struct {
    int             total_ops;
    int             succeeded;
    int             failed;
    cp_op_result_t *results;   /* array[total_ops]; caller frees via cp_result_free() */
} cp_result_t;

void cp_result_free(cp_result_t *r);

cp_result_t *cp_apply_delta(vfs_t *vfs, const fs_delta_t *d, int dry_run);

uint64_t cp_vfs_checksum(vfs_t *vfs);

/* filter: 0=all, 1=files only, 2=dirs only */
int cp_enumerate_paths(vfs_t *vfs, int filter,
                       char ***paths_out, size_t *n_out);
void cp_enumerate_paths_free(char **paths, size_t n);

void cp_dump_vfs(vfs_t *vfs);

/* creates any missing intermediate directories; EEXIST is ok */
int cp_ensure_parents(vfs_t *vfs, const char *path);

#endif /* CONTROL_PLANE_H */
