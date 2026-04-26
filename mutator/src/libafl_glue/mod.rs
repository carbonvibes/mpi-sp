/*
 * libafl_glue/mod.rs — bridge between FsDelta semantics and LibAFL primitives.
 *
 * Phase B adds:
 *   primary_content()  — extract the byte payload the target will consume
 *
 * Phase C will add:
 *   FuseLogObserver    — drains per-iteration FUSE write log → MutationGuidance
 *   FsAccessFeedback   — treats novel enoent/write-set paths as interesting
 */

use crate::delta::{FsDelta, FsOpKind};

/// Extract the primary fuzz content from a FsDelta.
///
/// Returns the content bytes of the first CreateFile or UpdateFile op, or an
/// empty slice when the delta has no file-content ops (e.g. a metadata-only
/// delta of Truncate + SetTimes ops).
///
/// This is the semantic bridge: the fuzzer mutates a FsDelta (a series of
/// filesystem operations), and this function distils the "what bytes does the
/// target actually read" view of that delta for in-process target harnesses.
/// Once Phase C wires the FUSE mount, the full delta is applied to the VFS and
/// the target reads through the mount — this function is only needed for the
/// in-process (no-FUSE) Phase B campaigns.
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
