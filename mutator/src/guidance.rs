use std::sync::{Mutex, OnceLock};

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

// ── Live guidance (populated by FuseLogObserver, read by mutators) ─────────
//
// The fuzzer is single-threaded from the mutators' perspective.
// The FUSE daemon runs in a background thread but only writes to the log
// while the target is running — before mutators are called. The Mutex
// protects the brief overlap at active→inactive transition.

static LIVE_GUIDANCE: OnceLock<Mutex<MutationGuidance>> = OnceLock::new();

fn live() -> &'static Mutex<MutationGuidance> {
    LIVE_GUIDANCE.get_or_init(|| Mutex::new(MutationGuidance::none()))
}

/// Called by FuseLogObserver::post_exec to publish fresh guidance.
pub fn update_live(g: MutationGuidance) {
    *live().lock().unwrap() = g;
}

/// Called by mutators in mutate() to read the latest guidance.
/// Returns a clone so mutators don't hold the lock during mutation.
pub fn peek_live() -> MutationGuidance {
    live().lock().unwrap().clone()
}
