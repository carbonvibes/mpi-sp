/*
 * delta.rs — Rust-native filesystem delta type.
 *
 * FsDelta is the canonical LibAFL Input for this fuzzer.  It mirrors
 * the C-side fs_delta_t / fs_op_t from control_plane/delta.h but lives
 * entirely in Rust-managed memory so the mutator stages can operate on
 * it without any FFI overhead.
 *
 * When a delta needs to be applied to the live VFS the ffi::apply_delta
 * function converts it into a C-side fs_delta_t via the delta_add_*
 * convenience calls and then calls cp_apply_delta.
 */

use libafl::{corpus::CorpusId, inputs::Input};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Op kind — mirrors fs_op_kind_t
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FsOpKind {
    CreateFile,
    UpdateFile,
    DeleteFile,
    Mkdir,
    Rmdir,
    SetTimes,
    Truncate,
}

// ─────────────────────────────────────────────────────────────────────────────
// FsOp — a single filesystem operation
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
pub struct FsOp {
    pub kind: FsOpKind,
    /// Absolute path — must start with '/'.
    pub path: String,
    /// Content bytes for CreateFile / UpdateFile ops.  Empty for all others.
    pub content: Vec<u8>,
    /// Semantic size: content length for file ops, new size for Truncate,
    /// 0 for directory / delete / set-times ops.
    pub size: usize,
    // SET_TIMES fields (zero for all other kinds).
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub atime_sec: i64,
    pub atime_nsec: i64,
}

impl FsOp {
    pub fn create_file(path: impl Into<String>, content: Vec<u8>) -> Self {
        let size = content.len();
        Self {
            kind: FsOpKind::CreateFile,
            path: path.into(),
            content,
            size,
            mtime_sec: 0,
            mtime_nsec: 0,
            atime_sec: 0,
            atime_nsec: 0,
        }
    }

    pub fn update_file(path: impl Into<String>, content: Vec<u8>) -> Self {
        let size = content.len();
        Self {
            kind: FsOpKind::UpdateFile,
            path: path.into(),
            content,
            size,
            mtime_sec: 0,
            mtime_nsec: 0,
            atime_sec: 0,
            atime_nsec: 0,
        }
    }

    pub fn delete_file(path: impl Into<String>) -> Self {
        Self {
            kind: FsOpKind::DeleteFile,
            path: path.into(),
            content: vec![],
            size: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            atime_sec: 0,
            atime_nsec: 0,
        }
    }

    pub fn mkdir(path: impl Into<String>) -> Self {
        Self {
            kind: FsOpKind::Mkdir,
            path: path.into(),
            content: vec![],
            size: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            atime_sec: 0,
            atime_nsec: 0,
        }
    }

    pub fn rmdir(path: impl Into<String>) -> Self {
        Self {
            kind: FsOpKind::Rmdir,
            path: path.into(),
            content: vec![],
            size: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            atime_sec: 0,
            atime_nsec: 0,
        }
    }

    pub fn truncate(path: impl Into<String>, new_size: usize) -> Self {
        Self {
            kind: FsOpKind::Truncate,
            path: path.into(),
            content: vec![],
            size: new_size,
            mtime_sec: 0,
            mtime_nsec: 0,
            atime_sec: 0,
            atime_nsec: 0,
        }
    }

    pub fn set_times(
        path: impl Into<String>,
        mtime_sec: i64, mtime_nsec: i64,
        atime_sec: i64, atime_nsec: i64,
    ) -> Self {
        Self {
            kind: FsOpKind::SetTimes,
            path: path.into(),
            content: vec![],
            size: 0,
            mtime_sec,
            mtime_nsec,
            atime_sec,
            atime_nsec,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FsDelta — the LibAFL Input type
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
pub struct FsDelta {
    pub ops: Vec<FsOp>,
}

impl FsDelta {
    pub fn new(ops: Vec<FsOp>) -> Self {
        Self { ops }
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }
}

impl Input for FsDelta {
    fn generate_name(&self, id: Option<CorpusId>) -> String {
        match id {
            Some(id) => format!("delta_{}_ops{}", id.0, self.ops.len()),
            None     => format!("delta_ops{}", self.ops.len()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Seed generators
// ─────────────────────────────────────────────────────────────────────────────

/// Build a minimal valid starting delta.
///
/// Uses UpdateFile so the op succeeds against the standard baseline (which
/// already contains `/input`).  CreateFile would collide → EEXIST, making
/// content mutators (ByteFlip, Replace) operate on a dead op.
pub fn generate_seed() -> FsDelta {
    FsDelta::new(vec![FsOp::update_file("/input", b"seed".to_vec())])
}

/// Generate a corpus of structurally diverse seed deltas.
///
/// Every seed targets paths that exist in the baseline VFS so content and
/// metadata mutators have an immediate chance of producing semantic yield.
///
/// `baseline_files` should be paths enumerated from the baseline VFS at
/// startup (e.g. `["/input", "/etc/config"]`).  Falls back to hard-coded
/// defaults when the slice is empty.
pub fn generate_seed_corpus(baseline_files: &[String]) -> Vec<FsDelta> {
    let primary   = baseline_files.first().map(String::as_str).unwrap_or("/input");
    let secondary = baseline_files.get(1).map(String::as_str).unwrap_or("/etc/config");

    vec![
        // 1. Update the primary file — guaranteed to succeed, content mutators work.
        FsDelta::new(vec![FsOp::update_file(primary, b"seed".to_vec())]),

        // 2. Update a different baseline file (e.g. config the target reads).
        FsDelta::new(vec![FsOp::update_file(secondary, b"[settings]\nverbose=1\n".to_vec())]),

        // 3. Truncate the primary file — exercises size-change paths in the target.
        FsDelta::new(vec![FsOp::truncate(primary, 2)]),

        // 4. Touch timestamps — exercises metadata-only code paths.
        FsDelta::new(vec![FsOp::set_times(primary, 1_700_000_000, 0, 1_700_000_000, 0)]),

        // 5. Multi-op: update content then modify metadata in one delta.
        FsDelta::new(vec![
            FsOp::update_file(primary, b"fuzzed content".to_vec()),
            FsOp::set_times(primary, 1_000_000_000, 0, 1_000_000_000, 0),
        ]),

        // 6. Create a fresh file at a path that doesn't exist — no EEXIST risk.
        FsDelta::new(vec![FsOp::create_file("/fuzz_input", b"new".to_vec())]),

        // 7. Directory + child creation sequence.
        FsDelta::new(vec![
            FsOp::mkdir("/fuzz_dir"),
            FsOp::create_file("/fuzz_dir/file", b"hello".to_vec()),
        ]),
    ]
}

/// A small fixed pool of structurally diverse deltas for SpliceDelta to draw
/// from before a real corpus is accumulated.
///
/// Targets existing baseline paths via UpdateFile / metadata ops to avoid
/// EEXIST collisions on baseline files.
pub fn initial_corpus_pool() -> Vec<FsDelta> {
    vec![
        // Update config — succeeds without any parent creation.
        FsDelta::new(vec![
            FsOp::update_file("/etc/config", b"[settings]\nverbose=1\n".to_vec()),
        ]),
        // New data directory with binary content.
        FsDelta::new(vec![
            FsOp::mkdir("/data"),
            FsOp::create_file("/data/a.bin", vec![0xde, 0xad, 0xbe, 0xef]),
            FsOp::create_file("/data/b.txt", b"hello\n".to_vec()),
        ]),
        // Update primary input file with a longer pattern.
        FsDelta::new(vec![
            FsOp::update_file("/input", b"AAAAAAAAAAAAAAAA".to_vec()),
        ]),
        // Metadata-only: timestamps then truncate on the primary file.
        FsDelta::new(vec![
            FsOp::set_times("/input", 1_700_000_000, 0, 1_700_000_000, 0),
            FsOp::truncate("/input", 2),
        ]),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_seed_corpus_returns_seven_families() {
        let files = vec!["/input".to_string(), "/etc/config".to_string()];
        let corpus = generate_seed_corpus(&files);
        assert_eq!(corpus.len(), 7, "expected exactly 7 seed families");
    }

    #[test]
    fn generate_seed_corpus_all_deltas_non_empty() {
        let files = vec!["/input".to_string(), "/etc/config".to_string()];
        let corpus = generate_seed_corpus(&files);
        for (i, delta) in corpus.iter().enumerate() {
            assert!(!delta.ops.is_empty(), "seed family {i} has no ops");
        }
    }

    #[test]
    fn generate_seed_corpus_all_ops_have_absolute_paths() {
        let files = vec!["/input".to_string(), "/etc/config".to_string()];
        let corpus = generate_seed_corpus(&files);
        for (i, delta) in corpus.iter().enumerate() {
            for op in &delta.ops {
                assert!(
                    op.path.starts_with('/'),
                    "seed family {i}: path '{}' is not absolute",
                    op.path
                );
            }
        }
    }

    #[test]
    fn generate_seed_corpus_uses_fallback_when_empty() {
        // With no baseline files the corpus must still be valid.
        let corpus = generate_seed_corpus(&[]);
        assert_eq!(corpus.len(), 7);
        for delta in &corpus {
            assert!(!delta.ops.is_empty());
        }
    }

    #[test]
    fn seed_one_uses_update_not_create() {
        // Seed family 1 must use UpdateFile so ByteFlip/Replace have a live target.
        let files = vec!["/input".to_string()];
        let corpus = generate_seed_corpus(&files);
        let first_op = &corpus[0].ops[0];
        assert_eq!(
            first_op.kind,
            FsOpKind::UpdateFile,
            "first seed must be UpdateFile — CreateFile would hit EEXIST on /input"
        );
    }

    #[test]
    fn initial_corpus_pool_has_no_eexist_collision() {
        // None of the donors should use CreateFile on baseline paths that
        // already exist (/input, /etc/config) — those would always fail.
        let pool = initial_corpus_pool();
        let baseline = ["/input", "/etc/config", "/etc"];
        for (i, delta) in pool.iter().enumerate() {
            for op in &delta.ops {
                if matches!(op.kind, FsOpKind::CreateFile) {
                    assert!(
                        !baseline.contains(&op.path.as_str()),
                        "pool[{i}] CreateFile on baseline path '{}' → always EEXIST",
                        op.path
                    );
                }
            }
        }
    }
}
