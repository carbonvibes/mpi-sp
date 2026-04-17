/*
 * vfs_bench.rs — Direct VFS benchmark (no FUSE, no mutator, no FFI delta).
 *
 * Measures the raw cost of two operations that sit on the critical path of
 * every fuzzing iteration:
 *
 *   1. vfs_reset_to_snapshot  — restoring the VFS to baseline after each run.
 *   2. apply_delta            — applying a mutated delta to the live VFS.
 *
 * Both are measured over a tight loop with a warm snapshot so the numbers
 * reflect steady-state throughput, not first-call overhead.
 *
 * Usage:
 *   cargo run --release --bin vfs_bench -- [iterations]   (default: 1000)
 *
 * Output is written to stdout in a format that docs/benchmark_baseline.md
 * can be copy-pasted from directly.
 */

use std::env;
use std::time::{Duration, Instant};

use fs_mutator::{
    delta::{FsDelta, FsOp},
    ffi::{
        apply_delta, vfs_create, vfs_create_file, vfs_destroy, vfs_mkdir,
        vfs_reset_to_snapshot, vfs_save_snapshot, VfsT,
    },
};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

struct Stats {
    n: usize,
    total: Duration,
    min: Duration,
    max: Duration,
}

impl Stats {
    fn new() -> Self {
        Self { n: 0, total: Duration::ZERO, min: Duration::MAX, max: Duration::ZERO }
    }

    fn record(&mut self, d: Duration) {
        self.n += 1;
        self.total += d;
        if d < self.min { self.min = d; }
        if d > self.max { self.max = d; }
    }

    fn mean_ns(&self) -> u128 {
        if self.n == 0 { 0 } else { self.total.as_nanos() / self.n as u128 }
    }

    fn print(&self, label: &str) {
        println!("  {label}");
        println!("    n     : {}", self.n);
        println!("    mean  : {:>9} ns  ({:.2} µs)", self.mean_ns(), self.mean_ns() as f64 / 1000.0);
        println!("    min   : {:>9} ns", self.min.as_nanos());
        println!("    max   : {:>9} ns", self.max.as_nanos());
        println!("    total : {:>9} µs", self.total.as_micros());
        let iters_per_sec = if self.total.as_secs_f64() > 0.0 {
            self.n as f64 / self.total.as_secs_f64()
        } else {
            f64::INFINITY
        };
        println!("    rate  : {:.0} iters/s", iters_per_sec);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Baseline population
// ─────────────────────────────────────────────────────────────────────────────

unsafe fn populate_baseline(vfs: *mut VfsT) {
    vfs_create_file(vfs, c"/input".as_ptr(), b"seed".as_ptr(), 4);
    vfs_mkdir(vfs, c"/etc".as_ptr());
    vfs_create_file(
        vfs,
        c"/etc/config".as_ptr(),
        b"[settings]\nverbose=0\n".as_ptr(),
        20,
    );
    vfs_mkdir(vfs, c"/data".as_ptr());
    let magic: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];
    vfs_create_file(vfs, c"/data/a.bin".as_ptr(), magic.as_ptr(), magic.len());
}

// ─────────────────────────────────────────────────────────────────────────────
// Representative deltas of varying complexity
// ─────────────────────────────────────────────────────────────────────────────

fn delta_small() -> FsDelta {
    FsDelta::new(vec![
        FsOp::update_file("/input", b"mutated_content_12345".to_vec()),
    ])
}

fn delta_medium() -> FsDelta {
    FsDelta::new(vec![
        FsOp::update_file("/input", b"mutated_content_12345".to_vec()),
        FsOp::create_file("/tmp/new.txt", vec![0u8; 64]),
        FsOp::mkdir("/tmp/subdir"),
    ])
}

fn delta_large() -> FsDelta {
    let mut ops = Vec::new();
    for i in 0..8u8 {
        ops.push(FsOp::create_file(
            format!("/data/file_{i}.bin"),
            vec![i; 256],
        ));
    }
    ops.push(FsOp::update_file("/input", b"overwritten".to_vec()));
    ops.push(FsOp::mkdir("/tmp/deep/nested"));
    FsDelta::new(ops)
}

// ─────────────────────────────────────────────────────────────────────────────
// Main
// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    let n_iters: usize = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    println!("=== VFS Direct Benchmark ({n_iters} iterations each) ===\n");
    println!("  (no FUSE, no mutator overhead — raw C API cost)\n");

    // ── Set up VFS ────────────────────────────────────────────────────────
    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");

    unsafe { populate_baseline(vfs) };

    let snap_ret = unsafe { vfs_save_snapshot(vfs) };
    assert_eq!(snap_ret, 0, "vfs_save_snapshot() failed");

    // ─────────────────────────────────────────────────────────────────────
    // Benchmark 1: vfs_reset_to_snapshot cost (no apply — pure reset)
    // ─────────────────────────────────────────────────────────────────────
    println!("── Benchmark 1: vfs_reset_to_snapshot (baseline tree, no prior apply) ──");
    {
        let mut stats = Stats::new();
        for _ in 0..n_iters {
            let t0 = Instant::now();
            let ret = unsafe { vfs_reset_to_snapshot(vfs) };
            stats.record(t0.elapsed());
            assert_eq!(ret, 0, "reset failed");
        }
        stats.print("reset only");
    }
    println!();

    // ─────────────────────────────────────────────────────────────────────
    // Benchmark 2: apply_delta + reset  (small delta — 1 op)
    // ─────────────────────────────────────────────────────────────────────
    println!("── Benchmark 2: apply_delta + reset  (small: 1 op) ──");
    {
        let delta = delta_small();
        let mut apply_stats = Stats::new();
        let mut reset_stats = Stats::new();

        for _ in 0..n_iters {
            let t0 = Instant::now();
            let _ = apply_delta(vfs, &delta);
            apply_stats.record(t0.elapsed());

            let t1 = Instant::now();
            let ret = unsafe { vfs_reset_to_snapshot(vfs) };
            reset_stats.record(t1.elapsed());
            assert_eq!(ret, 0);
        }
        apply_stats.print("apply (1 op)");
        reset_stats.print("reset after apply");
    }
    println!();

    // ─────────────────────────────────────────────────────────────────────
    // Benchmark 3: apply_delta + reset  (medium delta — 3 ops)
    // ─────────────────────────────────────────────────────────────────────
    println!("── Benchmark 3: apply_delta + reset  (medium: 3 ops) ──");
    {
        let delta = delta_medium();
        let mut apply_stats = Stats::new();
        let mut reset_stats = Stats::new();

        for _ in 0..n_iters {
            let t0 = Instant::now();
            let _ = apply_delta(vfs, &delta);
            apply_stats.record(t0.elapsed());

            let t1 = Instant::now();
            let ret = unsafe { vfs_reset_to_snapshot(vfs) };
            reset_stats.record(t1.elapsed());
            assert_eq!(ret, 0);
        }
        apply_stats.print("apply (3 ops)");
        reset_stats.print("reset after apply");
    }
    println!();

    // ─────────────────────────────────────────────────────────────────────
    // Benchmark 4: apply_delta + reset  (large delta — 10 ops)
    // ─────────────────────────────────────────────────────────────────────
    println!("── Benchmark 4: apply_delta + reset  (large: 10 ops) ──");
    {
        let delta = delta_large();
        let mut apply_stats = Stats::new();
        let mut reset_stats = Stats::new();

        for _ in 0..n_iters {
            let t0 = Instant::now();
            let _ = apply_delta(vfs, &delta);
            apply_stats.record(t0.elapsed());

            let t1 = Instant::now();
            let ret = unsafe { vfs_reset_to_snapshot(vfs) };
            reset_stats.record(t1.elapsed());
            assert_eq!(ret, 0);
        }
        apply_stats.print("apply (10 ops)");
        reset_stats.print("reset after apply");
    }
    println!();

    unsafe { vfs_destroy(vfs) };

    println!("=== Benchmark complete ===");
}
