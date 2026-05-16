//! fuzz_combined_afl — Campaign 3: Nautilus config grammar + FUSE rootfs mutation.
//!
//! Both dimensions are mutated every iteration:
//!   config.json  ← Nautilus grammar (OCI JSON), root.path overridden to FUSE mount
//!   rootfs       ← FUSE VFS mutated via FsDelta
//!
//! Run (as root, from /tmp/campaign3/):
//!   mkdir -p /tmp/campaign3
//!   cd /tmp/campaign3
//!   sudo unshare -m /path/to/fuzz_combined_afl <crun> <grammar.py> 2>&1 | tee /tmp/c3.log

use std::{
    borrow::Cow,
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    str::FromStr,
    thread,
    time::Duration,
};

use libafl::{
    BloomInputFilter, HasMetadata, StdFuzzerBuilder,
    corpus::{Corpus, CorpusId, OnDiskCorpus},
    events::{ProgressReporter, SimpleEventManager},
    executors::{HasObservers, StdChildArgs, forkserver::ForkserverExecutor},
    feedback_and_fast, feedback_or,
    feedbacks::{CrashFeedback, MaxMapFeedback, TimeFeedback,
                NautilusChunksMetadata},
    fuzzer::{Evaluator, Fuzzer},
    generators::{Generator, NautilusContext, NautilusGenerator},
    inputs::{Input, NautilusBytesConverter, NautilusInput, ToTargetBytes},
    monitors::SimpleMonitor,
    mutators::{
        HavocScheduledMutator, MutationResult, Mutator,
        NautilusRandomMutator, NautilusRecursionMutator, NautilusSpliceMutator,
    },
    observers::{CanTrack, HitcountsMapObserver, StdMapObserver, TimeObserver},
    schedulers::{IndexesLenTimeMinimizerScheduler, QueueScheduler},
    stages::{AflStatsStage, StdMutationalStage},
    state::{HasCorpus, StdState},
    Error,
};
use libafl::nautilus::grammartec::tree::TreeLike;
use libafl_bolts::{
    AsSliceMut, HasLen, Named, StdTargetArgs, Truncate, current_nanos,
    ownedref::OwnedSlice,
    rands::StdRand,
    shmem::{ShMem, ShMemProvider, UnixShMemProvider},
    tuples::{Handled, tuple_list},
};
use nix::sys::signal::Signal;
use serde::{Deserialize, Serialize};

use fs_mutator::{
    delta::{FsDelta, FsOp},
    ffi::{
        apply_delta, enumerate_vfs_all_paths, enumerate_vfs_dir_paths, enumerate_vfs_file_paths,
        vfs_create, vfs_create_file, vfs_mkdir, vfs_reset_to_snapshot, vfs_save_snapshot, VfsT,
    },
    mutators::{
        AddFileOp, ByteFlipFileContent, DestructiveMutator, LiveCorpus, MutatePath, RemoveOp,
        ReplaceFileContent, ReplayWriteFile, SpliceDelta, UpdateExistingFile,
    },
};

#[cfg(has_fuse3)]
use fs_mutator::ffi::{fuse_vfs_lib_init, fuse_vfs_lib_is_mounted, fuse_vfs_lib_run};

const MAP_SIZE: usize = 65536;

// ── CombinedInput ─────────────────────────────────────────────────────────────
// The corpus entry for Campaign 3. Both halves are mutated each round.

#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
pub struct CombinedInput {
    pub config: NautilusInput, // drives config.json content via Nautilus grammar
    pub rootfs: FsDelta,       // drives FUSE VFS state
}

impl Input for CombinedInput {
    fn generate_name(&self, idx: Option<CorpusId>) -> String {
        format!("combined_{}", idx.map(usize::from).unwrap_or(0))
    }
}

// Required by IndexesLenTimeMinimizerScheduler.
// Use FsDelta op count + 1 as a proxy for input "size".
impl HasLen for CombinedInput {
    fn len(&self) -> usize {
        self.rootfs.len().saturating_add(self.config.tree.size())
    }
}

// ── Mutator wrappers ──────────────────────────────────────────────────────────
//
// LibAFL mutators are typed as Mutator<NautilusInput, S> or Mutator<FsDelta, S>.
// These thin wrappers let both operate on CombinedInput by delegating to the
// appropriate sub-field.

pub struct ConfigMutator<M> {
    inner: M,
    name:  Cow<'static, str>,
}

impl<M: Named> ConfigMutator<M> {
    pub fn new(inner: M) -> Self {
        let name = Cow::Owned(format!("Config({})", inner.name()));
        Self { inner, name }
    }
}

impl<M: Named> Named for ConfigMutator<M> {
    fn name(&self) -> &Cow<'static, str> { &self.name }
}

impl<M, S> Mutator<CombinedInput, S> for ConfigMutator<M>
where
    M: Mutator<NautilusInput, S>,
{
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut CombinedInput,
    ) -> Result<MutationResult, Error> {
        self.inner.mutate(state, &mut input.config)
    }

    fn post_exec(
        &mut self,
        state: &mut S,
        id: Option<CorpusId>,
    ) -> Result<(), Error> {
        self.inner.post_exec(state, id)
    }
}

pub struct RootfsMutator<M> {
    inner: M,
    name:  Cow<'static, str>,
}

impl<M: Named> RootfsMutator<M> {
    pub fn new(inner: M) -> Self {
        let name = Cow::Owned(format!("Rootfs({})", inner.name()));
        Self { inner, name }
    }
}

impl<M: Named> Named for RootfsMutator<M> {
    fn name(&self) -> &Cow<'static, str> { &self.name }
}

impl<M, S> Mutator<CombinedInput, S> for RootfsMutator<M>
where
    M: Mutator<FsDelta, S>,
{
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut CombinedInput,
    ) -> Result<MutationResult, Error> {
        self.inner.mutate(state, &mut input.rootfs)
    }

    fn post_exec(
        &mut self,
        state: &mut S,
        id: Option<CorpusId>,
    ) -> Result<(), Error> {
        self.inner.post_exec(state, id)
    }
}

// ── CombinedConverter ─────────────────────────────────────────────────────────
// Called by StdFuzzer before every forkserver execution.
//
// Steps per iteration:
//  1. Reset FUSE VFS to baseline snapshot, apply FsDelta  → rootfs mutation
//  2. Convert NautilusInput → JSON bytes via NautilusBytesConverter
//  3. Override "root.path" in the JSON with the FUSE mountpoint path
//  4. Write the modified JSON to config_path on disk      → config mutation
//  5. Return placeholder [0u8] — crun reads from argv[1], not stdin

struct CombinedConverter {
    context:      &'static NautilusContext,
    vfs:          *mut VfsT,
    config_path:  PathBuf,
    fuse_rootfs:  String,
    fallback_cfg: Vec<u8>,
}

unsafe impl Send for CombinedConverter {}
unsafe impl Sync for CombinedConverter {}

impl ToTargetBytes<CombinedInput> for CombinedConverter {
    fn to_target_bytes<'a>(&mut self, input: &'a CombinedInput) -> OwnedSlice<'a, u8> {
        // 1. Rootfs mutation
        unsafe { vfs_reset_to_snapshot(self.vfs) };
        let _ = apply_delta(self.vfs, &input.rootfs);

        // 2. Nautilus config → JSON
        let mut bytes_conv = NautilusBytesConverter::new(self.context);
        let raw = bytes_conv.to_target_bytes(&input.config);

        // 3. Override root.path
        let cfg = override_rootfs_path(&*raw, &self.fuse_rootfs)
            .unwrap_or_else(|| self.fallback_cfg.clone());

        // 4. Write to disk
        let _ = std::fs::write(&self.config_path, &cfg);

        // 5. Placeholder — crun reads config from argv[1]
        OwnedSlice::from(vec![0u8])
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse the Nautilus-generated JSON and force "root.path" to the FUSE mountpoint.
/// Returns None only if the JSON is completely unparseable.
fn override_rootfs_path(json: &[u8], fuse_rootfs: &str) -> Option<Vec<u8>> {
    let mut v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let obj = v.as_object_mut()?;
    // Ensure "root" key exists, then override "path"
    let root = obj
        .entry("root")
        .or_insert_with(|| serde_json::json!({"readonly": false}));
    if let Some(r) = root.as_object_mut() {
        r.insert(
            "path".to_string(),
            serde_json::Value::String(fuse_rootfs.to_string()),
        );
    }
    serde_json::to_vec(&v).ok()
}

/// Write a human-readable JSON sidecar for a corpus entry so the fuzz_dashboard can display it.
/// The sidecar is named `combined_<idx>.json` and lives in the corpus dir.
/// Format: { "config": "<rendered config string>", "ops": [<FsOp objects>] }
fn write_corpus_sidecar(
    corpus_dir: &std::path::Path,
    idx: usize,
    input: &CombinedInput,
    context: &'static NautilusContext,
) {
    let mut bytes_conv = NautilusBytesConverter::new(context);
    let raw = bytes_conv.to_target_bytes(&input.config);
    let config_str = String::from_utf8_lossy(&*raw).into_owned();

    let json = serde_json::json!({
        "config": config_str,
        "ops":    input.rootfs.ops,
    });

    let path = corpus_dir.join(format!("combined_{}.json", idx));
    let _ = std::fs::write(&path, json.to_string());
}

/// Minimal valid OCI config used when Nautilus JSON fails to parse.
fn make_fallback_config(rootfs_path: &str) -> Vec<u8> {
    serde_json::json!({
        "ociVersion": "1.0.0",
        "process": {
            "terminal": false,
            "user": {"username": "root"},
            "args": ["/bin/true"],
            "env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
            "cwd": "/"
        },
        "root": {"path": rootfs_path, "readonly": false},
        "hostname": "fuzz",
        "mounts": [
            {"destination": "/proc", "type": "proc",   "source": "proc"},
            {"destination": "/dev",  "type": "tmpfs",  "source": "tmpfs",
             "options": ["nosuid","strictatime","mode=755","size=65536k"]},
            {"destination": "/sys",  "type": "sysfs",  "source": "sysfs",
             "options": ["nosuid","noexec","nodev","ro"]},
            {"destination": "/tmp",  "type": "tmpfs",  "source": "tmpfs"}
        ],
        "linux": {
            "namespaces": [{"type": "pid"}, {"type": "mount"}]
        }
    })
    .to_string()
    .into_bytes()
}

// ── VFS baseline ──────────────────────────────────────────────────────────────

unsafe fn init_vfs(vfs: *mut VfsT, bin_true: &[u8]) {
    for dir in &[c"/bin", c"/proc", c"/dev", c"/sys", c"/tmp", c"/etc", c"/var", c"/run"] {
        vfs_mkdir(vfs, dir.as_ptr());
    }
    if !bin_true.is_empty() {
        vfs_create_file(vfs, c"/bin/true".as_ptr(), bin_true.as_ptr(), bin_true.len());
    }
    macro_rules! mkfile {
        ($path:expr, $content:expr) => {
            vfs_create_file(vfs, $path.as_ptr(), $content.as_ptr(), $content.len())
        };
    }
    mkfile!(c"/etc/passwd",
        b"root:x:0:0:root:/root:/bin/sh\nnobody:x:65534:65534:nobody:/:/usr/sbin/nologin\n");
    mkfile!(c"/etc/group",
        b"root:x:0:\ndaemon:x:1:\nbin:x:2:\nnobody:x:65534:\n");
    mkfile!(c"/etc/hosts",      b"127.0.0.1 localhost\n::1 localhost\n");
    mkfile!(c"/etc/hostname",   b"fuzz\n");
    mkfile!(c"/etc/resolv.conf", b"nameserver 8.8.8.8\n");
}

// ── FUSE startup ──────────────────────────────────────────────────────────────

#[cfg(has_fuse3)]
fn start_fuse(vfs: *mut VfsT, mountpoint: &str) {
    unsafe { fuse_vfs_lib_init(vfs) };
    let mp = std::ffi::CString::new(mountpoint).expect("mountpoint nul");
    thread::spawn(move || unsafe { fuse_vfs_lib_run(mp.as_ptr()) });

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if unsafe { fuse_vfs_lib_is_mounted() } != 0 { break; }
        if std::time::Instant::now() > deadline {
            eprintln!("ERROR: FUSE mount timed out at {mountpoint}");
            std::process::exit(1);
        }
        thread::sleep(Duration::from_millis(5));
    }
    println!("FUSE mounted at {mountpoint}");
}

#[cfg(not(has_fuse3))]
fn start_fuse(_vfs: *mut VfsT, _mountpoint: &str) {
    eprintln!("ERROR: libfuse3-dev not found at build time.");
    std::process::exit(1);
}

// ── Rootfs seed corpus ────────────────────────────────────────────────────────
// Identical to Campaign 2's seed set — all 6 groups.

fn rootfs_seeds(bin_true: &[u8]) -> Vec<FsDelta> {
    let mut seeds = vec![

        // ── Group 1: baseline and mount-target removals ───────────────────────
        // Clean rootfs — exercises full success path end-to-end
        FsDelta::new(vec![]),
        // Missing mount targets — exercises crun's mount-setup error paths
        FsDelta::new(vec![FsOp::rmdir("/proc")]),
        FsDelta::new(vec![FsOp::rmdir("/dev")]),
        FsDelta::new(vec![FsOp::rmdir("/sys")]),
        FsDelta::new(vec![FsOp::rmdir("/tmp")]),
        // Multiple mount targets missing at once
        FsDelta::new(vec![FsOp::rmdir("/proc"), FsOp::rmdir("/sys")]),
        FsDelta::new(vec![FsOp::rmdir("/dev"),  FsOp::rmdir("/tmp")]),
        FsDelta::new(vec![
            FsOp::rmdir("/proc"), FsOp::rmdir("/dev"),
            FsOp::rmdir("/sys"),  FsOp::rmdir("/tmp"),
        ]),

        // ── Group 2: binary execution variants ───────────────────────────────
        // Missing binary — ENOENT in execve
        FsDelta::new(vec![FsOp::delete_file("/bin/true")]),
        // Zero-length binary — ENOEXEC
        FsDelta::new(vec![FsOp::truncate("/bin/true", 0)]),
        // Truncate to small sizes — partial header reads
        FsDelta::new(vec![FsOp::truncate("/bin/true", 4)]),
        FsDelta::new(vec![FsOp::truncate("/bin/true", 16)]),
        FsDelta::new(vec![FsOp::truncate("/bin/true", 64)]),
        // Not an ELF at all
        FsDelta::new(vec![FsOp::update_file("/bin/true", b"not an elf\n".to_vec())]),
        // Valid magic but wrong class (32-bit ELF on 64-bit system → ENOEXEC)
        FsDelta::new(vec![FsOp::update_file("/bin/true", b"\x7fELF\x01\x01\x01\x00".to_vec())]),
        // Valid 64-bit magic but zeroed rest of header
        FsDelta::new(vec![FsOp::update_file("/bin/true",
            b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec())]),
        // Shell script without interpreter (ENOENT on /bin/sh)
        FsDelta::new(vec![FsOp::update_file("/bin/true", b"#!/bin/sh\nexit 0\n".to_vec())]),
        // Remove binary + its directory
        FsDelta::new(vec![FsOp::delete_file("/bin/true"), FsOp::rmdir("/bin")]),

        // ── Group 3: rich valid rootfs — exercises deeper success paths ───────
        // Full standard Linux directory tree (gives mutators more paths to work with)
        FsDelta::new(vec![
            FsOp::mkdir("/usr"),
            FsOp::mkdir("/usr/bin"),
            FsOp::mkdir("/usr/lib"),
            FsOp::mkdir("/usr/sbin"),
            FsOp::mkdir("/lib"),
            FsOp::mkdir("/lib64"),
            FsOp::mkdir("/sbin"),
            FsOp::mkdir("/opt"),
            FsOp::mkdir("/home"),
            FsOp::mkdir("/root"),
            FsOp::mkdir("/var/log"),
            FsOp::mkdir("/var/tmp"),
            FsOp::mkdir("/var/run"),
            FsOp::mkdir("/run/lock"),
        ]),
        // /dev pre-populated with entries crun's config references
        FsDelta::new(vec![
            FsOp::create_file("/dev/null",    b"".to_vec()),
            FsOp::create_file("/dev/zero",    b"".to_vec()),
            FsOp::create_file("/dev/full",    b"".to_vec()),
            FsOp::create_file("/dev/random",  b"".to_vec()),
            FsOp::create_file("/dev/urandom", b"".to_vec()),
            FsOp::create_file("/dev/tty",     b"".to_vec()),
            FsOp::create_file("/dev/console", b"".to_vec()),
            FsOp::create_file("/dev/ptmx",    b"".to_vec()),
            FsOp::mkdir("/dev/pts"),
            FsOp::mkdir("/dev/shm"),
        ]),
        // /etc files crun reads from rootfs
        FsDelta::new(vec![
            FsOp::create_file("/etc/group",
                b"root:x:0:\ndaemon:x:1:\nbin:x:2:\nnobody:x:65534:\n".to_vec()),
            FsOp::create_file("/etc/shadow",
                b"root:!:19000:0:99999:7:::\nnobody:!:19000::::::\n".to_vec()),
            FsOp::create_file("/etc/subuid", b"root:100000:65536\n".to_vec()),
            FsOp::create_file("/etc/subgid", b"root:100000:65536\n".to_vec()),
            FsOp::create_file("/etc/nsswitch.conf",
                b"passwd: files\ngroup: files\nhosts: files dns\n".to_vec()),
            FsOp::create_file("/etc/ld.so.cache",  b"\x00\x01\x02\x03".to_vec()),
            FsOp::create_file("/etc/ld.so.conf",   b"/usr/lib\n/lib\n".to_vec()),
            FsOp::create_file("/.dockerenv", b"".to_vec()),
        ]),
        // Extra binaries in /bin (exercises crun's path resolution)
        FsDelta::new(vec![
            FsOp::create_file("/bin/sh",   b"\x7fELF\x02\x01\x01\x00".to_vec()),
            FsOp::create_file("/bin/bash", b"\x7fELF\x02\x01\x01\x00".to_vec()),
            FsOp::create_file("/bin/ls",   b"\x7fELF\x02\x01\x01\x00".to_vec()),
        ]),

        // ── Group 4: rich rootfs + one broken thing ───────────────────────────
        // Full etc + missing /proc — gets deep into setup then fails at mount
        FsDelta::new(vec![
            FsOp::create_file("/etc/group",   b"root:x:0:\n".to_vec()),
            FsOp::create_file("/.dockerenv",  b"".to_vec()),
            FsOp::mkdir("/usr"),
            FsOp::mkdir("/lib"),
            FsOp::rmdir("/proc"),
        ]),
        // Full tree + corrupted binary — passes mount setup, fails at exec
        FsDelta::new(vec![
            FsOp::create_file("/etc/group",   b"root:x:0:\n".to_vec()),
            FsOp::mkdir("/usr"),
            FsOp::mkdir("/lib"),
            FsOp::update_file("/bin/true", b"\x7fELF\x01\x01\x01".to_vec()),
        ]),
        // Full tree + missing binary
        FsDelta::new(vec![
            FsOp::create_file("/etc/group",   b"root:x:0:\n".to_vec()),
            FsOp::mkdir("/usr"),
            FsOp::mkdir("/lib"),
            FsOp::delete_file("/bin/true"),
        ]),

        // ── Group 5: /etc removals ────────────────────────────────────────────
        FsDelta::new(vec![FsOp::delete_file("/etc/passwd")]),
        FsDelta::new(vec![FsOp::delete_file("/etc/hosts")]),
        FsDelta::new(vec![
            FsOp::delete_file("/etc/passwd"),
            FsOp::delete_file("/etc/hosts"),
            FsOp::delete_file("/etc/hostname"),
            FsOp::delete_file("/etc/resolv.conf"),
        ]),

        // ── Group 6: OCI / container-runtime specific ─────────────────────────
        FsDelta::new(vec![FsOp::create_file("/.dockerenv", b"".to_vec())]),
        FsDelta::new(vec![FsOp::create_file("/etc/ld.so.cache", b"\x00\x01".to_vec())]),
        // Proc populated before mount (crun overwrites with real proc)
        FsDelta::new(vec![
            FsOp::create_file("/proc/mounts", b"proc /proc proc rw 0 0\n".to_vec()),
            FsOp::create_file("/proc/self",   b"".to_vec()),
        ]),
    ];

    if bin_true.len() > 8 {
        // Corrupt ELF class field (offset 4)
        let mut c4 = bin_true.to_vec();
        c4[4] ^= 0xff;
        seeds.push(FsDelta::new(vec![FsOp::update_file("/bin/true", c4)]));

        // Corrupt ELF data field (offset 5 — endianness)
        let mut c5 = bin_true.to_vec();
        c5[5] ^= 0xff;
        seeds.push(FsDelta::new(vec![FsOp::update_file("/bin/true", c5)]));

        // Truncate to first 128 bytes — valid header, no content
        seeds.push(FsDelta::new(vec![FsOp::update_file(
            "/bin/true",
            bin_true[..128.min(bin_true.len())].to_vec(),
        )]));

        // Truncate to first 64 bytes — partial ELF header
        seeds.push(FsDelta::new(vec![FsOp::update_file(
            "/bin/true",
            bin_true[..64.min(bin_true.len())].to_vec(),
        )]));
    }

    seeds
}

// ── main ──────────────────────────────────────────────────────────────────────


fn main() {
    // Python runtime required for Nautilus grammar evaluation
    pyo3::prepare_freethreaded_python();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <crun-afl-binary> <grammar.py>", args[0]);
        eprintln!("  Run as root from /tmp/campaign3/");
        std::process::exit(1);
    }
    let crun_binary  = &args[1];
    let grammar_path = PathBuf::from(&args[2]);
    let pid          = std::process::id();

    println!("=== fuzz_combined_afl: Campaign 3 — Nautilus config + FUSE rootfs ===");
    println!("  crun    : {crun_binary}");
    println!("  grammar : {}", grammar_path.display());

    let cwd = std::env::current_dir()
        .expect("cannot determine CWD — run from /tmp/campaign3/");
    let corpus_dir    = cwd.join("corpus");
    let solutions_dir = cwd.join("crashes");
    let mountpoint    = format!("/tmp/campaign3-fuse-{pid}");
    let config_path   = cwd.join("config.json");

    for d in &[&corpus_dir, &solutions_dir] {
        std::fs::create_dir_all(d).unwrap_or_else(|e| {
            eprintln!("ERROR: cannot create {}: {e}", d.display());
            std::process::exit(1);
        });
    }
    std::fs::create_dir_all(&mountpoint).expect("failed to create FUSE mountpoint");

    let bin_true: Vec<u8> = std::fs::read("/bin/true")
        .or_else(|_| std::fs::read("/usr/bin/true"))
        .unwrap_or_default();

    // ── VFS + FUSE ────────────────────────────────────────────────────────────
    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");
    unsafe { init_vfs(vfs, &bin_true) };
    unsafe { vfs_save_snapshot(vfs) };

    let baseline_file_paths = enumerate_vfs_file_paths(vfs);
    let baseline_dir_paths  = enumerate_vfs_dir_paths(vfs);
    let baseline_all_paths  = enumerate_vfs_all_paths(vfs);
    let baseline_contents: Vec<(String, Vec<u8>)> = {
        let mut c = vec![("/etc/passwd".to_string(),
                          b"root:x:0:0:root:/root:/bin/sh\n".to_vec())];
        if !bin_true.is_empty() {
            c.push(("/bin/true".to_string(), bin_true.clone()));
        }
        c
    };

    start_fuse(vfs, &mountpoint);
    println!("  rootfs  : {mountpoint}");

    // ── Nautilus context (Box::leak → 'static so all components can borrow it) ─
    let context: &'static NautilusContext =
        Box::leak(Box::new(NautilusContext::from_file(100, grammar_path).unwrap()));

    let fallback_cfg = make_fallback_config(&mountpoint);

    // ── AFL shared memory (coverage map) ─────────────────────────────────────
    let mut shmem_provider = UnixShMemProvider::new().unwrap();
    let mut shmem = shmem_provider.new_shmem(MAP_SIZE).unwrap();
    unsafe { shmem.write_to_env("__AFL_SHM_ID").unwrap() };
    let shmem_buf = shmem.as_slice_mut();

    let edges_observer = unsafe {
        HitcountsMapObserver::new(StdMapObserver::new("shared_mem", shmem_buf)).track_indices()
    };
    let time_observer  = TimeObserver::new("time");
    let map_feedback   = MaxMapFeedback::new(&edges_observer);

    // ── AflStatsStage ─────────────────────────────────────────────────────────
    let tokens = libafl::mutators::Tokens::new();
    let afl_stats_stage = AflStatsStage::builder()
        .stats_file(PathBuf::from_str("fuzzer_stats").unwrap())
        .plot_file(PathBuf::from_str("plot_data").unwrap())
        .report_interval(Duration::from_secs(15))
        .map_feedback(&map_feedback)
        .tokens(&tokens)
        .banner("fuzz-combined-afl".into())
        .version("0.1.0".to_string())
        .exec_timeout(2)
        .build()
        .expect("AflStatsStage build failed");

    // ── Feedbacks + state ─────────────────────────────────────────────────────
    let mut feedback = feedback_or!(
        MaxMapFeedback::new(&edges_observer),
        TimeFeedback::new(&time_observer),
    );
    let mut objective = feedback_and_fast!(
        CrashFeedback::new(),
        MaxMapFeedback::with_name("mapfeedback_metadata_objective", &edges_observer),
    );

    let mut state = StdState::new(
        StdRand::with_seed(current_nanos()),
        OnDiskCorpus::<CombinedInput>::new(&corpus_dir).expect("corpus dir"),
        OnDiskCorpus::<CombinedInput>::new(&solutions_dir).expect("solutions dir"),
        &mut feedback,
        &mut objective,
    )
    .expect("StdState");

    // Nautilus tree chunk cache (required by NautilusRecursionMutator)
    let _ = state.metadata_or_insert_with::<NautilusChunksMetadata>(|| {
        NautilusChunksMetadata::new("/tmp/".into())
    });
    // Token metadata (required by AflStatsStage)
    state.add_metadata(tokens.clone());

    // ── Monitor + event manager ───────────────────────────────────────────────
    let monitor = SimpleMonitor::new(|s| {
        println!("{s}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    });
    let mut mgr = SimpleEventManager::new(monitor);

    // ── Scheduler ─────────────────────────────────────────────────────────────
    let observer_ref = edges_observer.handle();
    let scheduler    = IndexesLenTimeMinimizerScheduler::new(&edges_observer, QueueScheduler::new());

    // ── Converter ─────────────────────────────────────────────────────────────
    let converter = CombinedConverter {
        context,
        vfs,
        config_path:  config_path.clone(),
        fuse_rootfs:  mountpoint.clone(),
        fallback_cfg: fallback_cfg.clone(),
    };

    // ── Fuzzer ────────────────────────────────────────────────────────────────
    let mut fuzzer = StdFuzzerBuilder::new()
        .input_filter(BloomInputFilter::default())
        .target_bytes_converter(converter)
        .scheduler(scheduler)
        .feedback(feedback)
        .objective(objective)
        .build();

    // ── Executor ──────────────────────────────────────────────────────────────
    let mut executor = ForkserverExecutor::builder()
        .program(crun_binary)
        .arg(config_path.to_str().expect("config path not UTF-8"))
        .debug_child(false)
        .coverage_map_size(MAP_SIZE)
        .timeout(Duration::from_millis(1200))
        .kill_signal(Signal::SIGKILL)
        .build(tuple_list!(time_observer, edges_observer))
        .expect("ForkserverExecutor build failed");

    if let Some(dynamic_map_size) = executor.coverage_map_size() {
        executor.observers_mut()[&observer_ref]
            .as_mut()
            .truncate(dynamic_map_size);
    }

    // ── LiveCorpus for SpliceDelta (updated after every fuzz_one) ─────────────
    let r_seeds = rootfs_seeds(&bin_true);
    let live_corpus: LiveCorpus = Rc::new(RefCell::new(r_seeds.clone()));

    // ── Mutators ──────────────────────────────────────────────────────────────
    //
    // Config mutators (operate on CombinedInput.config via ConfigMutator wrapper):
    //   NautilusRandomMutator × 4  — random tree node replacements
    //   NautilusRecursionMutator   — recursive tree expansion
    //
    // Rootfs mutators (operate on CombinedInput.rootfs via RootfsMutator wrapper):
    //   ByteFlipFileContent, ReplaceFileContent, AddFileOp, RemoveOp,
    //   MutatePath, SpliceDelta (uses LiveCorpus), DestructiveMutator,
    //   UpdateExistingFile, ReplayWriteFile

    let mutators = tuple_list!(
        ConfigMutator::new(NautilusRandomMutator::new(context)),
        ConfigMutator::new(NautilusRandomMutator::new(context)),
        ConfigMutator::new(NautilusRandomMutator::new(context)),
        ConfigMutator::new(NautilusRandomMutator::new(context)),
        ConfigMutator::new(NautilusRecursionMutator::new(context)),
        ConfigMutator::new(NautilusSpliceMutator::new(context)),
        ConfigMutator::new(NautilusSpliceMutator::new(context)),
        ConfigMutator::new(NautilusSpliceMutator::new(context)),
        RootfsMutator::new(ByteFlipFileContent::new()),
        RootfsMutator::new(ReplaceFileContent::new()),
        RootfsMutator::new(AddFileOp::new()),
        RootfsMutator::new(RemoveOp::new()),
        RootfsMutator::new(MutatePath::with_baseline(
            baseline_file_paths.clone(),
            baseline_dir_paths.clone(),
            baseline_all_paths.clone(),
        )),
        RootfsMutator::new(SpliceDelta::new(live_corpus.clone())),
        RootfsMutator::new(DestructiveMutator::with_baseline(
            baseline_file_paths.clone(),
            baseline_dir_paths.clone(),
            baseline_all_paths.clone(),
        )),
        RootfsMutator::new(UpdateExistingFile::new(baseline_file_paths.clone())
            .with_baseline_contents(baseline_contents)),
        RootfsMutator::new(ReplayWriteFile::new(baseline_file_paths.clone())),
    );
    let scheduled   = HavocScheduledMutator::new(mutators);
    let havoc_stage = StdMutationalStage::new(scheduled);
    let mut stages  = tuple_list!(havoc_stage, afl_stats_stage);

    // ── Seed corpus ───────────────────────────────────────────────────────────
    //
    // Strategy: generate N Nautilus configs × empty rootfs, then pair the full
    // rootfs seed set with the first generated config.  This seeds both
    // mutation dimensions from the start without requiring cross-product explosion.

    let mut generator = NautilusGenerator::new(context);
    let mut initial_configs: Vec<NautilusInput> = (0..32)
        .filter_map(|_| generator.generate(&mut state).ok())
        .collect();
    if initial_configs.is_empty() {
        panic!("NautilusGenerator failed to produce any configs — check grammar.py");
    }
    let baseline_config = initial_configs[0].clone();

    // Configs × empty rootfs
    let mut seeds: Vec<CombinedInput> = initial_configs
        .drain(..)
        .map(|c| CombinedInput { config: c, rootfs: FsDelta::new(vec![]) })
        .collect();

    // Rootfs seeds × baseline config
    for r in r_seeds {
        seeds.push(CombinedInput { config: baseline_config.clone(), rootfs: r });
    }

    if state.must_load_initial_inputs() {
        for seed in &seeds {
            let _ = fuzzer.add_input(&mut state, &mut executor, &mut mgr, seed.clone());
        }
    }

    // Prime LiveCorpus and NautilusChunksMetadata from seeds that made it into the corpus.
    // NautilusChunksMetadata must be populated manually since NautilusFeedback can't be used
    // with CombinedInput corpus — this replicates what NautilusFeedback.append_metadata does.
    for idx in 0..state.corpus().count() {
        let cid = CorpusId::from(idx);
        if let Ok(input) = state.corpus().cloned_input_for_id(cid) {
            write_corpus_sidecar(&corpus_dir, idx, &input, context);
            live_corpus.borrow_mut().push(input.rootfs);
            if let Some(meta) = state.metadata_map_mut().get_mut::<NautilusChunksMetadata>() {
                meta.cks.add_tree(input.config.tree.clone(), &context.ctx);
            }
        }
    }

    println!("Corpus: {} seeds loaded", state.corpus().count());
    println!("Starting Campaign 3 fuzzing loop — Ctrl-C to stop");
    println!("  corpus  → {}/", corpus_dir.display());
    println!("  crashes → {}/", solutions_dir.display());
    println!("  config  → {}", config_path.display());
    println!("  stats   → fuzzer_stats, plot_data\n");

    // ── Fuzzing loop ──────────────────────────────────────────────────────────
    loop {
        let before = state.corpus().count();

        fuzzer
            .fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr)
            .expect("fuzz_one failed");

        mgr.maybe_report_progress(&mut state, Duration::from_secs(2))
            .expect("progress report failed");

        // Sync newly discovered corpus entries into LiveCorpus (for SpliceDelta)
        // and NautilusChunksMetadata (for NautilusSpliceMutator).
        let after = state.corpus().count();
        for idx in before..after {
            let cid = CorpusId::from(idx);
            if let Ok(input) = state.corpus().cloned_input_for_id(cid) {
                write_corpus_sidecar(&corpus_dir, idx, &input, context);
                live_corpus.borrow_mut().push(input.rootfs);
                if let Some(meta) = state.metadata_map_mut().get_mut::<NautilusChunksMetadata>() {
                    meta.cks.add_tree(input.config.tree.clone(), &context.ctx);
                }
            }
        }
    }
}
