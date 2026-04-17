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
 *   - Runs N iterations: mutate → apply delta → reset to snapshot.
 *   - Exercises all 7 mutator stages and verifies the VFS is clean after reset.
 *   - Tracks semantic yield: how often the delta actually changed the VFS.
 *   - Prints a per-iteration summary and an end-of-run report.
 *
 * Usage:
 *   cargo run --bin fuzz -- [iterations]   (default: 50)
 *
 * Note: the FUSE mount is NOT required for this binary.  It talks directly
 * to the in-memory VFS through the FFI layer.
 */

use std::env;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use fs_mutator::{
    delta::{generate_seed, initial_corpus_pool, FsDelta},
    ffi::{
        apply_delta, cp_vfs_checksum, vfs_create, vfs_create_file, vfs_destroy, vfs_mkdir,
        vfs_reset_to_snapshot, vfs_save_snapshot, VfsT,
    },
    mutators::{
        AddFileOp, ByteFlipFileContent, DestructiveMutator, MutatePath, RemoveOp,
        ReplaceFileContent, SpliceDelta,
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
    fn rand(&self) -> &StdRand {
        &self.rand
    }
    fn rand_mut(&mut self) -> &mut StdRand {
        &mut self.rand
    }
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
// Mutator pool — one instance of each of the 7 stages
// ─────────────────────────────────────────────────────────────────────────────

enum AnyMutator {
    ByteFlip(ByteFlipFileContent),
    Replace(ReplaceFileContent),
    AddFile(AddFileOp),
    Remove(RemoveOp),
    MutatePath(MutatePath),
    Splice(SpliceDelta),
    Destructive(DestructiveMutator),
}

impl AnyMutator {
    fn name(&self) -> &'static str {
        match self {
            Self::ByteFlip(_)    => "ByteFlipFileContent",
            Self::Replace(_)     => "ReplaceFileContent",
            Self::AddFile(_)     => "AddFileOp",
            Self::Remove(_)      => "RemoveOp",
            Self::MutatePath(_)  => "MutatePath",
            Self::Splice(_)      => "SpliceDelta",
            Self::Destructive(_) => "DestructiveMutator",
        }
    }

    fn mutate(
        &mut self,
        state: &mut DumbState,
        input: &mut FsDelta,
    ) -> Result<MutationResult, libafl::Error> {
        match self {
            Self::ByteFlip(m)    => m.mutate(state, input),
            Self::Replace(m)     => m.mutate(state, input),
            Self::AddFile(m)     => m.mutate(state, input),
            Self::Remove(m)      => m.mutate(state, input),
            Self::MutatePath(m)  => m.mutate(state, input),
            Self::Splice(m)      => m.mutate(state, input),
            Self::Destructive(m) => m.mutate(state, input),
        }
    }
}

fn build_mutator_pool() -> Vec<AnyMutator> {
    vec![
        AnyMutator::ByteFlip(ByteFlipFileContent::new()),
        AnyMutator::Replace(ReplaceFileContent::new()),
        AnyMutator::AddFile(AddFileOp::new()),
        AnyMutator::Remove(RemoveOp::new()),
        AnyMutator::MutatePath(MutatePath::new()),
        AnyMutator::Splice(SpliceDelta::new(initial_corpus_pool())),
        AnyMutator::Destructive(DestructiveMutator::new()),
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

    // ── Mutator pool and state ────────────────────────────────────────────
    let mut mutators = build_mutator_pool();
    let mut state    = DumbState::new(0xdeadbeef_cafebabe);

    // ── Seed delta ────────────────────────────────────────────────────────
    let seed = generate_seed();

    // Per-iteration statistics.
    let mut n_mutated       = 0usize;
    let mut n_skipped       = 0usize;
    let mut n_apply_ok      = 0usize;  // apply_delta() returned Ok
    let mut n_apply_partial = 0usize;  // Ok but ≥1 op individually failed at VFS level
    let mut n_apply_err     = 0usize;  // catastrophic (OOM / null from C)
    let mut n_reset_ok      = 0usize;
    let mut n_reset_err     = 0usize;
    let mut n_yield         = 0usize;  // VFS checksum changed after apply

    // Reset cost timing.
    let mut reset_total = Duration::ZERO;
    let mut reset_min   = Duration::MAX;
    let mut reset_max   = Duration::ZERO;

    for i in 0..n_iters {
        // Start every iteration from the seed so results are reproducible.
        let mut delta = seed.clone();

        // Pick a random mutator and apply it.
        let m_idx  = state.rand_mut().below(NonZeroUsize::new(mutators.len()).unwrap());
        let result = mutators[m_idx].mutate(&mut state, &mut delta);
        let m_name = mutators[m_idx].name();

        let outcome = match result {
            Ok(MutationResult::Mutated) => { n_mutated += 1; "mutated" }
            Ok(MutationResult::Skipped) => { n_skipped += 1; "skipped" }
            Err(ref e) => {
                eprintln!("  iter {i}: mutator {m_name} returned error: {e}");
                "error"
            }
        };

        // Apply the (possibly mutated) delta to the VFS, then measure semantic
        // yield before resetting.
        let apply_ret     = apply_delta(vfs, &delta);
        let post_checksum = unsafe { cp_vfs_checksum(vfs) };

        let apply_tag = match &apply_ret {
            Ok(dr) => {
                n_apply_ok += 1;
                if dr.failed > 0 { n_apply_partial += 1; }
                if post_checksum != baseline_checksum {
                    n_yield += 1;
                    "ok(yield)"
                } else {
                    "ok"
                }
            }
            Err(e) => {
                n_apply_err += 1;
                eprintln!("  iter {i}: apply_delta returned errno {e}");
                "err"
            }
        };

        println!(
            "  iter {:>3}: mutator={:<22} outcome={:<8} ops={:>2}  apply={}",
            i, m_name, outcome, delta.ops.len(), apply_tag,
        );

        // Reset to the clean baseline snapshot — timed.
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
    let reset_mean_ns = if n_reset_ok > 0 {
        reset_total.as_nanos() / n_reset_ok as u128
    } else {
        0
    };
    let reset_min_ns = if n_reset_ok > 0 { reset_min.as_nanos() } else { 0 };
    let reset_max_ns = if n_reset_ok > 0 { reset_max.as_nanos() } else { 0 };
    let yield_pct    = if n_iters > 0 {
        100.0 * n_yield as f64 / n_iters as f64
    } else {
        0.0
    };

    println!("\n=== Summary ===");
    println!("  iterations     : {n_iters}");
    println!("  mutated        : {n_mutated}");
    println!("  skipped        : {n_skipped}");
    println!("  apply ok       : {n_apply_ok}");
    println!("  apply partial  : {n_apply_partial}  (ok but ≥1 op failed at VFS level)");
    println!("  apply err      : {n_apply_err}");
    println!("  reset ok       : {n_reset_ok}");
    println!("  reset err      : {n_reset_err}");
    println!("  semantic yield : {n_yield}/{n_iters} ({yield_pct:.1}%)");

    println!("\n=== Reset cost (vfs_reset_to_snapshot) ===");
    println!("  mean : {:>8} ns", reset_mean_ns);
    println!("  min  : {:>8} ns", reset_min_ns);
    println!("  max  : {:>8} ns", reset_max_ns);
    println!("  total: {:>8} µs", reset_total.as_micros());

    // Fail the run if any reset failed — stale state is the worst fuzzing bug.
    assert_eq!(n_reset_err, 0, "VFS reset failures detected — stale state bug");

    unsafe { vfs_destroy(vfs) };

    println!("\nAll resets clean. Phase A dumb loop OK.");
}
