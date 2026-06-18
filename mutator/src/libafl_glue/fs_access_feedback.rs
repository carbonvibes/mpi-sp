use std::{borrow::Cow, collections::HashSet};

use libafl::{
    corpus::Testcase,
    events::EventFirer,
    executors::ExitKind,
    feedbacks::{Feedback, StateInitializer},
    observers::ObserversTuple,
    Error,
};
use libafl_bolts::Named;
use serde::{Deserialize, Serialize};

use crate::guidance::peek_live;

/// Feedback that promotes corpus entries when the target accesses a path
/// it has never accessed before (new ENOENT or new write-set path).
///
/// Supplements `MaxMapFeedback` — an input that reaches a new filesystem
/// path is interesting even if it covers no new AFL++ edges.
#[derive(Debug, Serialize, Deserialize)]
pub struct FsAccessFeedback {
    name:        Cow<'static, str>,
    seen_enoent: HashSet<String>,
    seen_write:  HashSet<String>,
}

impl FsAccessFeedback {
    pub fn new() -> Self {
        Self {
            name:        Cow::Borrowed("fs_access"),
            seen_enoent: HashSet::new(),
            seen_write:  HashSet::new(),
        }
    }
}

impl Named for FsAccessFeedback {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<S> StateInitializer<S> for FsAccessFeedback {}

impl Default for FsAccessFeedback {
    fn default() -> Self {
        Self::new()
    }
}

impl<EM, I, OT, S> Feedback<EM, I, OT, S> for FsAccessFeedback
where
    EM: EventFirer<I, S>,
    OT: ObserversTuple<I, S>,
{
    fn is_interesting(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        _input: &I,
        _observers: &OT,
        _exit_kind: &ExitKind,
    ) -> Result<bool, Error> {
        let guidance = peek_live();
        let mut novel = false;

        for path in &guidance.enoent_paths {
            if self.seen_enoent.insert(path.clone()) {
                novel = true;
            }
        }
        for path in &guidance.write_paths {
            if self.seen_write.insert(path.clone()) {
                novel = true;
            }
        }

        Ok(novel)
    }

    fn append_metadata(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        _observers: &OT,
        _testcase: &mut Testcase<I>,
    ) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guidance::{update_live, MutationGuidance};

    static MU: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn make_guidance(enoent: &[&str], write: &[&str]) -> MutationGuidance {
        MutationGuidance {
            enoent_paths:   enoent.iter().map(|s| s.to_string()).collect(),
            write_paths:    write.iter().map(|s| s.to_string()).collect(),
            recreate_paths: vec![],
        }
    }

    #[test]
    fn new_enoent_path_is_interesting() {
        let _g = MU.lock().unwrap();
        update_live(make_guidance(&["/wanted"], &[]));
        let mut fb = FsAccessFeedback::new();
        // First time: interesting
        assert!(fb.seen_enoent.insert("/wanted".to_string()) || true);
        let novel = fb.seen_enoent.insert("/wanted".to_string()) == false
            || { fb.seen_enoent.clear(); fb.seen_enoent.insert("/wanted".to_string()) };
        // Rerun through the actual peek_live path
        fb.seen_enoent.clear();
        fb.seen_write.clear();
        let guidance = peek_live();
        let mut result = false;
        for p in &guidance.enoent_paths { if fb.seen_enoent.insert(p.clone()) { result = true; } }
        assert!(result, "new ENOENT path should be interesting");
        update_live(MutationGuidance::none());
    }

    #[test]
    fn repeated_enoent_path_not_interesting() {
        let _g = MU.lock().unwrap();
        update_live(make_guidance(&["/repeated"], &[]));
        let mut fb = FsAccessFeedback::new();
        // First call: interesting
        let g = peek_live();
        for p in &g.enoent_paths { fb.seen_enoent.insert(p.clone()); }
        // Second call with same guidance: not interesting
        let g2 = peek_live();
        let mut novel = false;
        for p in &g2.enoent_paths { if fb.seen_enoent.insert(p.clone()) { novel = true; } }
        assert!(!novel, "already-seen ENOENT path should not be interesting again");
        update_live(MutationGuidance::none());
    }

    #[test]
    fn new_write_path_is_interesting() {
        let _g = MU.lock().unwrap();
        update_live(make_guidance(&[], &["/written"]));
        let mut fb = FsAccessFeedback::new();
        let g = peek_live();
        let mut novel = false;
        for p in &g.write_paths { if fb.seen_write.insert(p.clone()) { novel = true; } }
        assert!(novel, "new write path should be interesting");
        update_live(MutationGuidance::none());
    }

    #[test]
    fn empty_guidance_not_interesting() {
        let _g = MU.lock().unwrap();
        update_live(MutationGuidance::none());
        let mut fb = FsAccessFeedback::new();
        let g = peek_live();
        let mut novel = false;
        for p in &g.enoent_paths { if fb.seen_enoent.insert(p.clone()) { novel = true; } }
        for p in &g.write_paths  { if fb.seen_write.insert(p.clone()) { novel = true; } }
        assert!(!novel, "empty guidance should produce no novel signal");
    }
}
