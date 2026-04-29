/// Feedback signals extracted from the per-iteration FUSE write log.
/// When all fields are empty the mutators fall back to fully random behavior.
#[derive(Clone, Debug, Default)]
pub struct MutationGuidance {
    /// paths the target wrote to, created, or renamed into
    pub write_paths: Vec<String>,

    /// paths the target requested but which didn't exist (ENOENT from getattr)
    pub enoent_paths: Vec<String>,

    /// paths the target deleted or renamed away
    pub recreate_paths: Vec<String>,
}

impl MutationGuidance {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn has_write(&self) -> bool {
        !self.write_paths.is_empty()
    }

    pub fn has_enoent(&self) -> bool {
        !self.enoent_paths.is_empty()
    }

    pub fn has_recreate(&self) -> bool {
        !self.recreate_paths.is_empty()
    }
}
