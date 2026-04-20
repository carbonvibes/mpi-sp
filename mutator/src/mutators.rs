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
use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::rc::Rc;

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

/// Cap on the live corpus size.  Once reached, novel deltas evict a random
/// non-seed entry rather than letting the corpus grow unboundedly.
pub const MAX_LIVE_CORPUS: usize = 128;

/// Shared, mutable corpus reference.  The harness holds one `LiveCorpus` and
/// passes `Rc::clone(...)` to every mutator that needs to read from or push
/// to it (currently only `SpliceDelta`).
///
/// Interior mutability via `RefCell` is required because the mutator API
/// takes `&mut self` but the harness also needs to push promoted deltas
/// between mutator calls.  Single-threaded use only (Phase A harness is
/// single-threaded; Phase B's LibAFL harness will migrate to `Arc<Mutex<_>>`
/// if needed).
pub type LiveCorpus = Rc<RefCell<Vec<crate::delta::FsDelta>>>;

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────

/// A small vocabulary of valid path components.
static PATH_COMPONENTS: &[&str] = &[
    "a", "b", "c", "d",
    "etc", "tmp", "var", "lib", "usr",
    "input", "output", "config", "data", "test", "run",
];

/// Dictionary of structurally interesting content values.
///
/// Covers: trigger strings the Week 6 demo target will crash on, magic bytes
/// for common file formats, boundary / overflow markers, and path-shaped
/// strings that sometimes confuse parsers.  `ReplaceFileContent` draws from
/// this pool with 40% probability; the other 60% fall back to random bytes so
/// the mutator still explores unstructured space.
static CONTENT_DICTIONARY: &[&[u8]] = &[
    b"foobar",                               // Week 6 demo crash trigger
    b"FOOBAR",
    b"",                                     // empty content
    b"\x7fELF",                              // ELF magic
    b"#!/bin/sh\n",                          // shell shebang
    b"[settings]\nverbose=1\ndebug=1\n",     // realistic config file
    b"\x00\x00\x00\x00",                     // 4 zero bytes
    b"\xff\xff\xff\xff",                     // all-ones
    b"../../../etc/passwd",                  // path traversal
    b"/dev/null",                            // special path
    b"%s%s%s%s",                             // format string
    b"A",                                    // single byte
    &[0xAA; 64],                             // 64 bytes alternating pattern
    &[0x00; 256],                            // 256 zero bytes (boundary size)
    &[0x41; 4096],                           // 4KB of 'A' (page-size content)
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

/// Pick from `baseline` with `bias_pct`% probability; fall back to a random path.
/// When `baseline` is empty, always returns a random path.
fn pick_or_random<R: Rand>(rand: &mut R, baseline: &[String], bias_pct: usize) -> String {
    if !baseline.is_empty() && rand.below(nz(100)) < bias_pct {
        pick(rand, baseline).clone()
    } else {
        random_path(rand)
    }
}

/// Pick a timestamp from a set of interesting edge-case values (40%) or a
/// random UNIX timestamp (60%).
///
/// Edge cases: epoch, pre-epoch (-1), 2038 i32::MAX boundary, a recent
/// timestamp (~2023), and zero nanoseconds vs a large nanosecond value.
fn pick_timestamp<R: Rand>(rand: &mut R) -> i64 {
    const INTERESTING: &[i64] = &[
        0,                   // epoch
        -1,                  // pre-epoch
        i32::MAX as i64,     // 2038 overflow boundary
        2_000_000_000,       // ~2033, post-2038 far future
        1_700_000_000,       // ~Nov 2023, current era
    ];
    if rand.below(nz(100)) < 40 {
        *pick(rand, INTERESTING)
    } else {
        rand.below(nz(u32::MAX as usize)) as i64
    }
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
///
/// With 40% probability draws a value from `CONTENT_DICTIONARY` (trigger
/// strings, magic bytes, boundary sizes) instead of random bytes.  Dictionary
/// entries are short structured values that have a much higher chance of
/// reaching parser error paths than uniform random input.
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

        // 40% dictionary draw, 60% random bytes.  The dictionary contains
        // short structured values (trigger strings, magic numbers, boundary
        // sizes) that are far more likely to reach parser error paths than
        // uniform random bytes of similar length.
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

        let using_guided = self.guidance.has_enoent()
            && state.rand_mut().below(nz(100)) < 70;

        let path = if using_guided {
            pick(state.rand_mut(), &self.guidance.enoent_paths).clone()
        } else {
            random_path(state.rand_mut())
        };

        // When using a guided ENOENT path the target tried to *open* a file
        // there — bias strongly toward CreateFile (90 %) rather than Mkdir.
        // For random paths, keep the default 70/30 split.
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

/// Mutate the path of a randomly chosen op.
///
/// Two modes, chosen at random:
/// - **Whole-path swap** (30 % when a target pool is non-empty): replace
///   the entire path with a known-interesting path — prefers
///   `guidance.enoent_paths` (paths the target tried to open) when populated,
///   otherwise falls back to `baseline_paths`.  Useful for turning a failing
///   op (random path that doesn't exist) into one the target actually reads.
/// - **Component swap** (otherwise): replace one component with a word from
///   `PATH_COMPONENTS`.  Explores the neighbourhood of the current path.
pub struct MutatePath {
    pub guidance:       MutationGuidance,
    /// All VFS paths at baseline.  When non-empty, 30 % of mutations replace
    /// the whole path with a baseline path rather than swapping a component.
    pub baseline_paths: Vec<String>,
}

impl MutatePath {
    pub fn new() -> Self {
        Self { guidance: MutationGuidance::none(), baseline_paths: vec![] }
    }

    pub fn with_baseline(paths: Vec<String>) -> Self {
        Self { guidance: MutationGuidance::none(), baseline_paths: paths }
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
        let op     = &mut input.ops[op_idx];

        // Whole-path swap: redirect the op to a known-interesting path.
        // Preference order when the swap roll hits:
        //   1. guidance.enoent_paths (highest signal — target wanted these)
        //   2. guidance.recreate_paths (target acts on these)
        //   3. baseline_paths (known to exist)
        let have_swap_target = self.guidance.has_enoent()
            || self.guidance.has_recreate()
            || !self.baseline_paths.is_empty();
        if have_swap_target && state.rand_mut().below(nz(100)) < 30 {
            let pool: &[String] = if self.guidance.has_enoent() {
                &self.guidance.enoent_paths
            } else if self.guidance.has_recreate() {
                &self.guidance.recreate_paths
            } else {
                &self.baseline_paths
            };
            op.path = pick(state.rand_mut(), pool).clone();
            return Ok(MutationResult::Mutated);
        }

        // Component swap: replace one segment with a PATH_COMPONENTS word.
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

/// Append a random contiguous slice of ops from a donor delta.
///
/// Unlike a strict prefix splice, this picks a random start offset so that
/// late-donor ops (e.g. metadata ops at the end of a sequence) can be spliced
/// independently of the earlier ops they follow.  Since `cp_ensure_parents`
/// handles missing parent directories, any slice is structurally safe.
///
/// The donor pool is a `LiveCorpus` shared with the harness — when the harness
/// promotes a mutated delta (novel semantic yield) it pushes into this pool
/// and subsequent `SpliceDelta` calls immediately see the new donor.
pub struct SpliceDelta {
    pub guidance:    MutationGuidance,
    pub corpus_pool: LiveCorpus,
}

impl SpliceDelta {
    /// Construct with a shared live corpus.
    pub fn new(pool: LiveCorpus) -> Self {
        Self {
            guidance: MutationGuidance::none(),
            corpus_pool: pool,
        }
    }

    /// Construct with a one-shot fixed pool (test/legacy helper).  The pool
    /// is wrapped in a fresh `Rc<RefCell<_>>` so the struct type still matches.
    pub fn new_fixed(pool: Vec<FsDelta>) -> Self {
        Self {
            guidance: MutationGuidance::none(),
            corpus_pool: Rc::new(RefCell::new(pool)),
        }
    }

    /// Number of donors currently available.
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
        let donor     = &pool[donor_idx];

        if donor.ops.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let available = MAX_OPS.saturating_sub(input.ops.len());
        if available == 0 {
            return Ok(MutationResult::Skipped);
        }

        // Pick a random start offset instead of always beginning at 0.
        // cp_ensure_parents makes any slice structurally safe even when
        // it skips an earlier Mkdir that creates the parent directory.
        let start       = state.rand_mut().below(nz(donor.ops.len()));
        let donor_slice = &donor.ops[start..];

        if donor_slice.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let max_take = donor_slice.len().min(available);
        let take_n   = 1 + state.rand_mut().below(nz(max_take));
        for op in donor_slice.iter().take(take_n) {
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
/// SetTimes.  These are the four op kinds that the other stages never
/// generate as primary ops, ensuring all seven FsOpKind variants are
/// reachable through the mutation pipeline.
///
/// Op-type-aware path selection (70 % baseline bias when lists are populated):
///
/// | Op | Path drawn from |
/// |---|---|
/// | `DeleteFile` | `baseline_file_paths` (files only) |
/// | `Rmdir` | `baseline_dir_paths` (dirs only) |
/// | `Truncate` | `baseline_file_paths` (files only) |
/// | `SetTimes` | `baseline_all_paths` (any path) |
///
/// `SetTimes` timestamps are drawn from a set of interesting edge-case values
/// (epoch, pre-epoch, 2038 boundary, far future) 40 % of the time.
pub struct DestructiveMutator {
    pub guidance:            MutationGuidance,
    pub baseline_file_paths: Vec<String>,  // for DeleteFile, Truncate
    pub baseline_dir_paths:  Vec<String>,  // for Rmdir
    pub baseline_all_paths:  Vec<String>,  // for SetTimes
}

impl DestructiveMutator {
    pub fn new() -> Self {
        Self {
            guidance:            MutationGuidance::none(),
            baseline_file_paths: vec![],
            baseline_dir_paths:  vec![],
            baseline_all_paths:  vec![],
        }
    }

    /// Construct with pre-enumerated baseline path lists for op-type-aware
    /// path selection.
    pub fn with_baseline(
        file_paths: Vec<String>,
        dir_paths:  Vec<String>,
        all_paths:  Vec<String>,
    ) -> Self {
        Self {
            guidance:            MutationGuidance::none(),
            baseline_file_paths: file_paths,
            baseline_dir_paths:  dir_paths,
            baseline_all_paths:  all_paths,
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

        let op = match state.rand_mut().below(nz(4)) {
            0 => {
                // DeleteFile: prefer guidance.recreate_paths (the target has
                // shown it acts on these — re-deleting them may exercise the
                // same code path again) → then baseline files → random.
                let path = if self.guidance.has_recreate()
                    && state.rand_mut().below(nz(100)) < 50
                {
                    pick(state.rand_mut(), &self.guidance.recreate_paths).clone()
                } else {
                    pick_or_random(state.rand_mut(), &self.baseline_file_paths, 70)
                };
                FsOp::delete_file(path)
            }
            1 => {
                // Rmdir: same idea as DeleteFile but on directory paths.
                let path = if self.guidance.has_recreate()
                    && state.rand_mut().below(nz(100)) < 50
                {
                    pick(state.rand_mut(), &self.guidance.recreate_paths).clone()
                } else {
                    pick_or_random(state.rand_mut(), &self.baseline_dir_paths, 70)
                };
                FsOp::rmdir(path)
            }
            2 => {
                // Truncate: only meaningful on files.
                let path     = pick_or_random(state.rand_mut(), &self.baseline_file_paths, 70);
                let new_size = state.rand_mut().below(nz(1024));
                FsOp::truncate(path, new_size)
            }
            _ => {
                // SetTimes: any path (files and dirs can both have timestamps).
                // Use interesting edge-case timestamps 40 % of the time.
                let path       = pick_or_random(state.rand_mut(), &self.baseline_all_paths, 70);
                let mtime_sec  = pick_timestamp(state.rand_mut());
                let atime_sec  = pick_timestamp(state.rand_mut());
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

// ─────────────────────────────────────────────────────────────────────────────
// 8. UpdateExistingFile
// ─────────────────────────────────────────────────────────────────────────────

/// Append an UpdateFile op targeting a file that is known to exist in the
/// baseline VFS.
///
/// When `baseline_contents` is populated the mutator 50% of the time starts
/// from the real content of the chosen file and applies a small perturbation
/// (bit-flip, append, truncate, or dictionary splice), rather than emitting
/// 1–64 random bytes.  This matters for targets that read structured content
/// (e.g. `/etc/config`) — random-byte UpdateFile ops will short-circuit on the
/// first parse error and never exercise deeper parser state.
///
/// The other 50% (and always when `baseline_contents` is empty) produces
/// either a dictionary value or a random byte string, matching the behaviour
/// of `ReplaceFileContent`.
///
/// Skips when `baseline_file_paths` is empty (graceful no-op if the harness
/// did not populate it) or when the delta is already at `MAX_OPS`.
pub struct UpdateExistingFile {
    pub guidance:            MutationGuidance,
    /// Paths of regular files in the baseline VFS.  Populated by the harness
    /// at startup via `enumerate_vfs_file_paths`.
    pub baseline_file_paths: Vec<String>,
    /// Optional (path → content) pairs for baseline files.  When a chosen
    /// path has an entry here, the mutator can produce a small perturbation
    /// of the real content rather than random bytes.  Empty = always fall
    /// back to dictionary/random content.
    pub baseline_contents:   Vec<(String, Vec<u8>)>,
}

impl UpdateExistingFile {
    pub fn new(file_paths: Vec<String>) -> Self {
        Self {
            guidance:            MutationGuidance::none(),
            baseline_file_paths: file_paths,
            baseline_contents:   Vec::new(),
        }
    }

    /// Attach baseline (path, content) pairs for real-content perturbation.
    /// Only the paths also present in `baseline_file_paths` will ever be used.
    pub fn with_baseline_contents(mut self, contents: Vec<(String, Vec<u8>)>) -> Self {
        self.baseline_contents = contents;
        self
    }

    /// True when the mutator can produce anything (path list non-empty).
    pub fn has_baseline(&self) -> bool {
        !self.baseline_file_paths.is_empty()
    }

    /// Lookup the baseline content for a path; returns None when the path
    /// was not populated at startup.
    fn lookup_baseline<'a>(&'a self, path: &str) -> Option<&'a [u8]> {
        self.baseline_contents
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, c)| c.as_slice())
    }
}

/// Perturb a byte slice: bit-flip, append, truncate, or dictionary-splice.
///
/// Used by `UpdateExistingFile` to produce structurally-similar variants of
/// a real file content.  Preserves most of the structure, which is what lets
/// parsers reach deeper state than a fully random replacement would.
fn perturb_bytes<R: Rand>(rand: &mut R, base: &[u8]) -> Vec<u8> {
    let mut out = base.to_vec();
    match rand.below(nz(4)) {
        0 => {
            // Flip 1–4 random bits (graceful no-op on empty base).
            if !out.is_empty() {
                let n_flips = 1 + rand.below(nz(4));
                for _ in 0..n_flips {
                    let i    = rand.below(nz(out.len()));
                    let mask = 1u8 << rand.below(nz(8));
                    out[i] ^= mask;
                }
            }
        }
        1 => {
            // Append 1–32 random bytes.
            let n = 1 + rand.below(nz(32));
            for _ in 0..n { out.push(rand.below(nz(256)) as u8); }
        }
        2 => {
            // Truncate to a shorter length (at least 1 byte, or empty if base was empty).
            if out.len() > 1 {
                let new_len = rand.below(nz(out.len()));
                out.truncate(new_len);
            }
        }
        _ => {
            // Splice a dictionary entry into a random offset.
            let entry = *pick(rand, CONTENT_DICTIONARY);
            let off   = if out.is_empty() { 0 } else { rand.below(nz(out.len() + 1)) };
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
        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }
        if self.baseline_file_paths.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let path = pick(state.rand_mut(), &self.baseline_file_paths).clone();

        // Content selection strategy:
        //  1. If we have live baseline content for this path AND the coin
        //     lands (50%), perturb the real content — structured mutation.
        //  2. Else, 30% chance to draw from the dictionary (trigger strings,
        //     magic values).
        //  3. Else, random bytes — unstructured exploration.
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

        input.ops.push(FsOp::update_file(path, content));
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
        let mut m = SpliceDelta::new_fixed(vec![donor]);
        let mut delta = file_delta();
        let before = delta.ops.len();

        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated);
        assert!(delta.ops.len() > before, "splice should append at least 1 op");
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
        // Shared pool starts empty → skips.  After a donor is pushed to the
        // shared Rc<RefCell<>> pool, subsequent mutate calls see it.
        let mut state = TestState::new();
        let pool: LiveCorpus = Rc::new(RefCell::new(vec![]));
        let mut m = SpliceDelta::new(pool.clone());

        let mut delta = file_delta();
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Skipped, "empty pool must skip");

        pool.borrow_mut().push(FsDelta::new(vec![FsOp::mkdir("/live")]));
        let mut delta = file_delta();
        let res = m.mutate(&mut state, &mut delta).unwrap();
        assert_eq!(res, MutationResult::Mutated, "mutator must pick up live pool update");
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

    // ── DestructiveMutator baseline bias ─────────────────────────────────

    #[test]
    fn destructive_mutator_uses_baseline_paths() {
        let mut state = TestState::new();
        let file_paths = vec!["/input".to_string(), "/etc/config".to_string()];
        let dir_paths  = vec!["/etc".to_string()];
        let all_paths  = vec!["/input".to_string(), "/etc/config".to_string(), "/etc".to_string()];
        let mut m = DestructiveMutator::with_baseline(
            file_paths.clone(),
            dir_paths.clone(),
            all_paths.clone(),
        );

        // Run many iterations; at least one must use a baseline path (70 % bias).
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
        let dir_paths  = vec!["/etc".to_string()];
        let all_paths  = vec!["/input".to_string(), "/etc".to_string()];
        let mut m = DestructiveMutator::with_baseline(file_paths.clone(), dir_paths, all_paths);

        // Run many times; when a Truncate is produced it must use a file path.
        let mut seen_truncate = false;
        for _ in 0..200 {
            let mut delta = file_delta();
            m.mutate(&mut state, &mut delta).unwrap();
            let new_op = delta.ops.last().unwrap();
            if matches!(new_op.kind, FsOpKind::Truncate) {
                seen_truncate = true;
                // With 70 % bias and only one file path, must hit it eventually.
                // We just check the kind here; path bias is tested probabilistically above.
                assert!(new_op.path.starts_with('/'));
            }
        }
        assert!(seen_truncate, "Truncate kind never generated in 200 tries");
    }

    // ── MutatePath with baseline ──────────────────────────────────────────

    #[test]
    fn mutate_path_whole_swap_uses_baseline_path() {
        let mut state = TestState::new();
        let baseline = vec!["/etc/config".to_string(), "/input".to_string()];
        let mut m = MutatePath::with_baseline(baseline.clone());

        // 30 % whole-path swap; 50 runs should hit it at least once.
        let mut used_baseline = false;
        for _ in 0..50 {
            let mut delta = FsDelta::new(vec![FsOp::mkdir("/random/path")]);
            m.mutate(&mut state, &mut delta).unwrap();
            if baseline.contains(&delta.ops[0].path) {
                used_baseline = true;
                break;
            }
        }
        assert!(used_baseline, "whole-path swap never used a baseline path in 50 tries");
    }

    // ── 8. UpdateExistingFile ─────────────────────────────────────────────

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
        assert!(baseline_files.contains(&new_op.path), "path should come from baseline");
        // Content may occasionally be an empty dictionary entry — that is a
        // valid UpdateFile.  Only the size/content invariant must hold.
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

    // ── Real-content perturbation ─────────────────────────────────────────

    #[test]
    fn update_existing_file_perturbs_baseline_content() {
        // With the perturb branch active (baseline_contents populated), at
        // least one call out of 100 should produce content that is a small
        // perturbation of the baseline — detect this by checking for a
        // bit-flip: hamming distance ≤ 16 but non-zero on a 4-byte base.
        let mut state = TestState::new();
        let base = b"seed".to_vec();
        let contents = vec![("/input".to_string(), base.clone())];
        let mut m = UpdateExistingFile::new(vec!["/input".to_string()])
            .with_baseline_contents(contents);

        let mut found_small_perturb = false;
        for _ in 0..200 {
            let mut delta = FsDelta::new(vec![FsOp::mkdir("/seed_dir")]);
            m.mutate(&mut state, &mut delta).unwrap();
            let new_op = delta.ops.last().unwrap();
            if new_op.kind == FsOpKind::UpdateFile
                && new_op.content.len() == base.len()
                && new_op.content != base
            {
                // same length but differing bytes → bit-flip branch
                found_small_perturb = true;
                break;
            }
        }
        assert!(found_small_perturb, "no bit-flip-style perturbation seen in 200 tries");
    }

    #[test]
    fn perturb_bytes_handles_empty_base() {
        // Empty base must not panic (below() on zero-length would).
        let mut rand = StdRand::with_seed(0x1234);
        for _ in 0..20 {
            let _ = perturb_bytes(&mut rand, &[]);
        }
    }

    // ── Content dictionary ────────────────────────────────────────────────

    #[test]
    fn replace_file_content_uses_dictionary_sometimes() {
        // Over many runs, at least one replacement should exactly match a
        // dictionary entry — confirms the 40% dictionary branch fires.
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
        assert!(used_dict, "ReplaceFileContent never picked a dictionary entry in 200 tries");
    }

    // ── MutatePath guidance ───────────────────────────────────────────────

    #[test]
    fn mutate_path_whole_swap_prefers_enoent_paths() {
        let mut state = TestState::new();
        let mut guidance = MutationGuidance::none();
        guidance.enoent_paths = vec!["/wanted/by/target".to_string()];
        let mut m = MutatePath::with_baseline(vec!["/etc/config".to_string()])
            .with_guidance(guidance);

        let mut used_enoent = false;
        for _ in 0..100 {
            let mut delta = FsDelta::new(vec![FsOp::mkdir("/start")]);
            m.mutate(&mut state, &mut delta).unwrap();
            if delta.ops[0].path == "/wanted/by/target" {
                used_enoent = true;
                break;
            }
        }
        assert!(used_enoent, "guidance.enoent_paths never used in whole-swap across 100 tries");
    }

    // ── DestructiveMutator guidance ───────────────────────────────────────

    #[test]
    fn destructive_mutator_delete_prefers_recreate_paths() {
        let mut state = TestState::new();
        let mut guidance = MutationGuidance::none();
        guidance.recreate_paths = vec!["/deleted/by/target".to_string()];
        let mut m = DestructiveMutator::with_baseline(
            vec!["/input".to_string()],
            vec!["/etc".to_string()],
            vec!["/input".to_string(), "/etc".to_string()],
        ).with_guidance(guidance);

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
        assert!(used_recreate, "recreate_paths never used by destructive in 200 tries");
    }
}
