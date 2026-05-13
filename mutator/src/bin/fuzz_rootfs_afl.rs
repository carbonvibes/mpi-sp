//! fuzz_rootfs_afl — Campaign 2: FUSE rootfs mutation + AFL ForkserverExecutor.
//!
//! The config.json is fixed (baseline, never mutated).
//! Only the rootfs (FUSE VFS, via FsDelta) is mutated each iteration.
//! Coverage is measured via AFL shared memory from the afl-clang-lto crun binary.
//!
//! Run (as root, from /tmp/campaign2/):
//!   mkdir -p /tmp/campaign2
//!   cd /tmp/campaign2
//!   sudo /path/to/fuzz_rootfs_afl /path/to/crun-afl/crun 2>&1 | tee /tmp/campaign2_fuzz.log

use std::{
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
    feedback_and_fast,
    feedbacks::{CrashFeedback, MaxMapFeedback, TimeFeedback},
    fuzzer::{Evaluator, Fuzzer},
    inputs::ToTargetBytes,
    monitors::SimpleMonitor,
    mutators::HavocScheduledMutator,
    observers::{CanTrack, HitcountsMapObserver, StdMapObserver, TimeObserver},
    schedulers::QueueScheduler,
    stages::{AflStatsStage, StdMutationalStage},
    state::{HasCorpus, StdState},
};
use libafl::feedback_or;
use libafl_bolts::{
    AsSliceMut, StdTargetArgs, Truncate, current_nanos,
    ownedref::OwnedSlice,
    rands::StdRand,
    shmem::{ShMem, ShMemProvider, UnixShMemProvider},
    tuples::{Handled, tuple_list},
};
use nix::sys::signal::Signal;

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

// ── FsDeltaConverter ─────────────────────────────────────────────────────────
// Applied by StdFuzzer before each forkserver execution.
// Resets VFS to baseline snapshot, applies the FsDelta (rootfs mutation),
// then returns empty bytes — crun reads from the fixed config file (argv[1]),
// not from stdin.

struct FsDeltaConverter {
    vfs: *mut VfsT,
}

// VfsT is an opaque C pointer; we access it only from the fuzzing thread.
unsafe impl Send for FsDeltaConverter {}
unsafe impl Sync for FsDeltaConverter {}

impl ToTargetBytes<FsDelta> for FsDeltaConverter {
    fn to_target_bytes<'a>(&mut self, input: &'a FsDelta) -> OwnedSlice<'a, u8> {
        unsafe { vfs_reset_to_snapshot(self.vfs) };
        let _ = apply_delta(self.vfs, input);
        // crun reads config from argv[1], not stdin — return a placeholder byte
        // so LibAFL's forkserver doesn't panic writing a 0-length buffer.
        OwnedSlice::from(vec![0u8])
    }
}

// ── FUSE startup ──────────────────────────────────────────────────────────────

#[cfg(has_fuse3)]
fn start_fuse(vfs: *mut VfsT, mountpoint: &str) {
    unsafe { fuse_vfs_lib_init(vfs) };
    let mp = std::ffi::CString::new(mountpoint).expect("mountpoint nul");
    thread::spawn(move || unsafe { fuse_vfs_lib_run(mp.as_ptr()) });

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if unsafe { fuse_vfs_lib_is_mounted() } != 0 {
            break;
        }
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

// ── VFS baseline ──────────────────────────────────────────────────────────────

unsafe fn init_vfs(vfs: *mut VfsT, bin_true: &[u8]) {
    for dir in &[
        c"/bin",
        c"/proc",
        c"/dev",
        c"/sys",
        c"/tmp",
        c"/etc",
        c"/var",
        c"/run",
    ] {
        vfs_mkdir(vfs, dir.as_ptr());
    }

    if !bin_true.is_empty() {
        vfs_create_file(
            vfs,
            c"/bin/true".as_ptr(),
            bin_true.as_ptr(),
            bin_true.len(),
        );
    }

    macro_rules! mkfile {
        ($path:expr, $content:expr) => {
            vfs_create_file(vfs, $path.as_ptr(), $content.as_ptr(), $content.len())
        };
    }
    mkfile!(
        c"/etc/passwd",
        b"root:x:0:0:root:/root:/bin/sh\nnobody:x:65534:65534:nobody:/:/usr/sbin/nologin\n"
    );
    // Required for initgroups("root", 0) called when process.user.username is set
    mkfile!(
        c"/etc/group",
        b"root:x:0:\ndaemon:x:1:\nbin:x:2:\nnobody:x:65534:\n"
    );
    mkfile!(c"/etc/hosts",    b"127.0.0.1 localhost\n::1 localhost\n");
    mkfile!(c"/etc/hostname", b"fuzz\n");
    mkfile!(c"/etc/resolv.conf", b"nameserver 8.8.8.8\n");
}

// ── Fixed config.json ─────────────────────────────────────────────────────────

fn make_fixed_config(rootfs_path: &str) -> Vec<u8> {
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
            "namespaces": [
                {"type": "pid"},
                {"type": "mount"}
            ]
        }
    })
    .to_string()
    .into_bytes()
}

// ── Seed corpus ───────────────────────────────────────────────────────────────

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
        FsDelta::new(vec![FsOp::update_file("/bin/true", b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec())]),
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
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <crun-afl-binary>", args[0]);
        eprintln!("  Run from /tmp/campaign2/ as root.");
        eprintln!("  Example: cd /tmp/campaign2 && sudo {} /path/to/crun-afl/crun", args[0]);
        std::process::exit(1);
    }
    let crun_binary = &args[1];
    let pid = std::process::id();

    println!("=== fuzz_rootfs_afl: Campaign 2 — FUSE rootfs + AFL ForkserverExecutor ===");
    println!("  crun binary : {crun_binary}");

    let cwd = std::env::current_dir().expect(
        "cannot determine CWD — make sure /tmp/campaign2 exists and run: cd /tmp/campaign2"
    );
    let corpus_dir    = cwd.join("corpus");
    let solutions_dir = cwd.join("crashes");
    let mountpoint    = format!("/tmp/campaign2-fuse-{pid}");

    for d in &[&corpus_dir, &solutions_dir] {
        std::fs::create_dir_all(d).unwrap_or_else(|e| {
            eprintln!("ERROR: cannot create {}: {e}", d.display());
            eprintln!("  Run first: mkdir -p /tmp/campaign2/corpus /tmp/campaign2/crashes");
            eprintln!("  Then:      cd /tmp/campaign2");
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

    let fuse_rootfs = mountpoint.clone();
    unsafe { init_vfs(vfs, &bin_true) };
    unsafe { vfs_save_snapshot(vfs) };

    let baseline_file_paths = enumerate_vfs_file_paths(vfs);
    let baseline_dir_paths  = enumerate_vfs_dir_paths(vfs);
    let baseline_all_paths  = enumerate_vfs_all_paths(vfs);

    let baseline_contents: Vec<(String, Vec<u8>)> = {
        let mut c = vec![
            ("/etc/passwd".to_string(),
             b"root:x:0:0:root:/root:/bin/sh\n".to_vec()),
        ];
        if !bin_true.is_empty() {
            c.push(("/bin/true".to_string(), bin_true.clone()));
        }
        c
    };

    println!(
        "Baseline VFS: {} file(s), {} dir(s)",
        baseline_file_paths.len(),
        baseline_dir_paths.len(),
    );

    start_fuse(vfs, &mountpoint);

    // ── Fixed config.json ─────────────────────────────────────────────────────
    // Written once to disk; crun reads it via argv[1] on every iteration.
    let config_path = std::env::current_dir()
        .expect("no CWD")
        .join("config.json");
    let config_bytes = make_fixed_config(&fuse_rootfs);
    std::fs::write(&config_path, &config_bytes).expect("failed to write config.json");
    println!("  config      : {}", config_path.display());
    println!("  rootfs      : {fuse_rootfs}");

    // ── AFL shared memory (coverage map) ─────────────────────────────────────
    let mut shmem_provider = UnixShMemProvider::new().unwrap();
    let mut shmem = shmem_provider.new_shmem(MAP_SIZE).unwrap();
    unsafe { shmem.write_to_env("__AFL_SHM_ID").unwrap() };
    let shmem_buf = shmem.as_slice_mut();

    let edges_observer = unsafe {
        HitcountsMapObserver::new(StdMapObserver::new("shared_mem", shmem_buf)).track_indices()
    };
    let time_observer = TimeObserver::new("time");

    let map_feedback = MaxMapFeedback::new(&edges_observer);
    let tokens = libafl::mutators::Tokens::new();

    // ── AflStatsStage (writes fuzzer_stats + plot_data to CWD) ───────────────
    let afl_stats_stage = AflStatsStage::builder()
        .stats_file(PathBuf::from_str("fuzzer_stats").unwrap())
        .plot_file(PathBuf::from_str("plot_data").unwrap())
        .report_interval(Duration::from_secs(15))
        .map_feedback(&map_feedback)
        .tokens(&tokens)
        .banner("fuzz-rootfs-afl".into())
        .version("0.1.0".to_string())
        .exec_timeout(2)
        .build()
        .expect("AflStatsStage build failed");

    // ── Feedbacks + state ─────────────────────────────────────────────────────
    let mut feedback = libafl::feedback_or!(
        MaxMapFeedback::new(&edges_observer),
        TimeFeedback::new(&time_observer),
    );
    let mut objective = feedback_and_fast!(
        CrashFeedback::new(),
        MaxMapFeedback::with_name("mapfeedback_metadata_objective", &edges_observer),
    );

    let mut state = StdState::new(
        StdRand::with_seed(current_nanos()),
        OnDiskCorpus::<FsDelta>::new(&corpus_dir).expect("corpus dir"),
        OnDiskCorpus::<FsDelta>::new(&solutions_dir).expect("solutions dir"),
        &mut feedback,
        &mut objective,
    )
    .expect("StdState");

    state.add_metadata(tokens.clone());

    // ── Monitor + event manager ───────────────────────────────────────────────
    let monitor = SimpleMonitor::new(|s| {
        println!("{s}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    });
    let mut mgr = SimpleEventManager::new(monitor);

    // ── Scheduler + fuzzer ────────────────────────────────────────────────────
    let observer_ref = edges_observer.handle();

    let scheduler = QueueScheduler::new();

    let converter = FsDeltaConverter { vfs };

    let mut fuzzer = StdFuzzerBuilder::new()
        .input_filter(BloomInputFilter::default())
        .target_bytes_converter(converter)
        .scheduler(scheduler)
        .feedback(feedback)
        .objective(objective)
        .build();

    // ── ForkserverExecutor ────────────────────────────────────────────────────
    let mut executor = ForkserverExecutor::builder()
        .program(crun_binary)
        .arg(config_path.to_str().expect("config path not UTF-8"))
        .debug_child(false)
        .coverage_map_size(MAP_SIZE)
        .timeout(Duration::from_millis(1200))
        .kill_signal(Signal::SIGKILL)
        .build(tuple_list!(time_observer, edges_observer))
        .expect("ForkserverExecutor build failed");

    // Truncate coverage observer to the binary's actual edge count
    if let Some(dynamic_map_size) = executor.coverage_map_size() {
        executor.observers_mut()[&observer_ref]
            .as_mut()
            .truncate(dynamic_map_size);
    }

    // ── Seed corpus ───────────────────────────────────────────────────────────
    let seeds = rootfs_seeds(&bin_true);
    let live_corpus: LiveCorpus = Rc::new(RefCell::new(seeds.clone()));

    // ── Mutators ──────────────────────────────────────────────────────────────
    let mutators = tuple_list!(
        ByteFlipFileContent::new(),
        ReplaceFileContent::new(),
        AddFileOp::new(),
        RemoveOp::new(),
        MutatePath::with_baseline(
            baseline_file_paths.clone(),
            baseline_dir_paths.clone(),
            baseline_all_paths.clone(),
        ),
        SpliceDelta::new(live_corpus.clone()),
        DestructiveMutator::with_baseline(
            baseline_file_paths.clone(),
            baseline_dir_paths.clone(),
            baseline_all_paths.clone(),
        ),
        UpdateExistingFile::new(baseline_file_paths.clone())
            .with_baseline_contents(baseline_contents),
        ReplayWriteFile::new(baseline_file_paths.clone()),
    );
    let scheduled = HavocScheduledMutator::new(mutators);
    let havoc_stage = StdMutationalStage::new(scheduled);
    let mut stages = tuple_list!(havoc_stage, afl_stats_stage);

    // ── Seed loading ──────────────────────────────────────────────────────────
    if state.must_load_initial_inputs() {
        for delta in &seeds {
            fuzzer
                .add_input(&mut state, &mut executor, &mut mgr, delta.clone())
                .expect("failed to add seed");
        }
    }

    // Prime live_corpus with seeds that made it into corpus.
    // OnDiskCorpus drops input from memory on add(), so load from disk.
    for idx in 0..state.corpus().count() {
        let cid = CorpusId::from(idx);
        if let Ok(input) = state.corpus().cloned_input_for_id(cid) {
            live_corpus.borrow_mut().push(input);
        }
    }

    println!("Corpus: {} seeds loaded", state.corpus().count());
    println!("Starting fuzzing loop — Ctrl-C to stop");
    println!("  corpus  → {}/", corpus_dir.display());
    println!("  crashes → {}/", solutions_dir.display());
    println!("  stats   → fuzzer_stats, plot_data\n");

    // ── Fuzzing loop ──────────────────────────────────────────────────────────
    loop {
        let before = state.corpus().count();

        fuzzer
            .fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr)
            .expect("fuzz_one failed");

        mgr.maybe_report_progress(&mut state, Duration::from_secs(2))
            .expect("progress report failed");

        // Sync any newly-found corpus entries into live_corpus for SpliceDelta.
        let after = state.corpus().count();
        for idx in before..after {
            let cid = CorpusId::from(idx);
            if let Ok(input) = state.corpus().cloned_input_for_id(cid) {
                live_corpus.borrow_mut().push(input);
            }
        }
    }
}
