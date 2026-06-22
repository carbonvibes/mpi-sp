use std::borrow::Cow;
use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::rc::Rc;

use libafl::common::HasMetadata;
use libafl::{
    corpus::CorpusId,
    mutators::{MutationResult, Mutator},
    observers::cmp::{CmpValues, CmpValuesMetadata},
    state::HasRand,
    Error,
};
use libafl_bolts::{rands::Rand, Named};

use crate::{
    delta::{FsDelta, FsOp, FsOpKind},
    guidance::MutationGuidance,
};

pub const MAX_OPS: usize = 48;

/// evicts a random non-seed entry when full
pub const MAX_LIVE_CORPUS: usize = 128;

/// shared; harness pushes between mutator calls so needs interior mutability
pub type LiveCorpus = Rc<RefCell<Vec<crate::delta::FsDelta>>>;

static PATH_COMPONENTS: &[&str] = &[
    "a", "b", "c", "d", "etc", "tmp", "var", "lib", "usr", "input", "output", "config", "data",
    "test", "run",
];

// 40% chance to draw from this dictionary; rest is random bytes.
static CONTENT_DICTIONARY: &[&[u8]] = &[
    b"random_shit",
    b"cone_ice",
    b"paal_ice",
    b"chocobar",
    b"fahhhhhhh",
    b"",
    b"\x7fELF",                          // ELF magic
    b"#!/bin/sh\n",
    b"[settings]\nverbose=1\ndebug=1\n",
    b"\x00\x00\x00\x00",
    b"\xff\xff\xff\xff",
    b"../../../etc/passwd",
    b"/dev/null",
    b"%s%s%s%s",                         // format string
    b"A",
    &[0xAA; 64],
    &[0x00; 256],
    &[0x41; 4096],
];

#[inline]
fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("below() called with zero upper bound")
}

fn pick<'a, T, R: Rand>(rand: &mut R, slice: &'a [T]) -> &'a T {
    &slice[rand.below(nz(slice.len()))]
}

fn random_path<R: Rand>(rand: &mut R) -> String {
    let depth = 1 + rand.below(nz(3));
    let mut path = String::new();
    for _ in 0..depth {
        path.push('/');
        path.push_str(*pick(rand, PATH_COMPONENTS));
    }
    path
}

fn random_content<R: Rand>(rand: &mut R) -> Vec<u8> {
    let len = 1 + rand.below(nz(64));
    (0..len).map(|_| rand.below(nz(256)) as u8).collect()
}

fn pick_or_random<R: Rand>(rand: &mut R, baseline: &[String], bias_pct: usize) -> String {
    if !baseline.is_empty() && rand.below(nz(100)) < bias_pct {
        pick(rand, baseline).clone()
    } else {
        random_path(rand)
    }
}

fn pick_timestamp<R: Rand>(rand: &mut R) -> i64 {
    const INTERESTING: &[i64] = &[
        0,
        -1,
        i32::MAX as i64, // 2038 overflow boundary
        2_000_000_000,
        1_700_000_000,
    ];
    if rand.below(nz(100)) < 40 {
        *pick(rand, INTERESTING)
    } else {
        rand.below(nz(u32::MAX as usize)) as i64
    }
}

/// sets 1-4 bytes in a file op, or appends up to 8 (20%)
pub struct ByteFlipFileContent {
    pub guidance: MutationGuidance,
}

impl ByteFlipFileContent {
    pub fn new() -> Self {
        Self {
            guidance: MutationGuidance::none(),
        }
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

        if state.rand_mut().below(nz(100)) < 20 {
            let n_append = 1 + state.rand_mut().below(nz(8));
            for _ in 0..n_append {
                op.content.push(state.rand_mut().below(nz(256)) as u8);
            }
            op.size = op.content.len();
            return Ok(MutationResult::Mutated);
        }

        let n_sets = 1 + state.rand_mut().below(nz(4));
        for _ in 0..n_sets {
            let byte_idx = state.rand_mut().below(nz(content_len));
            op.content[byte_idx] = state.rand_mut().below(nz(256)) as u8;
        }

        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

/// replaces entire content of a random file op
pub struct ReplaceFileContent {
    pub guidance: MutationGuidance,
}

impl ReplaceFileContent {
    pub fn new() -> Self {
        Self {
            guidance: MutationGuidance::none(),
        }
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

        let new_content = if state.rand_mut().below(nz(100)) < 40 {
            pick(state.rand_mut(), CONTENT_DICTIONARY).to_vec()
        } else {
            random_content(state.rand_mut())
        };

        let op = &mut input.ops[chosen];
        op.size = new_content.len();
        op.content = new_content;

        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

/// appends a CreateFile or Mkdir op
pub struct AddFileOp {
    pub guidance: MutationGuidance,
}

impl AddFileOp {
    pub fn new() -> Self {
        Self {
            guidance: MutationGuidance::none(),
        }
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

        let guidance = crate::guidance::peek_live();
        let using_guided = guidance.has_enoent() && state.rand_mut().below(nz(100)) < 70;

        let path = if using_guided {
            pick(state.rand_mut(), &guidance.enoent_paths).clone()
        } else {
            random_path(state.rand_mut())
        };

        let file_bias = if using_guided { 90 } else { 70 };
        let op = if state.rand_mut().below(nz(100)) < file_bias {
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

/// removes a random op; skips if only one remains
pub struct RemoveOp {
    pub guidance: MutationGuidance,
}

impl RemoveOp {
    pub fn new() -> Self {
        Self {
            guidance: MutationGuidance::none(),
        }
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

/// mutates an op's path, respecting op semantics (see branches below)
pub struct MutatePath {
    pub guidance: MutationGuidance,
    pub baseline_file_paths: Vec<String>,
    pub baseline_dir_paths: Vec<String>,
    pub baseline_all_paths: Vec<String>,
}

impl MutatePath {
    pub fn new() -> Self {
        Self {
            guidance: MutationGuidance::none(),
            baseline_file_paths: vec![],
            baseline_dir_paths: vec![],
            baseline_all_paths: vec![],
        }
    }

    pub fn with_baseline(
        file_paths: Vec<String>,
        dir_paths: Vec<String>,
        all_paths: Vec<String>,
    ) -> Self {
        Self {
            guidance: MutationGuidance::none(),
            baseline_file_paths: file_paths,
            baseline_dir_paths: dir_paths,
            baseline_all_paths: all_paths,
        }
    }

    pub fn with_guidance(mut self, g: MutationGuidance) -> Self {
        self.guidance = g;
        self
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

        let new_path = match op.kind {
            // must hit a real baseline file or it's a no-op
            FsOpKind::UpdateFile | FsOpKind::Truncate => {
                if self.baseline_file_paths.is_empty() {
                    return Ok(MutationResult::Skipped);
                }
                pick(state.rand_mut(), &self.baseline_file_paths).clone()
            }

            // 80% real files, 20% random to hit ENOENT
            FsOpKind::DeleteFile => {
                if !self.baseline_file_paths.is_empty() && state.rand_mut().below(nz(100)) < 80 {
                    pick(state.rand_mut(), &self.baseline_file_paths).clone()
                } else {
                    random_path(state.rand_mut())
                }
            }

            FsOpKind::Rmdir => {
                if !self.baseline_dir_paths.is_empty() && state.rand_mut().below(nz(100)) < 80 {
                    pick(state.rand_mut(), &self.baseline_dir_paths).clone()
                } else {
                    random_path(state.rand_mut())
                }
            }

            FsOpKind::SetTimes => {
                pick_or_random(state.rand_mut(), &self.baseline_all_paths, 80)
            }

            // bias toward FUSE-guided paths (enoent/write/recreate)
            FsOpKind::CreateFile | FsOpKind::Mkdir | FsOpKind::CreateSymlink => {
                let guidance = crate::guidance::peek_live();
                let have_swap_target = guidance.has_enoent()
                    || guidance.has_write()
                    || guidance.has_recreate()
                    || !self.baseline_all_paths.is_empty();
                if have_swap_target && state.rand_mut().below(nz(100)) < 30 {
                    let pool: &[String] = if guidance.has_enoent() {
                        &guidance.enoent_paths
                    } else if guidance.has_write() {
                        &guidance.write_paths
                    } else if guidance.has_recreate() {
                        &guidance.recreate_paths
                    } else {
                        &self.baseline_all_paths
                    };
                    pick(state.rand_mut(), pool).clone()
                } else {
                    let mut parts: Vec<&str> =
                        op.path.split('/').filter(|s| !s.is_empty()).collect();
                    if parts.is_empty() {
                        random_path(state.rand_mut())
                    } else {
                        let part_idx = state.rand_mut().below(nz(parts.len()));
                        parts[part_idx] = *pick(state.rand_mut(), PATH_COMPONENTS);
                        format!("/{}", parts.join("/"))
                    }
                }
            }
        };

        op.path = new_path;
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

/// appends a random slice of ops from a donor delta in the pool
pub struct SpliceDelta {
    pub guidance: MutationGuidance,
    pub corpus_pool: LiveCorpus,
}

impl SpliceDelta {
    pub fn new(pool: LiveCorpus) -> Self {
        Self {
            guidance: MutationGuidance::none(),
            corpus_pool: pool,
        }
    }

    pub fn new_fixed(pool: Vec<FsDelta>) -> Self {
        Self {
            guidance: MutationGuidance::none(),
            corpus_pool: Rc::new(RefCell::new(pool)),
        }
    }

    pub fn pool_len(&self) -> usize {
        self.corpus_pool.borrow().len()
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
        let pool = self.corpus_pool.borrow();
        if pool.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let donor_idx = state.rand_mut().below(nz(pool.len()));
        let donor = &pool[donor_idx];

        if donor.ops.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let available = MAX_OPS.saturating_sub(input.ops.len());
        if available == 0 {
            return Ok(MutationResult::Skipped);
        }

        let start = state.rand_mut().below(nz(donor.ops.len()));
        let donor_slice = &donor.ops[start..];

        if donor_slice.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let max_take = donor_slice.len().min(available);
        let take_n = 1 + state.rand_mut().below(nz(max_take));
        for op in donor_slice.iter().take(take_n) {
            input.ops.push(op.clone());
        }

        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

/// appends a DeleteFile/Rmdir/Truncate/SetTimes op
pub struct DestructiveMutator {
    pub guidance: MutationGuidance,
    pub baseline_file_paths: Vec<String>,
    pub baseline_dir_paths: Vec<String>,
    pub baseline_all_paths: Vec<String>,
}

impl DestructiveMutator {
    pub fn new() -> Self {
        Self {
            guidance: MutationGuidance::none(),
            baseline_file_paths: vec![],
            baseline_dir_paths: vec![],
            baseline_all_paths: vec![],
        }
    }

    pub fn with_baseline(
        file_paths: Vec<String>,
        dir_paths: Vec<String>,
        all_paths: Vec<String>,
    ) -> Self {
        Self {
            guidance: MutationGuidance::none(),
            baseline_file_paths: file_paths,
            baseline_dir_paths: dir_paths,
            baseline_all_paths: all_paths,
        }
    }

    pub fn with_guidance(mut self, g: MutationGuidance) -> Self {
        self.guidance = g;
        self
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

        let guidance = crate::guidance::peek_live();
        let op = match state.rand_mut().below(nz(4)) {
            0 => {
                let path = if guidance.has_recreate() && state.rand_mut().below(nz(100)) < 50 {
                    pick(state.rand_mut(), &guidance.recreate_paths).clone()
                } else {
                    pick_or_random(state.rand_mut(), &self.baseline_file_paths, 70)
                };
                FsOp::delete_file(path)
            }
            1 => {
                let path = if guidance.has_recreate() && state.rand_mut().below(nz(100)) < 50 {
                    pick(state.rand_mut(), &guidance.recreate_paths).clone()
                } else {
                    pick_or_random(state.rand_mut(), &self.baseline_dir_paths, 70)
                };
                FsOp::rmdir(path)
            }
            2 => {
                let path = pick_or_random(state.rand_mut(), &self.baseline_file_paths, 70);
                let new_size = state.rand_mut().below(nz(1024));
                FsOp::truncate(path, new_size)
            }
            _ => {
                let path = pick_or_random(state.rand_mut(), &self.baseline_all_paths, 70);
                let mtime_sec = pick_timestamp(state.rand_mut());
                let atime_sec = pick_timestamp(state.rand_mut());
                FsOp::set_times(path, mtime_sec, 0, atime_sec, 0)
            }
        };

        input.ops.push(op);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

/// appends an UpdateFile op on a baseline file; perturbs real content 50% when baseline_contents set
pub struct UpdateExistingFile {
    pub guidance: MutationGuidance,
    pub baseline_file_paths: Vec<String>,
    pub baseline_contents: Vec<(String, Vec<u8>)>,
}

impl UpdateExistingFile {
    pub fn new(file_paths: Vec<String>) -> Self {
        Self {
            guidance: MutationGuidance::none(),
            baseline_file_paths: file_paths,
            baseline_contents: Vec::new(),
        }
    }

    pub fn with_baseline_contents(mut self, contents: Vec<(String, Vec<u8>)>) -> Self {
        self.baseline_contents = contents;
        self
    }

    pub fn has_baseline(&self) -> bool {
        !self.baseline_file_paths.is_empty()
    }

    fn lookup_baseline<'a>(&'a self, path: &str) -> Option<&'a [u8]> {
        self.baseline_contents
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, c)| c.as_slice())
    }
}

fn perturb_bytes<R: Rand>(rand: &mut R, base: &[u8]) -> Vec<u8> {
    let mut out = base.to_vec();
    match rand.below(nz(4)) {
        0 => {
            if !out.is_empty() {
                let n_flips = 1 + rand.below(nz(4));
                for _ in 0..n_flips {
                    let i = rand.below(nz(out.len()));
                    let mask = 1u8 << rand.below(nz(8));
                    out[i] ^= mask;
                }
            }
        }
        1 => {
            let n = 1 + rand.below(nz(32));
            for _ in 0..n {
                out.push(rand.below(nz(256)) as u8);
            }
        }
        2 => {
            if out.len() > 1 {
                let new_len = rand.below(nz(out.len()));
                out.truncate(new_len);
            }
        }
        _ => {
            let entry = *pick(rand, CONTENT_DICTIONARY);
            let off = if out.is_empty() {
                0
            } else {
                rand.below(nz(out.len() + 1))
            };
            out.splice(off..off, entry.iter().copied());
        }
    }
    out
}

impl Named for UpdateExistingFile {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("UpdateExistingFile");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for UpdateExistingFile
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if self.baseline_file_paths.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let guidance = crate::guidance::peek_live();
        let path = if guidance.has_write() && state.rand_mut().below(nz(100)) < 70 {
            let baseline_writes: Vec<&String> = guidance
                .write_paths
                .iter()
                .filter(|p| self.baseline_file_paths.contains(*p))
                .collect();
            if !baseline_writes.is_empty() {
                pick(state.rand_mut(), &baseline_writes).to_string()
            } else {
                pick(state.rand_mut(), &self.baseline_file_paths).clone()
            }
        } else {
            pick(state.rand_mut(), &self.baseline_file_paths).clone()
        };

        let content = {
            let base = self.lookup_baseline(&path).map(|b| b.to_vec());
            let use_perturb = base.is_some() && state.rand_mut().below(nz(100)) < 50;
            if use_perturb {
                perturb_bytes(state.rand_mut(), base.as_deref().unwrap())
            } else if state.rand_mut().below(nz(100)) < 30 {
                pick(state.rand_mut(), CONTENT_DICTIONARY).to_vec()
            } else {
                random_content(state.rand_mut())
            }
        };

        // update in place to avoid dead ops (last write wins)
        if let Some(existing) = input
            .ops
            .iter_mut()
            .rev()
            .find(|op| op.kind == FsOpKind::UpdateFile && op.path == path)
        {
            existing.content = content;
            existing.size = existing.content.len();
            return Ok(MutationResult::Mutated);
        }

        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }
        input.ops.push(FsOp::update_file(path, content));
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

/// CreateFile for target-written paths not in baseline
pub struct ReplayWriteFile {
    pub guidance: MutationGuidance,
    pub baseline_file_paths: Vec<String>,
}

impl ReplayWriteFile {
    pub fn new(baseline_file_paths: Vec<String>) -> Self {
        Self {
            guidance: MutationGuidance::none(),
            baseline_file_paths,
        }
    }
}

impl Named for ReplayWriteFile {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("ReplayWriteFile");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for ReplayWriteFile
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }

        let guidance = crate::guidance::peek_live();
        let new_paths: Vec<&String> = guidance
            .write_paths
            .iter()
            .filter(|p| !self.baseline_file_paths.contains(*p))
            .collect();

        if new_paths.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let path = pick(state.rand_mut(), &new_paths).to_string();
        let content = if state.rand_mut().below(nz(100)) < 30 {
            pick(state.rand_mut(), CONTENT_DICTIONARY).to_vec()
        } else {
            random_content(state.rand_mut())
        };

        input.ops.push(FsOp::create_file(path, content));
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

/// CmpLog I2S: substitutes known operand pairs into file content
pub struct FsDeltaI2SMutator;

impl FsDeltaI2SMutator {
    pub fn new() -> Self {
        Self
    }
}

impl Named for FsDeltaI2SMutator {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("FsDeltaI2SMutator");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for FsDeltaI2SMutator
where
    S: HasRand + HasMetadata,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        let Ok(meta) = state.metadata::<CmpValuesMetadata>() else {
            return Ok(MutationResult::Skipped);
        };

        // both directions
        let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for val in &meta.list {
            match val {
                CmpValues::U8((a, b, _)) if a != b => {
                    pairs.push((vec![*a], vec![*b]));
                    pairs.push((vec![*b], vec![*a]));
                }
                CmpValues::U16((a, b, _)) if a != b => {
                    pairs.push((a.to_le_bytes().to_vec(), b.to_le_bytes().to_vec()));
                    pairs.push((b.to_le_bytes().to_vec(), a.to_le_bytes().to_vec()));
                }
                CmpValues::U32((a, b, _)) if a != b => {
                    pairs.push((a.to_le_bytes().to_vec(), b.to_le_bytes().to_vec()));
                    pairs.push((b.to_le_bytes().to_vec(), a.to_le_bytes().to_vec()));
                }
                CmpValues::U64((a, b, _)) if a != b => {
                    pairs.push((a.to_le_bytes().to_vec(), b.to_le_bytes().to_vec()));
                    pairs.push((b.to_le_bytes().to_vec(), a.to_le_bytes().to_vec()));
                }
                _ => {}
            }
        }

        if pairs.is_empty() {
            return Ok(MutationResult::Skipped);
        }

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

        let op_idx = *pick(state.rand_mut(), &candidates);
        let (lhs, rhs) = pick(state.rand_mut(), &pairs).clone();

        let content_len = input.ops[op_idx].content.len();
        if lhs.is_empty() || lhs.len() > content_len {
            return Ok(MutationResult::Skipped);
        }

        let search_end = content_len - lhs.len() + 1;
        let start = if search_end > 1 {
            state.rand_mut().below(nz(search_end))
        } else {
            0
        };

        let pos = (start..search_end)
            .chain(0..start)
            .find(|&i| input.ops[op_idx].content[i..i + lhs.len()] == lhs[..]);

        let Some(pos) = pos else {
            return Ok(MutationResult::Skipped);
        };

        let end = pos + lhs.len();
        let mut new_content = Vec::with_capacity(content_len - lhs.len() + rhs.len());
        new_content.extend_from_slice(&input.ops[op_idx].content[..pos]);
        new_content.extend_from_slice(&rhs);
        new_content.extend_from_slice(&input.ops[op_idx].content[end..]);
        input.ops[op_idx].content = new_content;
        input.ops[op_idx].size = input.ops[op_idx].content.len();

        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::{FsDelta, FsOp};
    use crate::guidance::{update_live, MutationGuidance};
    use libafl::mutators::MutationResult;
    use libafl::state::HasRand;
    use libafl_bolts::rands::StdRand;

    // guidance tests touch a global; serialize them
    static GUIDANCE_TEST_MU: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct TestState {
        rand: StdRand,
    }

    impl TestState {
        fn new() -> Self {
            Self {
                rand: StdRand::with_seed(0xdeadbeef_cafebabe),
            }
        }
    }

    impl HasRand for TestState {
        type Rand = StdRand;
        fn rand(&self) -> &StdRand {
            &self.rand
        }
        fn rand_mut(&mut self) -> &mut StdRand {
            &mut self.rand
        }
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

    #[test]
    fn byte_flip_mutates_content() {
        let mut state = TestState::new();
        let mut m = ByteFlipFileContent::new();
        let mut delta = file_delta();
        let original = delta.ops[0].content.clone();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert_ne!(delta.ops[0].content, original);
    }

    #[test]
    fn byte_flip_skips_when_no_file_ops() {
        let mut state = TestState::new();
        let mut m = ByteFlipFileContent::new();
        let mut delta = FsDelta::new(vec![FsOp::mkdir("/empty")]);
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
    }

    #[test]
    fn replace_file_content_changes_content_and_size() {
        let mut state = TestState::new();
        let mut m = ReplaceFileContent::new();
        let mut delta = file_delta();
        let original_content = delta.ops[0].content.clone();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert_ne!(delta.ops[0].content, original_content);
        // size must stay in sync with content len
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

    #[test]
    fn add_file_op_grows_delta() {
        let mut state = TestState::new();
        let mut m = AddFileOp::new();
        let mut delta = file_delta();
        let before = delta.ops.len();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert_eq!(delta.ops.len(), before + 1);
        let new_op = delta.ops.last().unwrap();
        assert!(
            matches!(new_op.kind, FsOpKind::CreateFile | FsOpKind::Mkdir),
            "unexpected kind: {:?}",
            new_op.kind
        );
        assert!(new_op.path.starts_with('/'));
    }

    #[test]
    fn add_file_op_uses_guidance_enoent_paths() {
        let _guard = GUIDANCE_TEST_MU.lock().unwrap();
        let mut g = MutationGuidance::none();
        g.enoent_paths = vec!["/guided/path".to_string()];
        update_live(g);

        let mut state = TestState::new();
        let mut m = AddFileOp::new();
        let delta = file_delta();

        let mut used_guided = false;
        for _ in 0..50 {
            let mut d = delta.clone();
            m.mutate(&mut state, &mut d).unwrap();
            if d.ops.last().unwrap().path == "/guided/path" {
                used_guided = true;
                break;
            }
        }
        update_live(MutationGuidance::none());
        assert!(used_guided, "guided path was never chosen in 50 tries");
    }

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

    #[test]
    fn mutate_path_changes_a_component() {
        let mut state = TestState::new();
        let mut m = MutatePath::new();
        let original_path = "/input".to_string();
        let mut changed = false;
        for _ in 0..20 {
            let mut delta = file_delta();
            m.mutate(&mut state, &mut delta).unwrap();
            if delta.ops[0].path != original_path {
                changed = true;
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

    #[test]
    fn splice_delta_appends_ops_from_donor() {
        let mut state = TestState::new();
        let donor = FsDelta::new(vec![
            FsOp::mkdir("/splice_a"),
            FsOp::create_file("/splice_b", b"data".to_vec()),
        ]);
        let mut m = SpliceDelta::new_fixed(vec![donor]);
        let mut delta = file_delta();
        let before = delta.ops.len();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert!(
            delta.ops.len() > before,
            "splice should append at least 1 op"
        );
    }

    #[test]
    fn splice_delta_skips_empty_pool() {
        let mut state = TestState::new();
        let mut m = SpliceDelta::new_fixed(vec![]);
        let mut delta = file_delta();
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
    }

    #[test]
    fn splice_delta_sees_live_corpus_updates() {
        // empty pool skips; after pushing a donor, mutate sees it
        let mut state = TestState::new();
        let pool: LiveCorpus = Rc::new(RefCell::new(vec![]));
        let mut m = SpliceDelta::new(pool.clone());

        let mut delta = file_delta();
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped, "empty pool must skip");

        pool.borrow_mut()
            .push(FsDelta::new(vec![FsOp::mkdir("/live")]));
        let mut delta = file_delta();
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(
            res,
            MutationResult::Mutated,
            "mutator must pick up live pool update"
        );
    }

    #[test]
    fn destructive_mutator_grows_delta() {
        let mut state = TestState::new();
        let mut m = DestructiveMutator::new();
        let mut delta = file_delta();
        let before = delta.ops.len();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert_eq!(delta.ops.len(), before + 1);

        let new_op = delta.ops.last().unwrap();
        assert!(
            matches!(
                new_op.kind,
                FsOpKind::DeleteFile | FsOpKind::Rmdir | FsOpKind::Truncate | FsOpKind::SetTimes
            ),
            "unexpected kind: {:?}",
            new_op.kind
        );
        assert!(new_op.path.starts_with('/'));
    }

    #[test]
    fn destructive_mutator_generates_all_four_kinds() {
        let mut state = TestState::new();
        let mut m = DestructiveMutator::new();
        let mut seen = std::collections::HashSet::new();

        for _ in 0..200 {
            let mut delta = file_delta();
            m.mutate(&mut state, &mut delta).unwrap();
            seen.insert(std::mem::discriminant(&delta.ops.last().unwrap().kind));
        }
        assert_eq!(seen.len(), 4, "not all destructive kinds were generated");
    }

    #[test]
    fn add_file_op_skips_at_max_ops() {
        let mut state = TestState::new();
        let mut m = AddFileOp::new();
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
        let mut m = SpliceDelta::new_fixed(vec![donor]);
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

    #[test]
    fn destructive_mutator_uses_baseline_paths() {
        let mut state = TestState::new();
        let file_paths = vec!["/input".to_string(), "/etc/config".to_string()];
        let dir_paths = vec!["/etc".to_string()];
        let all_paths = vec![
            "/input".to_string(),
            "/etc/config".to_string(),
            "/etc".to_string(),
        ];
        let mut m = DestructiveMutator::with_baseline(
            file_paths.clone(),
            dir_paths.clone(),
            all_paths.clone(),
        );

        let mut used_baseline = false;
        for _ in 0..50 {
            let mut delta = file_delta();
            m.mutate(&mut state, &mut delta).unwrap();
            let new_op = delta.ops.last().unwrap();
            if all_paths.contains(&new_op.path) {
                used_baseline = true;
                break;
            }
        }
        assert!(used_baseline, "baseline path was never chosen in 50 tries");
    }

    #[test]
    fn destructive_mutator_truncate_targets_file_paths() {
        let mut state = TestState::new();
        let file_paths = vec!["/input".to_string()];
        let dir_paths = vec!["/etc".to_string()];
        let all_paths = vec!["/input".to_string(), "/etc".to_string()];
        let mut m = DestructiveMutator::with_baseline(file_paths.clone(), dir_paths, all_paths);

        let mut seen_truncate = false;
        for _ in 0..200 {
            let mut delta = file_delta();
            m.mutate(&mut state, &mut delta).unwrap();
            let new_op = delta.ops.last().unwrap();
            if matches!(new_op.kind, FsOpKind::Truncate) {
                seen_truncate = true;
                assert!(new_op.path.starts_with('/'));
            }
        }
        assert!(seen_truncate, "Truncate kind never generated in 200 tries");
    }

    #[test]
    fn mutate_path_whole_swap_uses_baseline_path() {
        let mut state = TestState::new();
        let baseline = vec!["/etc/config".to_string(), "/input".to_string()];
        let mut m = MutatePath::with_baseline(vec![], vec![], baseline.clone());

        let mut used_baseline = false;
        for _ in 0..50 {
            let mut delta = FsDelta::new(vec![FsOp::mkdir("/random/path")]);
            m.mutate(&mut state, &mut delta).unwrap();
            if baseline.contains(&delta.ops[0].path) {
                used_baseline = true;
                break;
            }
        }
        assert!(
            used_baseline,
            "whole-path swap never used a baseline path in 50 tries"
        );
    }

    #[test]
    fn update_existing_file_appends_update_op() {
        let mut state = TestState::new();
        let baseline_files = vec!["/input".to_string(), "/etc/config".to_string()];
        let mut m = UpdateExistingFile::new(baseline_files.clone());
        let mut delta = file_delta();
        let before = delta.ops.len();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert_eq!(delta.ops.len(), before + 1);

        let new_op = delta.ops.last().unwrap();
        assert_eq!(new_op.kind, FsOpKind::UpdateFile, "should be UpdateFile");
        assert!(
            baseline_files.contains(&new_op.path),
            "path should come from baseline"
        );
        // content may be an empty dict entry; only size/content invariant matters
        assert_eq!(new_op.size, new_op.content.len(), "size/content must match");
    }

    #[test]
    fn update_existing_file_skips_when_no_baseline() {
        let mut state = TestState::new();
        let mut m = UpdateExistingFile::new(vec![]);
        let mut delta = file_delta();
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
        assert_eq!(delta.ops.len(), 1, "delta must not grow");
    }

    #[test]
    fn update_existing_file_skips_at_max_ops() {
        let mut state = TestState::new();
        let mut m = UpdateExistingFile::new(vec!["/input".to_string()]);
        let ops = (0..MAX_OPS)
            .map(|i| FsOp::create_file(format!("/f{i}"), vec![i as u8]))
            .collect();
        let mut delta = FsDelta::new(ops);

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
        assert_eq!(delta.ops.len(), MAX_OPS);
    }

    #[test]
    fn update_existing_file_perturbs_baseline_content() {
        // perturb branch should yield a same-length-but-changed content (bit-flip)
        let mut state = TestState::new();
        let base = b"seed".to_vec();
        let contents = vec![("/input".to_string(), base.clone())];
        let mut m =
            UpdateExistingFile::new(vec!["/input".to_string()]).with_baseline_contents(contents);

        let mut found_small_perturb = false;
        for _ in 0..200 {
            let mut delta = FsDelta::new(vec![FsOp::mkdir("/seed_dir")]);
            m.mutate(&mut state, &mut delta).unwrap();
            let new_op = delta.ops.last().unwrap();
            if new_op.kind == FsOpKind::UpdateFile
                && new_op.content.len() == base.len()
                && new_op.content != base
            {
                found_small_perturb = true;
                break;
            }
        }
        assert!(
            found_small_perturb,
            "no bit-flip-style perturbation seen in 200 tries"
        );
    }

    #[test]
    fn perturb_bytes_handles_empty_base() {
        // empty base must not panic (below() on zero len would)
        let mut rand = StdRand::with_seed(0x1234);
        for _ in 0..20 {
            let _ = perturb_bytes(&mut rand, &[]);
        }
    }

    #[test]
    fn replace_file_content_uses_dictionary_sometimes() {
        // confirms the 40% dictionary branch fires
        let mut state = TestState::new();
        let mut m = ReplaceFileContent::new();

        let mut used_dict = false;
        for _ in 0..200 {
            let mut delta = file_delta();
            m.mutate(&mut state, &mut delta).unwrap();
            let c = &delta.ops[0].content;
            if CONTENT_DICTIONARY.iter().any(|e| *e == c.as_slice()) {
                used_dict = true;
                break;
            }
        }
        assert!(
            used_dict,
            "ReplaceFileContent never picked a dictionary entry in 200 tries"
        );
    }

    #[test]
    fn mutate_path_whole_swap_prefers_enoent_paths() {
        let _guard = GUIDANCE_TEST_MU.lock().unwrap();
        let mut g = MutationGuidance::none();
        g.enoent_paths = vec!["/wanted/by/target".to_string()];
        update_live(g);

        let mut state = TestState::new();
        let mut m = MutatePath::with_baseline(vec![], vec![], vec!["/etc/config".to_string()]);

        let mut used_enoent = false;
        for _ in 0..100 {
            let mut delta = FsDelta::new(vec![FsOp::mkdir("/start")]);
            m.mutate(&mut state, &mut delta).unwrap();
            if delta.ops[0].path == "/wanted/by/target" {
                used_enoent = true;
                break;
            }
        }
        update_live(MutationGuidance::none());
        assert!(
            used_enoent,
            "guidance.enoent_paths never used in whole-swap across 100 tries"
        );
    }

    #[test]
    fn destructive_mutator_delete_prefers_recreate_paths() {
        let _guard = GUIDANCE_TEST_MU.lock().unwrap();
        let mut g = MutationGuidance::none();
        g.recreate_paths = vec!["/deleted/by/target".to_string()];
        update_live(g);

        let mut state = TestState::new();
        let mut m = DestructiveMutator::with_baseline(
            vec!["/input".to_string()],
            vec!["/etc".to_string()],
            vec!["/input".to_string(), "/etc".to_string()],
        );

        let mut used_recreate = false;
        for _ in 0..200 {
            let mut delta = file_delta();
            m.mutate(&mut state, &mut delta).unwrap();
            let new_op = delta.ops.last().unwrap();
            if matches!(new_op.kind, FsOpKind::DeleteFile | FsOpKind::Rmdir)
                && new_op.path == "/deleted/by/target"
            {
                used_recreate = true;
                break;
            }
        }
        update_live(MutationGuidance::none());
        assert!(
            used_recreate,
            "recreate_paths never used by destructive in 200 tries"
        );
    }

    #[test]
    fn update_existing_file_prefers_write_paths() {
        let _guard = GUIDANCE_TEST_MU.lock().unwrap();
        let mut g = MutationGuidance::none();
        g.write_paths = vec!["/input".to_string()];
        update_live(g);

        let mut state = TestState::new();
        let mut m = UpdateExistingFile::new(vec!["/input".to_string(), "/etc/config".to_string()]);

        let mut used_write = false;
        for _ in 0..100 {
            let mut delta = FsDelta::new(vec![FsOp::mkdir("/seed")]);
            m.mutate(&mut state, &mut delta).unwrap();
            let new_op = delta.ops.last().unwrap();
            if new_op.kind == FsOpKind::UpdateFile && new_op.path == "/input" {
                used_write = true;
                break;
            }
        }
        update_live(MutationGuidance::none());
        assert!(used_write, "guidance.write_paths (baseline-intersecting) never preferred by UpdateExistingFile in 100 tries");
    }

    #[test]
    fn mutate_path_whole_swap_prefers_write_paths_over_recreate() {
        let _guard = GUIDANCE_TEST_MU.lock().unwrap();
        let mut g = MutationGuidance::none();
        g.write_paths = vec!["/written/path".to_string()];
        g.recreate_paths = vec!["/recreate/path".to_string()];
        update_live(g);

        let mut state = TestState::new();
        let mut m = MutatePath::with_baseline(
            vec!["/input".to_string()],
            vec![],
            vec!["/input".to_string()],
        );

        let mut used_write = false;
        for _ in 0..200 {
            let mut delta = FsDelta::new(vec![FsOp::mkdir("/random/path")]);
            m.mutate(&mut state, &mut delta).unwrap();
            if delta.ops[0].path == "/written/path" {
                used_write = true;
                break;
            }
        }
        update_live(MutationGuidance::none());
        assert!(
            used_write,
            "write_paths never preferred over recreate_paths in whole-swap"
        );
    }

    #[test]
    fn replay_write_file_skips_with_no_guidance() {
        let _guard = GUIDANCE_TEST_MU.lock().unwrap();
        update_live(MutationGuidance::none());
        let mut state = TestState::new();
        let mut m = ReplayWriteFile::new(vec!["/input".to_string()]);
        let mut delta = file_delta();
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped);
    }

    #[test]
    fn replay_write_file_skips_when_all_write_paths_in_baseline() {
        let _guard = GUIDANCE_TEST_MU.lock().unwrap();
        let baseline = vec!["/input".to_string(), "/etc/config".to_string()];
        let mut g = MutationGuidance::none();
        g.write_paths = baseline.clone(); // complement is empty
        update_live(g);
        let mut state = TestState::new();
        let mut m = ReplayWriteFile::new(baseline);
        let mut delta = file_delta();
        let res = m.mutate(&mut state, &mut delta).unwrap();
        update_live(MutationGuidance::none());
        assert_eq!(res, MutationResult::Skipped);
    }

    #[test]
    fn replay_write_file_creates_non_baseline_path() {
        let _guard = GUIDANCE_TEST_MU.lock().unwrap();
        let mut g = MutationGuidance::none();
        g.write_paths = vec!["/target/created/this".to_string()];
        update_live(g);

        let mut state = TestState::new();
        let mut m = ReplayWriteFile::new(vec!["/input".to_string()]);
        let mut delta = file_delta();
        let before = delta.ops.len();
        let res = m.mutate(&mut state, &mut delta).unwrap();
        update_live(MutationGuidance::none());
        assert_eq!(res, MutationResult::Mutated);
        assert_eq!(delta.ops.len(), before + 1);
        let new_op = delta.ops.last().unwrap();
        assert_eq!(new_op.kind, FsOpKind::CreateFile);
        assert_eq!(new_op.path, "/target/created/this");
    }

    #[test]
    fn replay_write_file_ignores_baseline_write_paths() {
        let _guard = GUIDANCE_TEST_MU.lock().unwrap();
        let mut g = MutationGuidance::none();
        g.write_paths = vec![
            "/input".to_string(),         // in baseline, ignored
            "/target/output".to_string(), // only valid pick
        ];
        update_live(g);

        let mut state = TestState::new();
        let mut m = ReplayWriteFile::new(vec!["/input".to_string()]);
        for _ in 0..100 {
            let mut delta = file_delta();
            m.mutate(&mut state, &mut delta).unwrap();
            let new_op = delta.ops.last().unwrap();
            assert_eq!(
                new_op.path, "/target/output",
                "ReplayWriteFile picked a baseline path — should be filtered out"
            );
        }
        update_live(MutationGuidance::none());
    }
}
