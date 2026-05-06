//! fuzz_crun — in-process crun fuzzer with real SanCov coverage.
//!
//! Unlike fuzz_runc (which spawns /usr/bin/crun as a subprocess), this binary
//! links libcrun.a directly and calls fuzz_crun_run_container() in-process.
//!
//! Coverage:
//!   libcrun.a is compiled with -fsanitize-coverage=trace-pc-guard,trace-cmp.
//!   Every code path inside crun (JSON parsing via libocispec/yajl, OCI spec
//!   validation, namespace setup, rootfs checks) updates EDGES_MAP in our
//!   process — MaxMapFeedback sees real coverage and guides mutations.
//!
//! Throughput vs fuzz_runc:
//!   No fork/exec overhead for crun itself; the container child still forks
//!   (executing /bin/true), but that fork is cheap and non-blocking.
//!   Expect 200–2000 exec/sec depending on host.
//!   nothing 
//! Usage:
//!   cargo run --release --bin fuzz_crun 2>&1 | tee /tmp/crun_fuzz.log
//!   cargo run --release --bin fuzz_crun -- --dry-run 20


use std::{
    cell::RefCell,
    ffi::CString,
    io::Write,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use libafl::observers::{HitcountsMapObserver, StdMapObserver};
use libafl::{
    corpus::{Corpus, CorpusId, OnDiskCorpus},
    events::{ProgressReporter, SimpleEventManager},
    executors::{ExitKind, InProcessExecutor},
    feedbacks::{CrashFeedback, MaxMapFeedback},
    fuzzer::{Evaluator, Fuzzer, StdFuzzer},
    monitors::SimpleMonitor,
    mutators::HavocScheduledMutator,
    schedulers::QueueScheduler,
    stages::StdMutationalStage,
    state::{HasCorpus, StdState},
};
use libafl_bolts::{current_nanos, rands::StdRand, tuples::tuple_list};
use libafl_targets::coverage::MAX_EDGES_FOUND;
use libafl_targets::{EDGES_MAP, EDGES_MAP_DEFAULT_SIZE};

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

// ── libcrun FFI ───────────────────────────────────────────────────────────────
// #[link] attributes here are PER-BINARY — they do not affect vfs_bench, fuzz,
// or any other binary in the package.  build.rs only emits cargo:rustc-link-lib
// for crun_harness.a (the thin wrapper, no SanCov); the SanCov-instrumented
// static archives are linked exclusively through these attributes so that
// non-fuzzing binaries never see the SanCov symbol requirements.

#[cfg_attr(has_libcrun, link(name = "crun", kind = "static"))]
#[cfg_attr(has_libcrun, link(name = "ocispec", kind = "static"))]
// libyajl: prefer bundled static build; fall back to system dylib.
#[cfg_attr(has_bundled_yajl, link(name = "yajl", kind = "static"))]
#[cfg_attr(all(has_libcrun, not(has_bundled_yajl)), link(name = "yajl"))]
// System libraries crun requires at link time.
#[cfg_attr(has_libcrun, link(name = "cap"))]
#[cfg_attr(has_libcrun, link(name = "seccomp"))]
#[cfg_attr(has_libcrun, link(name = "systemd"))]
extern "C" {
    /// Run one container iteration in-process via libcrun.
    /// config_json: NUL-terminated OCI config.json (root.path already set).
    /// state_root:  crun state directory (unique per fuzzer instance).
    /// id:          unique container ID per iteration.
    /// Returns 0 on success, -1 on parse/validation error, container exit code otherwise.
    fn fuzz_crun_run_container(
        config_json: *const std::os::raw::c_char,
        state_root: *const std::os::raw::c_char,
        id: *const std::os::raw::c_char,
    ) -> std::os::raw::c_int;
}

static ITER: AtomicU64 = AtomicU64::new(0);

// ── FUSE startup ──────────────────────────────────────────────────────────────

#[cfg(has_fuse3)]
fn start_fuse(vfs: *mut VfsT, mountpoint: &str) {
    unsafe { fuse_vfs_lib_init(vfs) };
    let mp = CString::new(mountpoint).expect("mountpoint contains nul byte");
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

unsafe fn init_vfs(vfs: *mut VfsT, config_bytes: &[u8], bin_true: &[u8]) {
    vfs_create_file(
        vfs,
        c"/config.json".as_ptr(),
        config_bytes.as_ptr(),
        config_bytes.len(),
    );

    for dir in &[
        c"/rootfs",
        c"/rootfs/bin",
        c"/rootfs/proc",
        c"/rootfs/dev",
        c"/rootfs/sys",
        c"/rootfs/tmp",
        c"/rootfs/etc",
        c"/rootfs/var",
        c"/rootfs/run",
    ] {
        vfs_mkdir(vfs, dir.as_ptr());
    }

    if !bin_true.is_empty() {
        vfs_create_file(
            vfs,
            c"/rootfs/bin/true".as_ptr(),
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
        c"/rootfs/etc/passwd",
        b"root:x:0:0:root:/root:/bin/sh\nnobody:x:65534:65534:nobody:/:/usr/sbin/nologin\n"
    );
    mkfile!(
        c"/rootfs/etc/hosts",
        b"127.0.0.1 localhost\n::1 localhost\n"
    );
    mkfile!(c"/rootfs/etc/hostname", b"fuzz\n");
    mkfile!(c"/rootfs/etc/resolv.conf", b"nameserver 8.8.8.8\n");
}

// ── config.json helpers ───────────────────────────────────────────────────────

fn make_baseline_config(uid: u32, gid: u32, rootfs_path: &str) -> Vec<u8> {
    serde_json::json!({
        "ociVersion": "1.0.0",
        "process": {
            "terminal": false,
            "user": {"uid": 0, "gid": 0},
            "args": ["/bin/true"],
            "env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
            "cwd": "/",
            "noNewPrivileges": true
        },
        "root": {"path": rootfs_path, "readonly": false},
        "hostname": "fuzz",
        "linux": {
            "uidMappings": [{"containerID": 0, "hostID": uid, "size": 1}],
            "gidMappings": [{"containerID": 0, "hostID": gid, "size": 1}],
            "namespaces": [
                {"type": "pid"}, {"type": "mount"}, {"type": "user"}
            ]
        }
    })
    .to_string()
    .into_bytes()
}

/// Patch root.path in a (possibly mutated) config.json byte slice.
/// Valid JSON → patch root.path so crun always finds the FUSE rootfs.
/// Invalid JSON → return as-is so crun's own parser is the fuzzing target.
fn patch_root_path(config_bytes: &[u8], rootfs_path: &str) -> Vec<u8> {
    match serde_json::from_slice::<serde_json::Value>(config_bytes) {
        Ok(mut v) => {
            if let Some(root) = v.get_mut("root") {
                if let Some(obj) = root.as_object_mut() {
                    obj.insert(
                        "path".to_string(),
                        serde_json::Value::String(rootfs_path.to_string()),
                    );
                }
            }
            serde_json::to_vec(&v).unwrap_or_else(|_| config_bytes.to_vec())
        }
        Err(_) => config_bytes.to_vec(),
    }
}

// ── Seed corpus ───────────────────────────────────────────────────────────────

fn config_seeds(uid: u32, gid: u32) -> Vec<FsDelta> {
    let ns = serde_json::json!([{"type":"pid"},{"type":"mount"},{"type":"user"}]);
    let uid_maps = serde_json::json!([{"containerID":0,"hostID":uid,"size":1}]);
    let gid_maps = serde_json::json!([{"containerID":0,"hostID":gid,"size":1}]);
    let root = serde_json::json!({"path":"PLACEHOLDER","readonly":false});
    let proc_min = serde_json::json!({
        "terminal":false,"user":{"uid":0,"gid":0},
        "args":["/bin/true"],"env":["PATH=/bin"],"cwd":"/","noNewPrivileges":true
    });
    let linux = serde_json::json!({
        "namespaces": ns, "uidMappings": uid_maps, "gidMappings": gid_maps
    });

    let to_delta = |v: serde_json::Value| -> FsDelta {
        FsDelta::new(vec![FsOp::update_file(
            "/config.json",
            v.to_string().into_bytes(),
        )])
    };

    vec![
        // 1. Minimal valid spec (pid + mount + user ns only)
        to_delta(serde_json::json!({"ociVersion":"1.0.0","process":proc_min,
            "root":root,"hostname":"fuzz","linux":linux})),
        // 2. Read-only rootfs
        to_delta(serde_json::json!({"ociVersion":"1.0.0","process":proc_min,
            "root":{"path":"PLACEHOLDER","readonly":true},"hostname":"fuzz","linux":linux})),
        // 3. Elevated capabilities — large bounding set
        to_delta(serde_json::json!({"ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"args":["/bin/true"],
                "env":["PATH=/bin"],"cwd":"/",
                "capabilities":{
                    "bounding": ["CAP_CHOWN","CAP_DAC_OVERRIDE","CAP_DAC_READ_SEARCH",
                                 "CAP_FOWNER","CAP_FSETID","CAP_KILL","CAP_NET_BIND_SERVICE",
                                 "CAP_NET_RAW","CAP_SETGID","CAP_SETUID","CAP_SETPCAP",
                                 "CAP_SYS_CHROOT","CAP_SYS_PTRACE"],
                    "effective":  ["CAP_KILL","CAP_NET_BIND_SERVICE"],
                    "permitted":  ["CAP_KILL","CAP_NET_BIND_SERVICE"],
                    "inheritable":[],
                    "ambient":    []
                }},
            "root":root,"hostname":"fuzz","linux":linux})),
        // 5. Empty capabilities — exercises empty cap set validation
        to_delta(serde_json::json!({"ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"args":["/bin/true"],
                "env":["PATH=/bin"],"cwd":"/",
                "capabilities":{
                    "bounding":[],"effective":[],"permitted":[],
                    "inheritable":[],"ambient":[]
                }},
            "root":root,"hostname":"fuzz","linux":linux})),
        // 6. Extra rlimits — exercises rlimit setup codepath
        to_delta(serde_json::json!({"ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"args":["/bin/true"],
                "env":["PATH=/bin"],"cwd":"/",
                "rlimits":[
                    {"type":"RLIMIT_NOFILE","hard":1024,"soft":1024},
                    {"type":"RLIMIT_NPROC","hard":512,"soft":512},
                    {"type":"RLIMIT_AS","hard":536870912,"soft":536870912}
                ]},
            "root":root,"hostname":"fuzz","linux":linux})),
        // 7. noNewPrivileges = false
        to_delta(serde_json::json!({"ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"args":["/bin/true"],
                "env":["PATH=/bin"],"cwd":"/","noNewPrivileges":false},
            "root":root,"hostname":"fuzz","linux":linux})),
        // 8. Annotations block
        to_delta(serde_json::json!({"ociVersion":"1.0.0","process":proc_min,
            "root":root,"hostname":"fuzz",
            "annotations":{"com.example.key":"value"},
            "linux":linux})),
        // 9. Empty args — exercises arg-validation path
        to_delta(serde_json::json!({"ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"args":[],
                "env":["PATH=/bin"],"cwd":"/"},
            "root":root,"hostname":"fuzz","linux":linux})),
        // 10. Non-existent binary — exercises execve error path in crun
        to_delta(serde_json::json!({"ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},
                "args":["/bin/nonexistent"],"env":["PATH=/bin"],"cwd":"/"},
            "root":root,"hostname":"fuzz","linux":linux})),
        // 11. Incomplete OCI spec (valid JSON, missing required fields) — exercises
        //     libocispec validation error path (not yajl parse error)
        to_delta(serde_json::json!({"ociVersion":"1.0.0"})),
    ]
}

fn rootfs_seeds(bin_true: &[u8]) -> Vec<FsDelta> {
    let mut seeds = vec![
        FsDelta::new(vec![FsOp::rmdir("/rootfs/proc")]),
        FsDelta::new(vec![FsOp::rmdir("/rootfs/dev")]),
        FsDelta::new(vec![FsOp::rmdir("/rootfs/sys")]),
        FsDelta::new(vec![FsOp::delete_file("/rootfs/bin/true")]),
        FsDelta::new(vec![FsOp::update_file(
            "/rootfs/bin/true",
            b"not an elf".to_vec(),
        )]),
        FsDelta::new(vec![FsOp::update_file(
            "/rootfs/bin/true",
            b"\x7fELF".to_vec(),
        )]),
        FsDelta::new(vec![FsOp::truncate("/rootfs/bin/true", 0)]),
        FsDelta::new(vec![FsOp::delete_file("/rootfs/etc/passwd")]),
        FsDelta::new(vec![FsOp::create_file("/rootfs/.dockerenv", b"".to_vec())]),
    ];
    if bin_true.len() > 4 {
        let mut corrupt = bin_true.to_vec();
        corrupt[4] ^= 0xff;
        seeds.push(FsDelta::new(vec![FsOp::update_file(
            "/rootfs/bin/true",
            corrupt,
        )]));
    }
    seeds
}

// ── CLI ───────────────────────────────────────────────────────────────────────

struct Args {
    dry_run: Option<u64>,
}

impl Args {
    fn parse() -> Self {
        let argv: Vec<String> = std::env::args().collect();
        let mut dry_run = None;
        let mut i = 1;
        while i < argv.len() {
            if argv[i] == "--dry-run" {
                let n = argv.get(i + 1).and_then(|s| s.parse::<u64>().ok());
                if n.is_some() {
                    i += 1;
                }
                dry_run = Some(n.unwrap_or(20));
            }
            i += 1;
        }
        Self { dry_run }
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    #[cfg(not(has_libcrun))]
    {
        eprintln!("ERROR: libcrun not available at build time.");
        eprintln!("Build crun with SanCov first:");
        eprintln!("  cd vendor/crun && ./autogen.sh");
        eprintln!("  CC=clang CFLAGS=\"-fsanitize-coverage=trace-pc-guard,trace-cmp -O1 -g\" \\");
        eprintln!("    ./configure --disable-shared --enable-static && make -j$(nproc)");
        std::process::exit(1);
    }

    let args = Args::parse();
    let pid = std::process::id();

    println!("=== fuzz_crun: in-process libcrun with SanCov ===");
    if let Some(n) = args.dry_run {
        println!("  --dry-run: {n} iterations then exit\n");
    }

    let corpus_dir = PathBuf::from("corpus_crun");
    let solutions_dir = PathBuf::from("solutions_crun");
    let mountpoint = format!("/tmp/mpi-sp-fuse-crun-{pid}");
    let state_dir = format!("/tmp/crun-state-{pid}");

    for d in &[&corpus_dir, &solutions_dir] {
        std::fs::create_dir_all(d).expect("failed to create output dir");
    }
    std::fs::create_dir_all(&mountpoint).expect("failed to create FUSE mountpoint");
    std::fs::create_dir_all(&state_dir).expect("failed to create crun state dir");

    let bin_true: Vec<u8> = std::fs::read("/bin/true")
        .or_else(|_| std::fs::read("/usr/bin/true"))
        .unwrap_or_default();

    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    // ── VFS + FUSE ────────────────────────────────────────────────────────────
    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");

    // rootfs_path in config.json must point to the FUSE mount
    let fuse_rootfs_path = format!("{mountpoint}/rootfs");
    let baseline_config = make_baseline_config(uid, gid, &fuse_rootfs_path);

    unsafe { init_vfs(vfs, &baseline_config, &bin_true) };
    unsafe { vfs_save_snapshot(vfs) };

    let baseline_file_paths = enumerate_vfs_file_paths(vfs);
    let baseline_dir_paths = enumerate_vfs_dir_paths(vfs);
    let baseline_all_paths = enumerate_vfs_all_paths(vfs);

    let baseline_contents: Vec<(String, Vec<u8>)> = {
        let mut c = vec![
            (
                "/rootfs/etc/passwd".to_string(),
                b"root:x:0:0:root:/root:/bin/sh\n".to_vec(),
            ),
            ("/config.json".to_string(), baseline_config.clone()),
        ];
        if !bin_true.is_empty() {
            c.push(("/rootfs/bin/true".to_string(), bin_true.clone()));
        }
        c
    };

    println!(
        "Baseline: {} file(s), {} dir(s), {} total",
        baseline_file_paths.len(),
        baseline_dir_paths.len(),
        baseline_all_paths.len()
    );

    start_fuse(vfs, &mountpoint);

    // CStrings for the harness (stable pointers across iterations)
    let state_dir_cstr = CString::new(state_dir.clone()).unwrap();

    // ── Seed corpus ───────────────────────────────────────────────────────────
    let mut initial: Vec<FsDelta> = Vec::new();
    initial.extend(config_seeds(uid, gid));
    initial.extend(rootfs_seeds(&bin_true));
    initial.push(FsDelta::new(vec![FsOp::update_file(
        "/config.json",
        baseline_config.clone(),
    )]));

    let live_corpus: LiveCorpus = Rc::new(RefCell::new(initial.clone()));
    println!("Seed corpus: {} entries\n", initial.len());

    // ── Coverage observer ─────────────────────────────────────────────────────
    let map_size = unsafe {
        if MAX_EDGES_FOUND > 0 {
            MAX_EDGES_FOUND
        } else {
            EDGES_MAP_DEFAULT_SIZE
        }
    };
    println!("Coverage map: {map_size} guards (libcrun SanCov in-process)");

    #[allow(static_mut_refs)]
    let edges_observer = unsafe {
        HitcountsMapObserver::new(StdMapObserver::from_mut_ptr(
            "edges",
            EDGES_MAP.as_mut_ptr(),
            map_size,
        ))
    };

    let mut feedback = MaxMapFeedback::new(&edges_observer);
    let mut objective = CrashFeedback::new();

    let mut state = StdState::new(
        StdRand::with_seed(current_nanos()),
        OnDiskCorpus::<FsDelta>::new(&corpus_dir).expect("corpus dir"),
        OnDiskCorpus::<FsDelta>::new(&solutions_dir).expect("solutions dir"),
        &mut feedback,
        &mut objective,
    )
    .expect("StdState");

    let monitor = SimpleMonitor::new(|msg| {
        println!("{msg}");
        let _ = std::io::stdout().flush();
    });
    let mut mgr = SimpleEventManager::new(monitor);

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
    let mut stages = tuple_list!(havoc_stage);

    let scheduler = QueueScheduler::new();
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    // ── Harness ───────────────────────────────────────────────────────────────
    let fuse_config_path = format!("{mountpoint}/config.json");

    let mut harness = |input: &FsDelta| -> ExitKind {
        // Reset VFS to clean baseline
        unsafe { vfs_reset_to_snapshot(vfs) };
        let _ = apply_delta(vfs, input);

        // Read config.json from FUSE (may have been mutated by FsDelta ops)
        let raw = std::fs::read(&fuse_config_path).unwrap_or_else(|_| baseline_config.clone());

        // Patch root.path so crun always finds the FUSE rootfs regardless of
        // what the mutator did to the rest of the config.
        let patched = patch_root_path(&raw, &fuse_rootfs_path);

        // NUL-terminate for C
        let Ok(config_cstr) = CString::new(patched) else {
            return ExitKind::Ok; // embedded NUL — skip
        };

        let iter = ITER.fetch_add(1, Ordering::Relaxed);
        let id = format!("fuzz-{pid}-{iter}");
        let Ok(id_cstr) = CString::new(id) else {
            return ExitKind::Ok;
        };

        // Call libcrun in-process — SanCov fires here.
        // Return value is the container exit code or -1 for parse/validation
        // errors; both are expected and treated as Ok. Real crashes (SIGSEGV,
        // SIGABRT, etc.) are caught by InProcessExecutor's signal handlers and
        // reported as ExitKind::Crash — the harness return value is irrelevant
        // for those cases.
        let _ret = unsafe {
            fuzz_crun_run_container(
                config_cstr.as_ptr(),
                state_dir_cstr.as_ptr(),
                id_cstr.as_ptr(),
            )
        };

        ExitKind::Ok
    };

    // 10 s timeout per iteration (namespace setup can be slow)
    let mut executor = InProcessExecutor::with_timeout(
        &mut harness,
        tuple_list!(edges_observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
        Duration::from_secs(10),
    )
    .expect("InProcessExecutor");

    // Seed corpus
    let seed_start = std::time::Instant::now();
    for delta in &initial {
        fuzzer
            .add_input(&mut state, &mut executor, &mut mgr, delta.clone())
            .expect("failed to add seed");
    }
    println!(
        "Seeding done in {:.1}s\n",
        seed_start.elapsed().as_secs_f64()
    );
    println!("Starting fuzzing loop — Ctrl-C to stop");
    println!("  corpus  → corpus_crun/");
    println!("  crashes → solutions_crun/\n");

    let start = Instant::now(); // reset after seeding so exec/sec is accurate
    let mut total: u64 = 0;

    loop {
        let before = state.corpus().count();

        fuzzer
            .fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr)
            .expect("fuzzing iteration failed");

        total += 1;

        // maybe_report_progress is time-based (fires every 2s regardless of
        // corpus growth) and reports cumulative edge coverage from MaxMapFeedback.
        // The monitor callback flushes stdout so lines reach the log file
        // immediately even when piped through tee.
        mgr.maybe_report_progress(&mut state, Duration::from_secs(2))
            .expect("progress report failed");

        // Keep live_corpus in sync for SpliceDelta
        let after = state.corpus().count();
        for idx in before..after {
            let cid = CorpusId::from(idx);
            if let Ok(tc) = state.corpus().get(cid) {
                if let Some(input) = tc.borrow().input().clone() {
                    live_corpus.borrow_mut().push(input);
                }
            }
        }

        if let Some(max) = args.dry_run {
            if total >= max {
                let elapsed = start.elapsed().as_secs();
                let exec_sec = total as f64 / elapsed.max(1) as f64;
                println!("\n--dry-run: {total} iters in {elapsed}s ({exec_sec:.1} exec/s)");
                break;
            }
        }
    }
}
