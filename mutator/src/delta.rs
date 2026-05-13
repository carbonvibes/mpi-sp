use libafl::{corpus::CorpusId, inputs::Input};
use libafl_bolts::HasLen;
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
pub struct FsOp {
    pub kind: FsOpKind,
    /// Absolute path — must start with '/'.
    pub path: String,
    /// Content bytes for CreateFile / UpdateFile ops.
    pub content: Vec<u8>,
    /// Semantic size: content length for file ops, new size for Truncate, 0 otherwise.
    pub size: usize,
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
        mtime_sec: i64,
        mtime_nsec: i64,
        atime_sec: i64,
        atime_nsec: i64,
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

    /// Strip redundant content ops — for any path with multiple
    /// CreateFile/UpdateFile ops, only the last one matters.
    pub fn dedup_content_ops(&self) -> Self {
        let mut last_content_idx: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for (i, op) in self.ops.iter().enumerate() {
            if matches!(op.kind, FsOpKind::CreateFile | FsOpKind::UpdateFile) {
                last_content_idx.insert(op.path.as_str(), i);
            }
        }

        let ops = self
            .ops
            .iter()
            .enumerate()
            .filter(|(i, op)| match last_content_idx.get(op.path.as_str()) {
                Some(last) => *i == *last,
                None => true,
            })
            .map(|(_, op)| op.clone())
            .collect();

        Self { ops }
    }
}

impl HasLen for FsDelta {
    fn len(&self) -> usize {
        self.ops.len()
    }
}

impl Input for FsDelta {
    fn generate_name(&self, id: Option<CorpusId>) -> String {
        match id {
            Some(id) => format!("delta_{}_ops{}.json", id.0, self.ops.len()),
            None => format!("delta_ops{}.json", self.ops.len()),
        }
    }

    fn to_file<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), libafl::Error> {
        let clean = self.dedup_content_ops();
        let json = serde_json::to_string_pretty(&clean)
            .map_err(|e| libafl::Error::serialize(e.to_string()))?;
        std::fs::write(path, json.as_bytes())
            .map_err(|e| libafl::Error::os_error(e, "writing corpus entry"))
    }

    fn from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, libafl::Error> {
        let data =
            std::fs::read(path).map_err(|e| libafl::Error::os_error(e, "reading corpus entry"))?;
        serde_json::from_slice(&data).map_err(|e| libafl::Error::serialize(e.to_string()))
    }
}

/// Build a minimal valid starting delta using UpdateFile so it hits the
/// existing /input baseline without EEXIST.
pub fn generate_seed() -> FsDelta {
    FsDelta::new(vec![FsOp::update_file("/input", b"seed".to_vec())])
}

/// Generate a set of structurally diverse seed deltas against known baseline paths.
pub fn generate_seed_corpus(baseline_files: &[String]) -> Vec<FsDelta> {
    let primary = baseline_files
        .first()
        .map(String::as_str)
        .unwrap_or("/input");
    let secondary = baseline_files
        .get(1)
        .map(String::as_str)
        .unwrap_or("/etc/config");

    vec![
        FsDelta::new(vec![FsOp::update_file(primary, b"seed".to_vec())]),
        FsDelta::new(vec![FsOp::update_file(
            secondary,
            b"[settings]\nverbose=1\n".to_vec(),
        )]),
        FsDelta::new(vec![FsOp::truncate(primary, 2)]),
        FsDelta::new(vec![FsOp::set_times(
            primary,
            1_700_000_000,
            0,
            1_700_000_000,
            0,
        )]),
        FsDelta::new(vec![
            FsOp::update_file(primary, b"fuzzed content".to_vec()),
            FsOp::set_times(primary, 1_000_000_000, 0, 1_000_000_000, 0),
        ]),
        FsDelta::new(vec![FsOp::create_file("/fuzz_input", b"new".to_vec())]),
        FsDelta::new(vec![
            FsOp::mkdir("/fuzz_dir"),
            FsOp::create_file("/fuzz_dir/file", b"hello".to_vec()),
        ]),
    ]
}

/// Fixed donor pool for SpliceDelta before a real corpus has accumulated.
pub fn initial_corpus_pool() -> Vec<FsDelta> {
    vec![
        FsDelta::new(vec![FsOp::update_file(
            "/etc/config",
            b"[settings]\nverbose=1\n".to_vec(),
        )]),
        FsDelta::new(vec![
            FsOp::mkdir("/data"),
            FsOp::create_file("/data/a.bin", vec![0xde, 0xad, 0xbe, 0xef]),
            FsOp::create_file("/data/b.txt", b"hello\n".to_vec()),
        ]),
        FsDelta::new(vec![FsOp::update_file(
            "/input",
            b"AAAAAAAAAAAAAAAA".to_vec(),
        )]),
        FsDelta::new(vec![
            FsOp::set_times("/input", 1_700_000_000, 0, 1_700_000_000, 0),
            FsOp::truncate("/input", 2),
        ]),
    ]
}

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
        let corpus = generate_seed_corpus(&[]);
        assert_eq!(corpus.len(), 7);
        for delta in &corpus {
            assert!(!delta.ops.is_empty());
        }
    }

    #[test]
    fn seed_one_uses_update_not_create() {
        // UpdateFile avoids EEXIST on /input which already exists in the baseline
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
