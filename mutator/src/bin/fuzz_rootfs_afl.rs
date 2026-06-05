use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    str::FromStr,
    sync::Arc,
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
    symlink_mutators::{
        ExecutableSymlinkMutator, LoopAndDepthMutator, MountDestinationSymlinkMutator,
        MountOptionSymlinkMutator, ParentComponentSymlinkMutator, SymlinkEscapeMutator,
    },
    symlink_utils::{replace_with_symlink, BaselineIndex},
};

#[cfg(has_fuse3)]
use fs_mutator::ffi::{fuse_vfs_lib_init, fuse_vfs_lib_is_mounted, fuse_vfs_lib_run};

const MAP_SIZE: usize = 65536;

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
        // crun reads config from argv[1], not stdin; return a placeholder byte
        // so LibAFL's forkserver doesn't panic on a 0-length write.
        OwnedSlice::from(vec![0u8])
    }
}

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
    // Needed for initgroups("root", 0) when process.user.username is set.
    mkfile!(
        c"/etc/group",
        b"root:x:0:\ndaemon:x:1:\nbin:x:2:\nnobody:x:65534:\n"
    );
    mkfile!(c"/etc/hosts",    b"127.0.0.1 localhost\n::1 localhost\n");
    mkfile!(c"/etc/hostname", b"fuzz\n");
    mkfile!(c"/etc/resolv.conf", b"nameserver 8.8.8.8\n");
}

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

fn rootfs_seeds(bin_true: &[u8]) -> Vec<FsDelta> {
    let mut seeds = vec![

        FsDelta::new(vec![]),
        FsDelta::new(vec![FsOp::rmdir("/proc")]),
        FsDelta::new(vec![FsOp::rmdir("/dev")]),
        FsDelta::new(vec![FsOp::rmdir("/sys")]),
        FsDelta::new(vec![FsOp::rmdir("/tmp")]),
        FsDelta::new(vec![FsOp::rmdir("/proc"), FsOp::rmdir("/sys")]),
        FsDelta::new(vec![FsOp::rmdir("/dev"),  FsOp::rmdir("/tmp")]),
        FsDelta::new(vec![
            FsOp::rmdir("/proc"), FsOp::rmdir("/dev"),
            FsOp::rmdir("/sys"),  FsOp::rmdir("/tmp"),
        ]),

        FsDelta::new(vec![FsOp::delete_file("/bin/true")]),
        FsDelta::new(vec![FsOp::truncate("/bin/true", 0)]),
        FsDelta::new(vec![FsOp::truncate("/bin/true", 4)]),
        FsDelta::new(vec![FsOp::truncate("/bin/true", 16)]),
        FsDelta::new(vec![FsOp::truncate("/bin/true", 64)]),
        FsDelta::new(vec![FsOp::update_file("/bin/true", b"not an elf\n".to_vec())]),
        FsDelta::new(vec![FsOp::update_file("/bin/true", b"\x7fELF\x01\x01\x01\x00".to_vec())]),
        FsDelta::new(vec![FsOp::update_file("/bin/true", b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec())]),
        FsDelta::new(vec![FsOp::update_file("/bin/true", b"#!/bin/sh\nexit 0\n".to_vec())]),
        FsDelta::new(vec![FsOp::delete_file("/bin/true"), FsOp::rmdir("/bin")]),

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
        FsDelta::new(vec![
            FsOp::create_file("/bin/sh",   b"\x7fELF\x02\x01\x01\x00".to_vec()),
            FsOp::create_file("/bin/bash", b"\x7fELF\x02\x01\x01\x00".to_vec()),
            FsOp::create_file("/bin/ls",   b"\x7fELF\x02\x01\x01\x00".to_vec()),
        ]),

        FsDelta::new(vec![
            FsOp::create_file("/etc/group",   b"root:x:0:\n".to_vec()),
            FsOp::create_file("/.dockerenv",  b"".to_vec()),
            FsOp::mkdir("/usr"),
            FsOp::mkdir("/lib"),
            FsOp::rmdir("/proc"),
        ]),
        FsDelta::new(vec![
            FsOp::create_file("/etc/group",   b"root:x:0:\n".to_vec()),
            FsOp::mkdir("/usr"),
            FsOp::mkdir("/lib"),
            FsOp::update_file("/bin/true", b"\x7fELF\x01\x01\x01".to_vec()),
        ]),
        FsDelta::new(vec![
            FsOp::create_file("/etc/group",   b"root:x:0:\n".to_vec()),
            FsOp::mkdir("/usr"),
            FsOp::mkdir("/lib"),
            FsOp::delete_file("/bin/true"),
        ]),

        FsDelta::new(vec![FsOp::delete_file("/etc/passwd")]),
        FsDelta::new(vec![FsOp::delete_file("/etc/hosts")]),
        FsDelta::new(vec![
            FsOp::delete_file("/etc/passwd"),
            FsOp::delete_file("/etc/hosts"),
            FsOp::delete_file("/etc/hostname"),
            FsOp::delete_file("/etc/resolv.conf"),
        ]),

        FsDelta::new(vec![FsOp::create_file("/.dockerenv", b"".to_vec())]),
        FsDelta::new(vec![FsOp::create_file("/etc/ld.so.cache", b"\x00\x01".to_vec())]),
        FsDelta::new(vec![
            FsOp::create_file("/proc/mounts", b"proc /proc proc rw 0 0\n".to_vec()),
            FsOp::create_file("/proc/self",   b"".to_vec()),
        ]),
    ];

    if bin_true.len() > 8 {
        let mut c4 = bin_true.to_vec();
        c4[4] ^= 0xff;
        seeds.push(FsDelta::new(vec![FsOp::update_file("/bin/true", c4)]));

        let mut c5 = bin_true.to_vec();
        c5[5] ^= 0xff;
        seeds.push(FsDelta::new(vec![FsOp::update_file("/bin/true", c5)]));

        seeds.push(FsDelta::new(vec![FsOp::update_file(
            "/bin/true",
            bin_true[..128.min(bin_true.len())].to_vec(),
        )]));

        seeds.push(FsDelta::new(vec![FsOp::update_file(
            "/bin/true",
            bin_true[..64.min(bin_true.len())].to_vec(),
        )]));
    }

    seeds
}

fn rootfs_symlink_seeds(index: &BaselineIndex) -> Vec<FsDelta> {
    let mut seeds = Vec::new();

    for (path, target) in &[
        ("/proc", "../../proc"),
        ("/dev",  "../../dev"),
        ("/sys",  "../../sys"),
        ("/tmp",  "../../tmp"),
    ] {
        seeds.push(FsDelta::new(replace_with_symlink(path, target, index)));
    }

    let mut combined = Vec::new();
    for (path, target) in &[("/proc", "../../proc"), ("/dev", "../../dev"), ("/sys", "../../sys")] {
        combined.extend(replace_with_symlink(path, target, index));
    }
    if !combined.is_empty() {
        seeds.push(FsDelta::new(combined));
    }

    for (path, target) in &[
        ("/proc", "/proc"),
        ("/dev",  "/proc/self/fd"),
        ("/sys",  "/sys"),
        ("/proc", "/proc/self/exe"),
    ] {
        seeds.push(FsDelta::new(replace_with_symlink(path, target, index)));
    }

    seeds.push(FsDelta::new(replace_with_symlink("/proc", "/etc/passwd", index)));
    seeds.push(FsDelta::new(replace_with_symlink("/dev",  "/bin/true",   index)));

    seeds.push(FsDelta::new(replace_with_symlink("/proc", "/nonexistent", index)));
    seeds.push(FsDelta::new(replace_with_symlink("/dev",  "/missing",     index)));

    for (path, target) in &[
        ("/etc", "../../etc"),
        ("/bin", "../../bin"),
        ("/lib", "../../lib"),
        ("/usr", "../../usr"),
        ("/dev", "/proc/self/fd"),
    ] {
        seeds.push(FsDelta::new(replace_with_symlink(path, target, index)));
    }

    for target in &[
        "/proc/self/exe",
        "/proc/self/mem",
        "/proc/self/fd/0",
        "/dev/zero",
        "/dev/null",
        "../../usr/bin/python3",
    ] {
        seeds.push(FsDelta::new(replace_with_symlink("/bin/true", target, index)));
    }

    seeds.push(FsDelta::new(replace_with_symlink("/etc/passwd", "../../etc/passwd",    index)));
    seeds.push(FsDelta::new(replace_with_symlink("/etc/passwd", "/etc/passwd",         index)));
    seeds.push(FsDelta::new(replace_with_symlink("/etc/passwd", "../../../etc/shadow", index)));
    seeds.push(FsDelta::new(replace_with_symlink("/etc/group",  "../../etc/group",     index)));

    seeds.push(FsDelta::new(vec![FsOp::create_symlink("/loop", "/loop")]));
    seeds.push(FsDelta::new(vec![
        FsOp::create_symlink("/a", "/b"),
        FsOp::create_symlink("/b", "/c"),
        FsOp::create_symlink("/c", "/a"),
    ]));

    seeds.push(FsDelta::new(replace_with_symlink("/etc/passwd", "../../../etc/shadow", index)));
    seeds.push(FsDelta::new(vec![
        FsOp::create_symlink("/bin/sh", "../../../proc/sysrq-trigger"),
    ]));

    seeds.push(FsDelta::new(vec![FsOp::create_symlink("/bin/sh", "/nonexistent")]));
    seeds.push(FsDelta::new(replace_with_symlink("/proc", "/nonexistent", index)));

    seeds.push(FsDelta::new(vec![FsOp::create_symlink("/bin/x", "////proc//self//exe")]));
    seeds.push(FsDelta::new(vec![FsOp::create_symlink("/bin/x", "../../../proc/./self/./exe")]));

    seeds.retain(|d| !d.is_empty());
    seeds
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <crun-afl-binary>", args[0]);
        std::process::exit(1);
    }
    let crun_binary = &args[1];
    let pid = std::process::id();

    let cwd = std::env::current_dir().expect("cannot determine CWD");
    let corpus_dir    = cwd.join("corpus");
    let solutions_dir = cwd.join("crashes");
    let mountpoint    = format!("/tmp/campaign2-fuse-{pid}");

    for d in &[&corpus_dir, &solutions_dir] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::create_dir_all(&mountpoint).expect("failed to create FUSE mountpoint");

    let bin_true: Vec<u8> = std::fs::read("/bin/true")
        .or_else(|_| std::fs::read("/usr/bin/true"))
        .unwrap_or_default();

    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");

    let fuse_rootfs = mountpoint.clone();
    unsafe { init_vfs(vfs, &bin_true) };
    unsafe { vfs_save_snapshot(vfs) };

    let baseline_file_paths = enumerate_vfs_file_paths(vfs);
    let baseline_dir_paths  = enumerate_vfs_dir_paths(vfs);
    let baseline_all_paths  = enumerate_vfs_all_paths(vfs);
    let baseline_index      = Arc::new(BaselineIndex::build(vfs));

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

    start_fuse(vfs, &mountpoint);

    let config_path = std::env::current_dir()
        .expect("no CWD")
        .join("config.json");
    let config_bytes = make_fixed_config(&fuse_rootfs);
    std::fs::write(&config_path, &config_bytes).expect("failed to write config.json");

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

    let monitor = SimpleMonitor::new(|s| {
        println!("{s}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    });
    let mut mgr = SimpleEventManager::new(monitor);

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

    let mut seeds = rootfs_seeds(&bin_true);
    seeds.extend(rootfs_symlink_seeds(&baseline_index));
    let live_corpus: LiveCorpus = Rc::new(RefCell::new(seeds.clone()));

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
        MountDestinationSymlinkMutator::new(Arc::clone(&baseline_index)),
        MountDestinationSymlinkMutator::new(Arc::clone(&baseline_index)),
        MountOptionSymlinkMutator::new(Arc::clone(&baseline_index)),
        ExecutableSymlinkMutator::new(Arc::clone(&baseline_index)),
        ParentComponentSymlinkMutator::new(Arc::clone(&baseline_index)),
        SymlinkEscapeMutator::new(Arc::clone(&baseline_index)),
        LoopAndDepthMutator::new(),
    );
    let scheduled = HavocScheduledMutator::new(mutators);
    let havoc_stage = StdMutationalStage::new(scheduled);
    let mut stages = tuple_list!(havoc_stage, afl_stats_stage);

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
