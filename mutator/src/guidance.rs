/*
 * guidance.rs — Mutation guidance from the FUSE write log.
 *
 * Phase A: all fields default to empty — no guidance is used.
 * Phase B: the harness loop populates this from fuse_iter_log_t after each
 *          target run and passes it to the mutator stages so they can bias
 *          their decisions toward paths the target actually cares about.
 */

/// Feedback signals extracted from the per-iteration FUSE write log.
///
/// Passed to every mutator stage.  When all fields are empty (Phase A)
/// the mutators fall back to fully random behavior.
#[derive(Clone, Debug, Default)]
pub struct MutationGuidance {
    /// Paths the target requested but which did not exist (ENOENT from
    /// fvfs_getattr).  AddFileOp biases toward creating these paths.
    pub enoent_paths: Vec<String>,

    /// Paths the target deleted (UNLINK/RMDIR) or renamed away
    /// (RENAME_FROM).  The target reached code that acts on these paths,
    /// so recreating them in future iterations exercises the same code.
    pub recreate_paths: Vec<String>,
}

impl MutationGuidance {
    /// No guidance — Phase A default.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn has_enoent(&self) -> bool {
        !self.enoent_paths.is_empty()
    }

    pub fn has_recreate(&self) -> bool {
        !self.recreate_paths.is_empty()
    }
}
