use crate::delta::{FsDelta, FsOpKind};

/// Returns content of the first CreateFile/UpdateFile op, or empty for metadata-only deltas.
pub fn primary_content(delta: &FsDelta) -> &[u8] {
    delta
        .ops
        .iter()
        .find(|op| matches!(op.kind, FsOpKind::CreateFile | FsOpKind::UpdateFile))
        .map(|op| op.content.as_slice())
        .unwrap_or(b"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::{FsDelta, FsOp};

    #[test]
    fn primary_content_returns_first_file_op_content() {
        let delta = FsDelta::new(vec![
            FsOp::update_file("/input", b"hello".to_vec()),
            FsOp::update_file("/other", b"world".to_vec()),
        ]);
        assert_eq!(primary_content(&delta), b"hello");
    }

    #[test]
    fn primary_content_skips_non_file_ops() {
        let delta = FsDelta::new(vec![
            FsOp::mkdir("/tmp"),
            FsOp::create_file("/tmp/f", b"data".to_vec()),
        ]);
        assert_eq!(primary_content(&delta), b"data");
    }

    #[test]
    fn primary_content_returns_empty_for_metadata_only_delta() {
        let delta = FsDelta::new(vec![
            FsOp::truncate("/input", 4),
            FsOp::set_times("/input", 0, 0, 0, 0),
        ]);
        assert_eq!(primary_content(&delta), b"");
    }
}
