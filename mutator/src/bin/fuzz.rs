/*
 * fuzz.rs — Phase A dumb loop harness.
 *
 * Validates that the mutation → apply → reset cycle is correct and stable
 * before the full LibAFL harness and FUSE target execution are wired in
 * (Week 6 / Phase B).
 *
 * What this does NOT do (Phase B):
 *   - Fork / exec the target binary through the FUSE mount.
 *   - Collect the FUSE write log and populate MutationGuidance.
 *   - Manage a LibAFL corpus.
 *
 * What this DOES do (Phase A):
 *   - Creates an in-memory VFS, seeds a baseline, saves a snapshot.
 *   - Enumerates baseline paths (files / dirs / all) so mutators can target
 *     real VFS entries with op-type-aware path selection.
 *   - Builds a diverse seed corpus (7 families) from the baseline paths.
 *   - Runs N iterations:
 *       • pick a random seed from the corpus
 *       • apply 1–3 random mutations
 *       • apply the delta to the VFS
 *       • track semantic yield (checksum change)
 *       • reset to the saved snapshot
 *   - Exercises all 8 mutator stages; verifies the VFS is clean after every reset.
 *   - Prints a per-iteration summary and an end-of-run report.
 *
 * Usage:
 *   cargo run --bin fuzz -- [iterations]   (default: 50)
 *
 * Note: the FUSE mount is NOT required.  This binary talks directly to the
 * in-memory VFS through the FFI layer.
 */

use std::cell::RefCell;
use std::collections::HashSet;
use std::env;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::time::{Duration, Instant};

use fs_mutator::{
    delta::{generate_seed_corpus, initial_corpus_pool, FsDelta},
    ffi::{
        apply_delta, cp_vfs_checksum,
        enumerate_vfs_all_paths, enumerate_vfs_dir_paths, enumerate_vfs_file_paths,
        vfs_create, vfs_create_file, vfs_destroy, vfs_mkdir,
        vfs_reset_to_snapshot, vfs_save_snapshot, VfsT,
    },
    mutators::{
        AddFileOp, ByteFlipFileContent, DestructiveMutator, LiveCorpus, MutatePath,
        RemoveOp, ReplaceFileContent, SpliceDelta, UpdateExistingFile,
        MAX_LIVE_CORPUS, MAX_OPS,
    },
};

use libafl::{mutators::{MutationResult, Mutator}, state::HasRand};
use libafl_bolts::rands::{Rand, StdRand};

// ─────────────────────────────────────────────────────────────────────────────
// Minimal state — only needs HasRand for Phase A
// ─────────────────────────────────────────────────────────────────────────────

struct DumbState {
    rand: StdRand,
}

impl DumbState {
    fn new(seed: u64) -> Self {
        Self { rand: StdRand::with_seed(seed) }
    }
}

impl HasRand for DumbState {
    type Rand = StdRand;
    fn rand(&self) -> &StdRand     { &self.rand }
    fn rand_mut(&mut self) -> &mut StdRand { &mut self.rand }
}

// ─────────────────────────────────────────────────────────────────────────────
// Baseline population
// ─────────────────────────────────────────────────────────────────────────────

unsafe fn populate_baseline(vfs: *mut VfsT) {
    let seed_content   = b"seed";
    let input_path     = c"/input";
    let etc_path       = c"/etc";
    let config_path    = c"/etc/config";
    let config_content = b"[settings]\nverbose=0\n";

    vfs_create_file(vfs, input_path.as_ptr(), seed_content.as_ptr(), seed_content.len());
    vfs_mkdir(vfs, etc_path.as_ptr());
    vfs_create_file(
        vfs,
        config_path.as_ptr(),
        config_content.as_ptr(),
        config_content.len(),
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Mutator pool — one instance of each of the 8 stages
// ─────────────────────────────────────────────────────────────────────────────

enum AnyMutator {
    ByteFlip(ByteFlipFileContent),
    Replace(ReplaceFileContent),
    AddFile(AddFileOp),
    Remove(RemoveOp),
    MutatePath(MutatePath),
    Splice(SpliceDelta),
    Destructive(DestructiveMutator),
    UpdateExisting(UpdateExistingFile),
}

impl AnyMutator {
    fn name(&self) -> &'static str {
        match self {
            Self::ByteFlip(_)       => "ByteFlipFileContent",
            Self::Replace(_)        => "ReplaceFileContent",
            Self::AddFile(_)        => "AddFileOp",
            Self::Remove(_)         => "RemoveOp",
            Self::MutatePath(_)     => "MutatePath",
            Self::Splice(_)         => "SpliceDelta",
            Self::Destructive(_)    => "DestructiveMutator",
            Self::UpdateExisting(_) => "UpdateExistingFile",
        }
    }

    fn mutate(
        &mut self,
        state: &mut DumbState,
        input: &mut FsDelta,
    ) -> Result<MutationResult, libafl::Error> {
        match self {
            Self::ByteFlip(m)       => m.mutate(state, input),
            Self::Replace(m)        => m.mutate(state, input),
            Self::AddFile(m)        => m.mutate(state, input),
            Self::Remove(m)         => m.mutate(state, input),
            Self::MutatePath(m)     => m.mutate(state, input),
            Self::Splice(m)         => m.mutate(state, input),
            Self::Destructive(m)    => m.mutate(state, input),
            Self::UpdateExisting(m) => m.mutate(state, input),
        }
    }
}

impl AnyMutator {
    /// Return false when the mutator would definitely return `Skipped` on
    /// this delta.  The harness uses this to pick a stage that has a real
    /// chance of doing work, rather than burning 1–3 mutation budget slots
    /// on guaranteed skips.  This is a fast structural check only — it does
    /// not guarantee the mutation will succeed, only that preconditions are met.
    fn can_apply(&self, d: &FsDelta) -> bool {
        use fs_mutator::delta::FsOpKind;
        match self {
            Self::ByteFlip(_)       => d.ops.iter().any(|o|
                matches!(o.kind, FsOpKind::CreateFile | FsOpKind::UpdateFile)
                    && !o.content.is_empty()),
            Self::Replace(_)        => d.ops.iter().any(|o|
                matches!(o.kind, FsOpKind::CreateFile | FsOpKind::UpdateFile)),
            Self::AddFile(_)        => d.ops.len() < MAX_OPS,
            Self::Remove(_)         => d.ops.len() > 1,
            Self::MutatePath(_)     => !d.ops.is_empty(),
            Self::Splice(m)         => d.ops.len() < MAX_OPS && m.pool_len() > 0,
            Self::Destructive(_)    => d.ops.len() < MAX_OPS,
            Self::UpdateExisting(m) => d.ops.len() < MAX_OPS && m.has_baseline(),
        }
    }
}

fn build_mutator_pool(
    file_paths: Vec<String>,
    dir_paths:  Vec<String>,
    all_paths:  Vec<String>,
    live_corpus: LiveCorpus,
    baseline_contents: Vec<(String, Vec<u8>)>,
) -> Vec<AnyMutator> {
    vec![
        AnyMutator::ByteFlip(ByteFlipFileContent::new()),
        AnyMutator::Replace(ReplaceFileContent::new()),
        AnyMutator::AddFile(AddFileOp::new()),
        AnyMutator::Remove(RemoveOp::new()),
        AnyMutator::MutatePath(MutatePath::with_baseline(all_paths.clone())),
        AnyMutator::Splice(SpliceDelta::new(live_corpus)),
        AnyMutator::Destructive(DestructiveMutator::with_baseline(
            file_paths.clone(),
            dir_paths,
            all_paths,
        )),
        AnyMutator::UpdateExisting(
            UpdateExistingFile::new(file_paths).with_baseline_contents(baseline_contents),
        ),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Main
// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    let n_iters: usize = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);

    println!("=== Phase A dumb loop: {n_iters} iterations ===\n");

    // ── Set up VFS ────────────────────────────────────────────────────────
    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");

    unsafe { populate_baseline(vfs) };

    let snap_ret = unsafe { vfs_save_snapshot(vfs) };
    assert_eq!(snap_ret, 0, "vfs_save_snapshot() failed");

    // Baseline checksum — used for semantic yield tracking.
    let baseline_checksum = unsafe { cp_vfs_checksum(vfs) };

    // ── Enumerate baseline paths ──────────────────────────────────────────
    let baseline_file_paths = enumerate_vfs_file_paths(vfs);
    let baseline_dir_paths  = enumerate_vfs_dir_paths(vfs);
    let baseline_all_paths  = enumerate_vfs_all_paths(vfs);
    println!(
        "Baseline: {} file(s), {} dir(s), {} total — {:?}",
        baseline_file_paths.len(),
        baseline_dir_paths.len(),
        baseline_all_paths.len(),
        baseline_all_paths,
    );

    // Baseline file contents — known at populate time; no FFI needed in Phase A.
    // UpdateExistingFile uses these to perturb live content instead of emitting
    // 1–64 random bytes.  Targets reading structured content (like /etc/config)
    // get mutations grounded in the real file shape.
    let baseline_contents: Vec<(String, Vec<u8>)> = vec![
        ("/input".to_string(),      b"seed".to_vec()),
        ("/etc/config".to_string(), b"[settings]\nverbose=0\n".to_vec()),
    ];

    // ── Live corpus ──────────────────────────────────────────────────────
    // Starts with the 7 seed families + 4 initial donors so SpliceDelta has
    // donor diversity from iteration 0.  The harness pushes novel deltas
    // (unique post-apply checksum) into this pool up to MAX_LIVE_CORPUS.
    let mut initial: Vec<FsDelta> = generate_seed_corpus(&baseline_file_paths);
    let seed_count = initial.len();
    initial.extend(initial_corpus_pool());
    let live_corpus: LiveCorpus = Rc::new(RefCell::new(initial));

    println!(
        "Live corpus: seeded with {} entries ({} seed families + {} donors)\n",
        live_corpus.borrow().len(),
        seed_count,
        live_corpus.borrow().len() - seed_count,
    );

    // ── Mutator pool and state ────────────────────────────────────────────
    let mut mutators = build_mutator_pool(
        baseline_file_paths,
        baseline_dir_paths,
        baseline_all_paths,
        live_corpus.clone(),
        baseline_contents,
    );
    let mut state = DumbState::new(0xdeadbeef_cafebabe);

    // Per-iteration statistics.
    let mut n_iters_mutated  = 0usize;   // iterations with ≥1 successful mutation
    let mut n_iters_skipped  = 0usize;   // iterations where every mutation skipped
    let mut n_apply_ok       = 0usize;
    let mut n_apply_partial  = 0usize;
    let mut n_apply_err      = 0usize;
    let mut n_reset_ok       = 0usize;
    let mut n_reset_err      = 0usize;
    let mut n_yield          = 0usize;
    let mut n_promoted       = 0usize;

    // Reset cost timing.
    let mut reset_total = Duration::ZERO;
    let mut reset_min   = Duration::MAX;
    let mut reset_max   = Duration::ZERO;

    // Seen post-apply checksums — used to dedup promotions into the live corpus.
    let mut seen_checksums: HashSet<u64> = HashSet::new();
    seen_checksums.insert(baseline_checksum);

    let nz = |n: usize| NonZeroUsize::new(n).unwrap();

    for i in 0..n_iters {
        // Pick a seed from the live corpus (includes seeds + promoted deltas).
        let (seed_idx, mut delta) = {
            let corpus = live_corpus.borrow();
            let idx    = state.rand_mut().below(nz(corpus.len()));
            (idx, corpus[idx].clone())
        };

        // Apply 1–3 mutations to the same delta before applying to the VFS.
        // Each mutation picks from the subset of mutators that can_apply to
        // the current delta — this skips guaranteed no-ops (e.g. RemoveOp on
        // a 1-op delta) without burning a mutation budget slot.
        let n_mut = 1 + state.rand_mut().below(nz(3));
        let mut any_mutated = false;
        let mut first_name  = "";
        for j in 0..n_mut {
            // Build the applicable subset.  Fall back to uniform pick if
            // somehow every mutator is inapplicable (shouldn't happen with
            // the current 8 stages, but defensive).
            let applicable: Vec<usize> = (0..mutators.len())
                .filter(|&k| mutators[k].can_apply(&delta))
                .collect();
            let m_idx = if applicable.is_empty() {
                state.rand_mut().below(nz(mutators.len()))
            } else {
                applicable[state.rand_mut().below(nz(applicable.len()))]
            };
            if j == 0 { first_name = mutators[m_idx].name(); }
            match mutators[m_idx].mutate(&mut state, &mut delta) {
                Ok(MutationResult::Mutated) => { any_mutated = true; }
                Ok(MutationResult::Skipped) => {}
                Err(ref e) => {
                    eprintln!("  iter {i}: mutator {} error: {e}", mutators[m_idx].name());
                }
            }
        }
        if any_mutated { n_iters_mutated += 1; } else { n_iters_skipped += 1; }

        // Apply delta, measure yield.
        let apply_ret     = apply_delta(vfs, &delta);
        let post_checksum = unsafe { cp_vfs_checksum(vfs) };

        let mut promoted = false;
        let apply_tag = match &apply_ret {
            Ok(dr) => {
                n_apply_ok += 1;
                if dr.failed > 0 { n_apply_partial += 1; }
                if post_checksum != baseline_checksum {
                    n_yield += 1;
                    // Promote on novel post-apply checksum.  The live corpus
                    // is bounded at MAX_LIVE_CORPUS; once full, we replace a
                    // random non-seed entry (preserves structural diversity).
                    if seen_checksums.insert(post_checksum) {
                        promoted = true;
                        n_promoted += 1;
                        let mut corpus = live_corpus.borrow_mut();
                        if corpus.len() < MAX_LIVE_CORPUS {
                            corpus.push(delta.clone());
                        } else {
                            let victim = seed_count
                                + state.rand_mut().below(nz(corpus.len() - seed_count));
                            corpus[victim] = delta.clone();
                        }
                    }
                    if promoted { "ok(yield+)" } else { "ok(yield)" }
                } else { "ok" }
            }
            Err(e) => {
                n_apply_err += 1;
                eprintln!("  iter {i}: apply_delta errno {e}");
                "err"
            }
        };

        let outcome = if any_mutated { "mutated" } else { "skipped" };
        println!(
            "  iter {:>3}: seed={:>3} muts={} first={:<22} outcome={:<8} ops={:>2}  apply={}",
            i, seed_idx, n_mut, first_name, outcome, delta.ops.len(), apply_tag,
        );

        // Reset — timed.
        let t0        = Instant::now();
        let reset_ret = unsafe { vfs_reset_to_snapshot(vfs) };
        let elapsed   = t0.elapsed();

        if reset_ret == 0 {
            n_reset_ok  += 1;
            reset_total += elapsed;
            if elapsed < reset_min { reset_min = elapsed; }
            if elapsed > reset_max { reset_max = elapsed; }
        } else {
            n_reset_err += 1;
            eprintln!("  iter {i}: vfs_reset_to_snapshot() returned {reset_ret}");
        }
    }

    // ── Summary ───────────────────────────────────────────────────────────
    let reset_mean_ns = if n_reset_ok > 0 { reset_total.as_nanos() / n_reset_ok as u128 } else { 0 };
    let reset_min_ns  = if n_reset_ok > 0 { reset_min.as_nanos() } else { 0 };
    let reset_max_ns  = if n_reset_ok > 0 { reset_max.as_nanos() } else { 0 };
    let yield_pct     = if n_iters > 0 { 100.0 * n_yield as f64 / n_iters as f64 } else { 0.0 };

    println!("\n=== Summary ===");
    println!("  iterations     : {n_iters}");
    println!("  iters mutated  : {n_iters_mutated}");
    println!("  iters skipped  : {n_iters_skipped}");
    println!("  apply ok       : {n_apply_ok}");
    println!("  apply partial  : {n_apply_partial}  (ok but ≥1 op failed at VFS level)");
    println!("  apply err      : {n_apply_err}");
    println!("  reset ok       : {n_reset_ok}");
    println!("  reset err      : {n_reset_err}");
    println!("  semantic yield : {n_yield}/{n_iters} ({yield_pct:.1}%)");
    println!(
        "  promoted       : {n_promoted}  (novel-checksum deltas pushed to live corpus)",
    );
    println!("  corpus final   : {} entries", live_corpus.borrow().len());

    println!("\n=== Reset cost (vfs_reset_to_snapshot) ===");
    println!("  mean : {:>8} ns", reset_mean_ns);
    println!("  min  : {:>8} ns", reset_min_ns);
    println!("  max  : {:>8} ns", reset_max_ns);
    println!("  total: {:>8} µs", reset_total.as_micros());

    assert_eq!(n_reset_err, 0, "VFS reset failures detected — stale state bug");

    unsafe { vfs_destroy(vfs) };

    println!("\nAll resets clean. Phase A dumb loop OK.");
}
