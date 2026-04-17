/*
 * mutators.rs — LibAFL mutator stages for FsDelta.
 *
 * Each struct implements libafl::mutators::Mutator<FsDelta, S> and
 * libafl_bolts::Named, as required by LibAFL 0.15.
 *
 * All stages accept a MutationGuidance field (empty in Phase A, populated
 * from the FUSE write log in Phase B).
 */

use std::borrow::Cow;
use std::num::NonZeroUsize;

use libafl::{
    corpus::CorpusId,
    mutators::{MutationResult, Mutator},
    state::HasRand,
    Error,
};
use libafl_bolts::{rands::Rand, Named};

use crate::{
    delta::{FsDelta, FsOp, FsOpKind},
    guidance::MutationGuidance,
};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Hard cap on the number of ops in a single delta.
/// AddFileOp and SpliceDelta skip rather than grow past this.
/// Prevents unbounded delta bloat across iterations.
pub const MAX_OPS: usize = 20;

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────

/// A small vocabulary of valid path components.
static PATH_COMPONENTS: &[&str] = &[
    "a", "b", "c", "d",
    "etc", "tmp", "var", "lib", "usr",
    "input", "output", "config", "data", "test", "run",
];

/// Wrap n in NonZeroUsize, panicking (programming error) if n == 0.
/// Only call this where callers have already checked n > 0.
#[inline]
fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("below() called with zero upper bound")
}

/// Pick a random element from a non-empty slice.
fn pick<'a, T, R: Rand>(rand: &mut R, slice: &'a [T]) -> &'a T {
    &slice[rand.below(nz(slice.len()))]
}

/// Generate a random absolute path with 1–3 components.
fn random_path<R: Rand>(rand: &mut R) -> String {
    let depth = 1 + rand.below(nz(3));
    let mut path = String::new();
    for _ in 0..depth {
        path.push('/');
        path.push_str(*pick(rand, PATH_COMPONENTS));
    }
    path
}

/// Generate random content: 1–64 random bytes.
fn random_content<R: Rand>(rand: &mut R) -> Vec<u8> {
    let len = 1 + rand.below(nz(64));
    (0..len).map(|_| rand.below(nz(256)) as u8).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. ByteFlipFileContent
// ─────────────────────────────────────────────────────────────────────────────

/// Flip 1–4 random bytes inside the content of a randomly chosen file op.
/// Skips if no file ops with non-empty content exist.
pub struct ByteFlipFileContent {
    pub guidance: MutationGuidance,
}

impl ByteFlipFileContent {
    pub fn new() -> Self {
        Self { guidance: MutationGuidance::none() }
    }
}

impl Named for ByteFlipFileContent {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("ByteFlipFileContent");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for ByteFlipFileContent
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        let candidates: Vec<usize> = input
            .ops
            .iter()
            .enumerate()
            .filter(|(_, op)| {
                matches!(op.kind, FsOpKind::CreateFile | FsOpKind::UpdateFile)
                    && !op.content.is_empty()
            })
            .map(|(i, _)| i)
            .collect();

        if candidates.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let chosen = *pick(state.rand_mut(), &candidates);
        let op = &mut input.ops[chosen];
        let content_len = op.content.len();

        let n_flips = 1 + state.rand_mut().below(nz(4));
        for _ in 0..n_flips {
            let byte_idx = state.rand_mut().below(nz(content_len));
            let flip_mask = 1u8 << state.rand_mut().below(nz(8));
            op.content[byte_idx] ^= flip_mask;
        }

        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. ReplaceFileContent
// ─────────────────────────────────────────────────────────────────────────────

/// Replace the entire content of a randomly chosen file op with fresh bytes.
pub struct ReplaceFileContent {
    pub guidance: MutationGuidance,
}

impl ReplaceFileContent {
    pub fn new() -> Self {
        Self { guidance: MutationGuidance::none() }
    }
}

impl Named for ReplaceFileContent {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("ReplaceFileContent");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for ReplaceFileContent
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        let candidates: Vec<usize> = input
            .ops
            .iter()
            .enumerate()
            .filter(|(_, op)| matches!(op.kind, FsOpKind::CreateFile | FsOpKind::UpdateFile))
            .map(|(i, _)| i)
            .collect();

        if candidates.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let chosen = *pick(state.rand_mut(), &candidates);
        let new_content = random_content(state.rand_mut());
        let op = &mut input.ops[chosen];
        op.size = new_content.len();
        op.content = new_content;

        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. AddFileOp
// ─────────────────────────────────────────────────────────────────────────────

/// Append a new CreateFile or Mkdir op with a random valid path.
/// Biased toward ENOENT paths from guidance when available.
pub struct AddFileOp {
    pub guidance: MutationGuidance,
}

impl AddFileOp {
    pub fn new() -> Self {
        Self { guidance: MutationGuidance::none() }
    }
}

impl Named for AddFileOp {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("AddFileOp");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for AddFileOp
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }

        let path = if self.guidance.has_enoent()
            && state.rand_mut().below(nz(100)) < 70
        {
            pick(state.rand_mut(), &self.guidance.enoent_paths).clone()
        } else {
            random_path(state.rand_mut())
        };

        let op = if state.rand_mut().below(nz(100)) < 70 {
            FsOp::create_file(path, random_content(state.rand_mut()))
        } else {
            FsOp::mkdir(path)
        };

        input.ops.push(op);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. RemoveOp
// ─────────────────────────────────────────────────────────────────────────────

/// Remove a random op from the delta.
/// Skips when only one op remains (empty delta is invalid).
pub struct RemoveOp {
    pub guidance: MutationGuidance,
}

impl RemoveOp {
    pub fn new() -> Self {
        Self { guidance: MutationGuidance::none() }
    }
}

impl Named for RemoveOp {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("RemoveOp");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for RemoveOp
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.len() <= 1 {
            return Ok(MutationResult::Skipped);
        }
        let idx = state.rand_mut().below(nz(input.ops.len()));
        input.ops.remove(idx);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. MutatePath
// ─────────────────────────────────────────────────────────────────────────────

/// Replace one path component of a randomly chosen op.
pub struct MutatePath {
    pub guidance: MutationGuidance,
}

impl MutatePath {
    pub fn new() -> Self {
        Self { guidance: MutationGuidance::none() }
    }
}

impl Named for MutatePath {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("MutatePath");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for MutatePath
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let op_idx = state.rand_mut().below(nz(input.ops.len()));
        let op = &mut input.ops[op_idx];

        let mut parts: Vec<&str> = op.path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let part_idx = state.rand_mut().below(nz(parts.len()));
        let new_component = *pick(state.rand_mut(), PATH_COMPONENTS);
        parts[part_idx] = new_component;

        op.path = format!("/{}", parts.join("/"));
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. SpliceDelta
// ─────────────────────────────────────────────────────────────────────────────

/// Take a random prefix of ops from a donor delta and append them to the
/// current delta.
pub struct SpliceDelta {
    pub guidance: MutationGuidance,
    /// Corpus pool for Phase A.  In Phase B, replaced by live corpus draws.
    pub corpus_pool: Vec<FsDelta>,
}

impl SpliceDelta {
    pub fn new(pool: Vec<FsDelta>) -> Self {
        Self {
            guidance: MutationGuidance::none(),
            corpus_pool: pool,
        }
    }
}

impl Named for SpliceDelta {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("SpliceDelta");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for SpliceDelta
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if self.corpus_pool.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let donor_idx = state.rand_mut().below(nz(self.corpus_pool.len()));
        let donor = &self.corpus_pool[donor_idx];

        if donor.ops.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let available = MAX_OPS.saturating_sub(input.ops.len());
        if available == 0 {
            return Ok(MutationResult::Skipped);
        }

        let max_take = donor.ops.len().min(available);
        let take_n = 1 + state.rand_mut().below(nz(max_take));
        for op in donor.ops.iter().take(take_n) {
            input.ops.push(op.clone());
        }

        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. DestructiveMutator
// ─────────────────────────────────────────────────────────────────────────────

/// Append a destructive or metadata op: DeleteFile, Rmdir, Truncate, or
/// SetTimes.  These are the four op kinds that the other six stages never
/// generate as primary ops, ensuring all seven FsOpKind variants are
/// reachable through the mutation pipeline.
pub struct DestructiveMutator {
    pub guidance: MutationGuidance,
}

impl DestructiveMutator {
    pub fn new() -> Self {
        Self { guidance: MutationGuidance::none() }
    }
}

impl Named for DestructiveMutator {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("DestructiveMutator");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for DestructiveMutator
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }

        let path = random_path(state.rand_mut());
        let op = match state.rand_mut().below(nz(4)) {
            0 => FsOp::delete_file(&path),
            1 => FsOp::rmdir(&path),
            2 => {
                let new_size = state.rand_mut().below(nz(1024));
                FsOp::truncate(&path, new_size)
            }
            _ => {
                let mtime_sec = state.rand_mut().below(nz(u32::MAX as usize)) as i64;
                let atime_sec = state.rand_mut().below(nz(u32::MAX as usize)) as i64;
                FsOp::set_times(&path, mtime_sec, 0, atime_sec, 0)
            }
        };

        input.ops.push(op);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::{FsDelta, FsOp};
    use libafl::mutators::MutationResult;
    use libafl::state::HasRand;
    use libafl_bolts::rands::StdRand;

    /// Minimal state that satisfies HasRand — mirrors DumbState in fuzz.rs.
    struct TestState {
        rand: StdRand,
    }

    impl TestState {
        fn new() -> Self {
            Self { rand: StdRand::with_seed(0xdeadbeef_cafebabe) }
        }
    }

    impl HasRand for TestState {
        type Rand = StdRand;
        fn rand(&self) -> &StdRand { &self.rand }
        fn rand_mut(&mut self) -> &mut StdRand { &mut self.rand }
    }

    fn file_delta() -> FsDelta {
        FsDelta::new(vec![FsOp::create_file("/input", b"hello world".to_vec())])
    }

    fn multi_op_delta() -> FsDelta {
        FsDelta::new(vec![
            FsOp::create_file("/a.txt", b"aaaa".to_vec()),
            FsOp::mkdir("/dir"),
            FsOp::create_file("/dir/b.txt", b"bbbb".to_vec()),
        ])
    }

    // ── 1. ByteFlipFileContent ────────────────────────────────────────────

    #[test]
    fn byte_flip_mutates_content() {
        let mut state = TestState::new();
        let mut m = ByteFlipFileContent::new();
        let mut delta = file_delta();
        let original = delta.ops[0].content.clone();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        // At least one byte must differ after flipping.
        assert_ne!(delta.ops[0].content, original);
    }

    #[test]
    fn byte_flip_skips_when_no_file_ops() {
        let mut state = TestState::new();
        let mut m = ByteFlipFileContent::new();
        // Delta with only a Mkdir op (no content).
        let mut delta = FsDelta::new(vec![FsOp::mkdir("/empty")]);
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
    }

    // ── 2. ReplaceFileContent ─────────────────────────────────────────────

    #[test]
    fn replace_file_content_changes_content_and_size() {
        let mut state = TestState::new();
        let mut m = ReplaceFileContent::new();
        let mut delta = file_delta();
        let original_content = delta.ops[0].content.clone();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        // Content should change (with overwhelming probability with a fixed seed).
        assert_ne!(delta.ops[0].content, original_content);
        // size field must be kept in sync with the new content length.
        assert_eq!(delta.ops[0].size, delta.ops[0].content.len());
    }

    #[test]
    fn replace_file_content_skips_when_no_file_ops() {
        let mut state = TestState::new();
        let mut m = ReplaceFileContent::new();
        let mut delta = FsDelta::new(vec![FsOp::rmdir("/gone")]);
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
    }

    // ── 3. AddFileOp ─────────────────────────────────────────────────────

    #[test]
    fn add_file_op_grows_delta() {
        let mut state = TestState::new();
        let mut m = AddFileOp::new();
        let mut delta = file_delta();
        let before = delta.ops.len();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert_eq!(delta.ops.len(), before + 1);
        // New op must be either CreateFile or Mkdir.
        let new_op = delta.ops.last().unwrap();
        assert!(
            matches!(new_op.kind, FsOpKind::CreateFile | FsOpKind::Mkdir),
            "unexpected kind: {:?}", new_op.kind
        );
        // Path must be absolute.
        assert!(new_op.path.starts_with('/'));
    }

    #[test]
    fn add_file_op_uses_guidance_enoent_paths() {
        let mut state = TestState::new();
        let mut m = AddFileOp::new();
        m.guidance.enoent_paths = vec!["/guided/path".to_string()];
        let delta = file_delta();

        // Run many times; at least one should use the guided path (70% bias).
        let mut used_guided = false;
        for _ in 0..50 {
            let mut d = delta.clone();
            m.mutate(&mut state, &mut d).unwrap();
            if d.ops.last().unwrap().path == "/guided/path" {
                used_guided = true;
                break;
            }
        }
        assert!(used_guided, "guided path was never chosen in 50 tries");
    }

    // ── 4. RemoveOp ───────────────────────────────────────────────────────

    #[test]
    fn remove_op_shrinks_delta() {
        let mut state = TestState::new();
        let mut m = RemoveOp::new();
        let mut delta = multi_op_delta();
        let before = delta.ops.len();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert_eq!(delta.ops.len(), before - 1);
    }

    #[test]
    fn remove_op_skips_single_op_delta() {
        let mut state = TestState::new();
        let mut m = RemoveOp::new();
        let mut delta = file_delta(); // exactly 1 op
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
        assert_eq!(delta.ops.len(), 1);
    }

    // ── 5. MutatePath ────────────────────────────────────────────────────

    #[test]
    fn mutate_path_changes_a_component() {
        let mut state = TestState::new();
        let mut m = MutatePath::new();
        // Run enough times to get a path that differs (not same component chosen).
        let original_path = "/input".to_string();
        let mut changed = false;
        for _ in 0..20 {
            let mut delta = file_delta();
            m.mutate(&mut state, &mut delta).unwrap();
            if delta.ops[0].path != original_path {
                changed = true;
                // Result must still be an absolute path.
                assert!(delta.ops[0].path.starts_with('/'));
                break;
            }
        }
        assert!(changed, "path never changed across 20 attempts");
    }

    #[test]
    fn mutate_path_skips_empty_delta() {
        let mut state = TestState::new();
        let mut m = MutatePath::new();
        let mut delta = FsDelta::new(vec![]);
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
    }

    // ── 6. SpliceDelta ───────────────────────────────────────────────────

    #[test]
    fn splice_delta_appends_ops_from_donor() {
        let mut state = TestState::new();
        let donor = FsDelta::new(vec![
            FsOp::mkdir("/splice_a"),
            FsOp::create_file("/splice_b", b"data".to_vec()),
        ]);
        let mut m = SpliceDelta::new(vec![donor]);
        let mut delta = file_delta();
        let before = delta.ops.len();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert!(delta.ops.len() > before, "splice should append at least 1 op");
    }

    #[test]
    fn splice_delta_skips_empty_pool() {
        let mut state = TestState::new();
        let mut m = SpliceDelta::new(vec![]);
        let mut delta = file_delta();
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
    }

    // ── 7. DestructiveMutator ────────────────────────────────────────────

    #[test]
    fn destructive_mutator_grows_delta() {
        let mut state = TestState::new();
        let mut m = DestructiveMutator::new();
        let mut delta = file_delta();
        let before = delta.ops.len();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert_eq!(delta.ops.len(), before + 1);

        // The new op must be one of the four destructive kinds.
        let new_op = delta.ops.last().unwrap();
        assert!(
            matches!(
                new_op.kind,
                FsOpKind::DeleteFile | FsOpKind::Rmdir | FsOpKind::Truncate | FsOpKind::SetTimes
            ),
            "unexpected kind: {:?}", new_op.kind
        );
        assert!(new_op.path.starts_with('/'));
    }

    #[test]
    fn destructive_mutator_generates_all_four_kinds() {
        let mut state = TestState::new();
        let mut m = DestructiveMutator::new();
        let mut seen = std::collections::HashSet::new();

        // Run enough iterations to expect all four kinds with a fixed seed.
        for _ in 0..200 {
            let mut delta = file_delta();
            m.mutate(&mut state, &mut delta).unwrap();
            seen.insert(std::mem::discriminant(&delta.ops.last().unwrap().kind));
        }
        // All four destructive variants should appear within 200 tries.
        assert_eq!(seen.len(), 4, "not all destructive kinds were generated");
    }

    // ── MAX_OPS cap ──────────────────────────────────────────────────────

    #[test]
    fn add_file_op_skips_at_max_ops() {
        let mut state = TestState::new();
        let mut m = AddFileOp::new();
        // Build a delta that is exactly at the cap.
        let ops = (0..MAX_OPS)
            .map(|i| FsOp::create_file(format!("/f{i}"), vec![i as u8]))
            .collect();
        let mut delta = FsDelta::new(ops);
        assert_eq!(delta.ops.len(), MAX_OPS);

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
        assert_eq!(delta.ops.len(), MAX_OPS, "delta grew past MAX_OPS");
    }

    #[test]
    fn splice_delta_skips_at_max_ops() {
        let mut state = TestState::new();
        let donor = FsDelta::new(vec![FsOp::mkdir("/extra")]);
        let mut m = SpliceDelta::new(vec![donor]);
        let ops = (0..MAX_OPS)
            .map(|i| FsOp::create_file(format!("/f{i}"), vec![i as u8]))
            .collect();
        let mut delta = FsDelta::new(ops);

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
        assert_eq!(delta.ops.len(), MAX_OPS);
    }

    #[test]
    fn destructive_mutator_skips_at_max_ops() {
        let mut state = TestState::new();
        let mut m = DestructiveMutator::new();
        let ops = (0..MAX_OPS)
            .map(|i| FsOp::create_file(format!("/f{i}"), vec![i as u8]))
            .collect();
        let mut delta = FsDelta::new(ops);

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
        assert_eq!(delta.ops.len(), MAX_OPS);
    }
}
