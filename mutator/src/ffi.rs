/*
 * ffi.rs — FFI bindings to the C control plane and VFS.
 *
 * The only entry point the rest of the crate needs is apply_delta(), which
 * translates a Rust FsDelta into a C fs_delta_t using the delta_add_*
 * convenience constructors, then calls cp_apply_delta() and returns the
 * per-op success/failure counts from cp_result_t.
 */

use std::ffi::CString;
use std::os::raw::c_int;

use libc::timespec;

use crate::delta::{FsDelta, FsOpKind};

// ─────────────────────────────────────────────────────────────────────────────
// C types
// ─────────────────────────────────────────────────────────────────────────────

/// Opaque handle to C's vfs_t.
#[repr(C)]
pub struct VfsT {
    _private: [u8; 0],
}

/// Opaque handle to C's fs_delta_t.
#[repr(C)]
pub struct FsDeltaC {
    _private: [u8; 0],
}

/// Transparent layout matching C's cp_op_result_t.
#[repr(C)]
pub struct CpOpResultT {
    pub op_index: c_int,
    pub error:    c_int,
    pub message:  *const i8,
}

/// Transparent layout matching C's cp_result_t.
/// Fields must stay in sync with control_plane/control_plane.h.
#[repr(C)]
pub struct CpResultT {
    pub total_ops: c_int,
    pub succeeded: c_int,
    pub failed:    c_int,
    pub results:   *mut CpOpResultT,
}

// ─────────────────────────────────────────────────────────────────────────────
// Raw C bindings
// ─────────────────────────────────────────────────────────────────────────────

extern "C" {
    // ── VFS lifecycle ─────────────────────────────────────────────────────
    pub fn vfs_create() -> *mut VfsT;
    pub fn vfs_destroy(vfs: *mut VfsT);
    pub fn vfs_save_snapshot(vfs: *mut VfsT) -> c_int;
    pub fn vfs_reset_to_snapshot(vfs: *mut VfsT) -> c_int;

    pub fn vfs_create_file(
        vfs: *mut VfsT,
        path: *const i8,
        content: *const u8,
        len: usize,
    ) -> c_int;
    pub fn vfs_mkdir(vfs: *mut VfsT, path: *const i8) -> c_int;

    // ── Delta lifecycle ───────────────────────────────────────────────────
    pub fn delta_create() -> *mut FsDeltaC;
    pub fn delta_free(d: *mut FsDeltaC);

    pub fn delta_add_create_file(
        d: *mut FsDeltaC,
        path: *const i8,
        content: *const u8,
        len: usize,
    ) -> c_int;
    pub fn delta_add_update_file(
        d: *mut FsDeltaC,
        path: *const i8,
        content: *const u8,
        len: usize,
    ) -> c_int;
    pub fn delta_add_delete_file(d: *mut FsDeltaC, path: *const i8) -> c_int;
    pub fn delta_add_mkdir(d: *mut FsDeltaC, path: *const i8) -> c_int;
    pub fn delta_add_rmdir(d: *mut FsDeltaC, path: *const i8) -> c_int;
    pub fn delta_add_set_times(
        d: *mut FsDeltaC,
        path: *const i8,
        mtime: *const timespec,
        atime: *const timespec,
    ) -> c_int;
    pub fn delta_add_truncate(d: *mut FsDeltaC, path: *const i8, new_size: usize) -> c_int;

    // ── Control plane ─────────────────────────────────────────────────────
    pub fn cp_apply_delta(
        vfs: *mut VfsT,
        d: *const FsDeltaC,
        dry_run: c_int,
    ) -> *mut CpResultT;
    pub fn cp_result_free(r: *mut CpResultT);

    /// FNV-1a hash of the current VFS tree.  Used for semantic yield tracking.
    pub fn cp_vfs_checksum(vfs: *mut VfsT) -> u64;
}

// ─────────────────────────────────────────────────────────────────────────────
// Public result type
// ─────────────────────────────────────────────────────────────────────────────

/// Per-call outcome of apply_delta().
///
/// Returned as `Ok(DeltaResult)` even when individual ops fail — that is
/// expected fuzzer behaviour.  `Err(errno)` is reserved for catastrophic
/// failures (OOM constructing the C delta, null return from cp_apply_delta).
#[derive(Debug, Clone, Copy)]
pub struct DeltaResult {
    pub succeeded: usize,
    pub failed:    usize,
}

impl DeltaResult {
    pub fn all_ok(&self) -> bool {
        self.failed == 0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helper
// ─────────────────────────────────────────────────────────────────────────────

/// Assert structural invariants on a delta in debug builds.
/// Called at the top of apply_delta so bad inputs are caught early.
#[inline]
fn validate_delta(delta: &FsDelta) {
    debug_assert!(!delta.ops.is_empty(), "delta must have at least one op");
    for op in &delta.ops {
        debug_assert!(
            op.path.starts_with('/'),
            "op path must be absolute: {}",
            op.path
        );
        if matches!(op.kind, FsOpKind::CreateFile | FsOpKind::UpdateFile) {
            debug_assert_eq!(
                op.size,
                op.content.len(),
                "size/content length mismatch for {}",
                op.path
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Safe wrapper
// ─────────────────────────────────────────────────────────────────────────────

/// Build a C-side fs_delta_t from a Rust FsDelta and apply it to the VFS.
///
/// Returns `Ok(DeltaResult)` on success.  `result.failed > 0` means that some
/// ops were individually rejected by the VFS (e.g. ENOENT on a delete of a
/// non-existent file); this is normal fuzzer behaviour and not an `Err`.
///
/// Returns `Err(errno)` only for catastrophic failures:
///   - OOM constructing the C delta (`-ENOMEM`)
///   - null pointer returned by cp_apply_delta (`-ENOMEM`)
///   - interior NUL byte in an op path (`-EINVAL`)
///   - delta_add_* returns a non-zero errno (`-errno`)
pub fn apply_delta(vfs: *mut VfsT, delta: &FsDelta) -> Result<DeltaResult, i32> {
    validate_delta(delta);

    let c_delta = unsafe { delta_create() };
    if c_delta.is_null() {
        return Err(-libc::ENOMEM);
    }

    for op in &delta.ops {
        let path = match CString::new(op.path.as_str()) {
            Ok(p) => p,
            Err(_) => {
                unsafe { delta_free(c_delta) };
                return Err(-libc::EINVAL);
            }
        };

        let ret = unsafe {
            match op.kind {
                FsOpKind::CreateFile => delta_add_create_file(
                    c_delta,
                    path.as_ptr(),
                    op.content.as_ptr(),
                    op.content.len(),
                ),
                FsOpKind::UpdateFile => delta_add_update_file(
                    c_delta,
                    path.as_ptr(),
                    op.content.as_ptr(),
                    op.content.len(),
                ),
                FsOpKind::DeleteFile => delta_add_delete_file(c_delta, path.as_ptr()),
                FsOpKind::Mkdir      => delta_add_mkdir(c_delta, path.as_ptr()),
                FsOpKind::Rmdir      => delta_add_rmdir(c_delta, path.as_ptr()),
                FsOpKind::Truncate   => delta_add_truncate(c_delta, path.as_ptr(), op.size),
                FsOpKind::SetTimes   => {
                    let mtime = timespec {
                        tv_sec:  op.mtime_sec  as libc::time_t,
                        tv_nsec: op.mtime_nsec as libc::c_long,
                    };
                    let atime = timespec {
                        tv_sec:  op.atime_sec  as libc::time_t,
                        tv_nsec: op.atime_nsec as libc::c_long,
                    };
                    delta_add_set_times(c_delta, path.as_ptr(), &mtime, &atime)
                }
            }
        };

        if ret != 0 {
            unsafe { delta_free(c_delta) };
            return Err(ret);
        }
    }

    let result_ptr = unsafe { cp_apply_delta(vfs, c_delta, 0) };
    unsafe { delta_free(c_delta) };

    if result_ptr.is_null() {
        return Err(-libc::ENOMEM);
    }

    let dr = unsafe {
        DeltaResult {
            succeeded: (*result_ptr).succeeded as usize,
            failed:    (*result_ptr).failed    as usize,
        }
    };
    unsafe { cp_result_free(result_ptr) };

    Ok(dr)
}

// ─────────────────────────────────────────────────────────────────────────────
// E2E integration tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::{FsDelta, FsOp};

    /// Create a VFS with the standard baseline and a saved snapshot.
    unsafe fn make_baseline_vfs() -> *mut VfsT {
        let vfs = vfs_create();
        assert!(!vfs.is_null(), "vfs_create() returned null");
        vfs_create_file(vfs, c"/input".as_ptr(), b"seed".as_ptr(), 4);
        vfs_mkdir(vfs, c"/etc".as_ptr());
        vfs_create_file(
            vfs,
            c"/etc/config".as_ptr(),
            b"[settings]\nverbose=0\n".as_ptr(),
            20,
        );
        assert_eq!(vfs_save_snapshot(vfs), 0, "vfs_save_snapshot() failed");
        vfs
    }

    #[test]
    fn e2e_create_file_succeeds() {
        let vfs = unsafe { make_baseline_vfs() };
        let delta = FsDelta::new(vec![FsOp::create_file("/new.txt", b"hello".to_vec())]);
        let dr = apply_delta(vfs, &delta).expect("apply_delta returned Err");
        assert!(dr.succeeded > 0, "no ops succeeded");
        assert_eq!(dr.failed, 0, "unexpected op failures");
        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn e2e_mkdir_and_create_file_succeeds() {
        let vfs = unsafe { make_baseline_vfs() };
        let delta = FsDelta::new(vec![
            FsOp::mkdir("/tmp"),
            FsOp::create_file("/tmp/test.txt", b"data".to_vec()),
        ]);
        let dr = apply_delta(vfs, &delta).expect("apply_delta returned Err");
        assert_eq!(dr.succeeded, 2, "expected both ops to succeed");
        assert_eq!(dr.failed, 0);
        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn e2e_update_existing_file_succeeds() {
        let vfs = unsafe { make_baseline_vfs() };
        let delta = FsDelta::new(vec![FsOp::update_file("/input", b"mutated_content".to_vec())]);
        let dr = apply_delta(vfs, &delta).expect("apply_delta returned Err");
        assert_eq!(dr.succeeded, 1);
        assert_eq!(dr.failed, 0);
        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn e2e_set_times_on_existing_file_succeeds() {
        let vfs = unsafe { make_baseline_vfs() };
        // Set timestamps on /input which exists in the baseline.
        let delta = FsDelta::new(vec![
            FsOp::set_times("/input", 1_000_000_000, 0, 1_000_000_000, 0),
        ]);
        let dr = apply_delta(vfs, &delta).expect("apply_delta returned Err");
        assert!(dr.succeeded > 0, "set_times op should have succeeded");
        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn e2e_failed_op_is_counted_not_panicked() {
        let vfs = unsafe { make_baseline_vfs() };
        // Delete a file that doesn't exist — should fail at VFS level, not crash.
        let delta = FsDelta::new(vec![FsOp::delete_file("/does_not_exist.txt")]);
        let dr = apply_delta(vfs, &delta).expect("apply_delta returned Err");
        // The call succeeds (Ok) but the op itself fails.
        assert_eq!(dr.succeeded + dr.failed, 1, "should have exactly 1 op result");
        unsafe { vfs_destroy(vfs) };
    }
}
