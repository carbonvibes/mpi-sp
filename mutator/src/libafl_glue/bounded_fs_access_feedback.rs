use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

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

/// Like [`super::fs_access_feedback::FsAccessFeedback`], this promotes a corpus
/// entry when the target reaches a filesystem path it has never reached before
/// (new ENOENT or new write path) — even if the input covers no new AFL++ edges.
///
/// The difference is how "never before" is decided. The original keys novelty on
/// the **exact path string** via an unbounded `HashSet<String>`; because the
/// config grammar + symlink mutators can mint endless distinct paths, that set —
/// and therefore the corpus — grows without bound.
///
/// This version keys novelty on a **bounded hashed bitmap**, exactly like AFL
/// edge coverage (`hash(path) % MAP_SIZE`): a path is "new" only if its bucket
/// was never set. The number of distinct buckets is capped at `MAP_SIZE`, so the
/// corpus contribution from filesystem novelty **plateaus** instead of bloating.
/// Hash collisions merge a few distinct paths into one bucket — the same
/// resolution/coverage trade AFL already makes for edges.
#[derive(Debug, Serialize, Deserialize)]
pub struct BoundedFsAccessFeedback {
    name: Cow<'static, str>,
    /// Virgin path-map: one byte per bucket, 0 = unseen, 1 = seen. Persists for
    /// the whole campaign so novelty is measured against everything seen so far.
    seen: Vec<u8>,
    /// `map_size - 1`; `seen.len()` is always a power of two so this masks the hash.
    mask: usize,
}

impl BoundedFsAccessFeedback {
    /// `map_size` is rounded up to the next power of two. It is the hard ceiling
    /// on how many distinct path-buckets (and thus FS-driven corpus entries) can
    /// ever be admitted.
    pub fn new(map_size: usize) -> Self {
        let size = map_size.max(1).next_power_of_two();
        Self {
            name: Cow::Borrowed("bounded_fs_access"),
            seen: vec![0u8; size],
            mask: size - 1,
        }
    }

    /// Marks `path`'s bucket as seen; returns true only the first time that
    /// bucket transitions 0 -> 1 (i.e. a never-before-hit path bucket).
    fn note(&mut self, path: &str) -> bool {
        let mut h = DefaultHasher::new();
        path.hash(&mut h);
        let idx = (h.finish() as usize) & self.mask;
        if self.seen[idx] == 0 {
            self.seen[idx] = 1;
            true
        } else {
            false
        }
    }
}

impl Named for BoundedFsAccessFeedback {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<S> StateInitializer<S> for BoundedFsAccessFeedback {}

impl<EM, I, OT, S> Feedback<EM, I, OT, S> for BoundedFsAccessFeedback
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
            if self.note(path) {
                novel = true;
            }
        }
        for path in &guidance.write_paths {
            if self.note(path) {
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

    #[test]
    fn new_bucket_is_interesting_repeat_is_not() {
        let mut fb = BoundedFsAccessFeedback::new(1024);
        assert!(fb.note("/wanted"), "first time a path bucket is hit is novel");
        assert!(!fb.note("/wanted"), "same path again is not novel");
    }

    #[test]
    fn distinct_paths_each_novel_until_map_saturates() {
        // With a tiny 2-bucket map, at most 2 buckets can ever be novel no matter
        // how many distinct paths arrive — this is what bounds the corpus.
        let mut fb = BoundedFsAccessFeedback::new(2);
        let mut novel_count = 0;
        for i in 0..1000 {
            if fb.note(&format!("/p/{i}")) {
                novel_count += 1;
            }
        }
        assert!(
            novel_count <= 2,
            "bounded map admitted {novel_count} > 2 buckets — not bounded"
        );
        assert!(novel_count >= 1, "expected at least one novel bucket");
    }

    #[test]
    fn map_size_rounded_to_power_of_two() {
        let fb = BoundedFsAccessFeedback::new(1000);
        assert_eq!(fb.seen.len(), 1024);
        assert_eq!(fb.mask, 1023);
    }
}
