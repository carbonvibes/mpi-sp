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
// Seed generator — produces the initial corpus entry
// ─────────────────────────────────────────────────────────────────────────────

/// Build a minimal valid delta: one file with known seed content.
/// This is what the fuzzer starts from before any mutations are applied.
pub fn generate_seed() -> FsDelta {
    FsDelta::new(vec![FsOp::create_file("/input", b"seed".to_vec())])
}

/// A small fixed pool of structurally diverse deltas for SpliceDelta to draw
/// from before a real corpus is accumulated.
pub fn initial_corpus_pool() -> Vec<FsDelta> {
    vec![
        FsDelta::new(vec![
            FsOp::mkdir("/etc"),
            FsOp::create_file("/etc/config", b"[settings]\nverbose=1\n".to_vec()),
        ]),
        FsDelta::new(vec![
            FsOp::mkdir("/data"),
            FsOp::create_file("/data/a.bin", vec![0xde, 0xad, 0xbe, 0xef]),
            FsOp::create_file("/data/b.txt", b"hello\n".to_vec()),
        ]),
        FsDelta::new(vec![
            FsOp::create_file("/input", b"AAAAAAAAAAAAAAAA".to_vec()),
        ]),
    ]
}
