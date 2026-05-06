//! fuzz_runc — Dedicated fuzzing harness for runc / crun OCI container runtimes.
//!
//! Both inputs to an OCI runtime are fuzzed:
//!   • /config.json  — OCI spec bytes stored in the VFS; byte-level mutations apply
//!                     naturally.  root.path is always patched to the FUSE rootfs.
//!   • /rootfs/…     — container filesystem; mutated by all FsDelta mutators via FUSE.
//!
//! Usage:
//!   cargo run --bin fuzz_runc --release                           # auto-detect runc/crun
//!   cargo run --bin fuzz_runc --release -- --target crun          # force crun
//!   cargo run --bin fuzz_runc --release -- --dry-run              # 20 iterations then exit
//!   cargo run --bin fuzz_runc --release -- --dry-run 50           # 50 iterations then exit
//!   cargo run --release --bin fuzz_runc 2>&1 | tee /tmp/runc_fuzz.log
//! Dashboard (in a second terminal):
//!   cd fuzz_dashboard && python3 server.py /tmp/runc_fuzz.log runc

use std::{
    cell::RefCell,
    ffi::CString,
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    process::{Command, Stdio},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
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

static ITER: AtomicU64 = AtomicU64::new(0);

// ── Target (runc / crun) ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Runc,
    Crun,
}

impl Target {
    fn detect() -> Self {
        for p in &["/usr/bin/crun", "/usr/local/bin/crun"] {
            if PathBuf::from(p).exists() {
                return Self::Crun;
            }
        }
        Self::Runc
    }
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "runc" => Some(Self::Runc),
            "crun" => Some(Self::Crun),
            _ => None,
        }
    }
    fn binary(self) -> &'static str {
        match self {
            Self::Runc => "/usr/bin/runc",
            Self::Crun => "/usr/bin/crun",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Runc => "runc",
            Self::Crun => "crun",
        }
    }
}

// ── CLI ───────────────────────────────────────────────────────────────────────

struct Args {
    target: Target,
    dry_run: Option<u64>,
}

impl Args {
    fn parse() -> Self {
        let argv: Vec<String> = std::env::args().collect();
        let mut target: Option<Target> = None;
        let mut dry_run: Option<u64> = None;
        let mut i = 1;
        while i < argv.len() {
            match argv[i].as_str() {
                "--target" => {
                    i += 1;
                    target = argv.get(i).and_then(|s| Target::from_str(s));
                    if target.is_none() {
                        eprintln!(
                            "Unknown target '{}'; use runc or crun",
                            argv.get(i).map_or("", |s| s.as_str())
                        );
                        std::process::exit(1);
                    }
                }
                "--dry-run" => {
                    let n = argv.get(i + 1).and_then(|s| s.parse::<u64>().ok());
                    if n.is_some() {
                        i += 1;
                    }
                    dry_run = Some(n.unwrap_or(20));
                }
                _ => {}
            }
            i += 1;
        }
        Self {
            target: target.unwrap_or_else(Target::detect),
            dry_run,
        }
    }
}

// ── VFS initialisation ────────────────────────────────────────────────────────

/// Populate the baseline VFS:
///   /config.json     — baseline OCI spec (root.path = "PLACEHOLDER")
///   /rootfs/…        — container filesystem with standard directories + bin/true
unsafe fn init_vfs(vfs: *mut VfsT, config_bytes: &[u8], bin_true: &[u8]) {
    // config.json lives at the VFS root; the harness reads it every iteration
    vfs_create_file(
        vfs,
        c"/config.json".as_ptr(),
        config_bytes.as_ptr(),
        config_bytes.len(),
    );

    // rootfs skeleton
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

/// Build the baseline OCI config.json with a placeholder root.path.
/// The harness patches root.path to the real FUSE mount path each iteration.
fn make_baseline_config(uid: u32, gid: u32) -> Vec<u8> {
    serde_json::json!({
        "ociVersion": "1.0.0",
        "process": {
            "terminal": false,
            "user": {"uid": 0, "gid": 0},
            "args": ["/bin/true"],
            "env": [
                "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                "TERM=xterm"
            ],
            "cwd": "/",
            "capabilities": {
                "bounding":  ["CAP_AUDIT_WRITE","CAP_KILL","CAP_NET_BIND_SERVICE"],
                "effective": ["CAP_AUDIT_WRITE","CAP_KILL","CAP_NET_BIND_SERVICE"],
                "permitted": ["CAP_AUDIT_WRITE","CAP_KILL","CAP_NET_BIND_SERVICE"]
            },
            "noNewPrivileges": true,
            "rlimits": [{"type":"RLIMIT_NOFILE","hard":1024,"soft":1024}]
        },
        "root": {"path": "PLACEHOLDER", "readonly": false},
        "hostname": "fuzz",
        "mounts": [
            {"destination":"/proc","type":"proc","source":"proc",
             "options":["nosuid","noexec","nodev"]},
            {"destination":"/dev","type":"tmpfs","source":"tmpfs",
             "options":["nosuid","strictatime","mode=755","size=65536k"]},
            {"destination":"/sys","type":"sysfs","source":"sysfs",
             "options":["nosuid","noexec","nodev","ro"]}
        ],
        "linux": {
            "uidMappings": [{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings": [{"containerID":0,"hostID":gid,"size":1}],
            "namespaces": [
                {"type":"pid"}, {"type":"mount"}, {"type":"user"}
            ]
        }
    })
    .to_string()
    .into_bytes()
}

/// Patch root.path in a (possibly mutated) config.json byte slice.
///
/// If the bytes are still valid JSON: patch root.path and re-serialise.
/// runc will then always find a valid rootfs while all other fields remain
/// as the fuzzer left them (corrupted args, caps, namespace lists, …).
///
/// If the bytes are NOT valid JSON: return them unchanged.
/// runc's own JSON parser is then the fuzzing target.
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
            serde_json::to_vec_pretty(&v).unwrap_or_else(|_| config_bytes.to_vec())
        }
        Err(_) => config_bytes.to_vec(), // malformed JSON → runc parser fuzzing
    }
}

// ── Seed corpus ───────────────────────────────────────────────────────────────

/// 10 structurally diverse config.json seeds as FsDelta UpdateFile ops.
/// Each variant exercises a different OCI spec feature / code path in runc.
fn config_seeds(uid: u32, gid: u32) -> Vec<FsDelta> {
    let ns = serde_json::json!([{"type":"pid"},{"type":"mount"},{"type":"user"}]);
    let _maps = serde_json::json!({
        "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
        "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]
    });
    let root_ro = serde_json::json!({"path":"PLACEHOLDER","readonly":true});
    let root_rw = serde_json::json!({"path":"PLACEHOLDER","readonly":false});
    let proc_min = serde_json::json!({"terminal":false,"user":{"uid":0,"gid":0},
        "args":["/bin/true"],"env":["PATH=/bin"],"cwd":"/","noNewPrivileges":true});

    let to_delta = |v: serde_json::Value| -> FsDelta {
        FsDelta::new(vec![FsOp::update_file(
            "/config.json",
            v.to_string().into_bytes(),
        )])
    };

    vec![
        // 1. Minimal — no mounts, no capabilities block
        to_delta(serde_json::json!({
            "ociVersion":"1.0.0","process":proc_min,"root":root_rw,
            "hostname":"fuzz","linux":{"namespaces":ns,
            "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]}
        })),
        // 2. Read-only rootfs
        to_delta(serde_json::json!({
            "ociVersion":"1.0.0","process":proc_min,"root":root_ro,
            "hostname":"fuzz","linux":{"namespaces":ns,
            "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]}
        })),
        // 3. Full mount set (proc + dev + sys + shm + mqueue)
        to_delta(serde_json::json!({
            "ociVersion":"1.0.0","process":proc_min,"root":root_rw,"hostname":"fuzz",
            "mounts":[
                {"destination":"/proc","type":"proc","source":"proc","options":["nosuid","noexec","nodev"]},
                {"destination":"/dev","type":"tmpfs","source":"tmpfs","options":["nosuid","strictatime","mode=755","size=65536k"]},
                {"destination":"/sys","type":"sysfs","source":"sysfs","options":["nosuid","noexec","nodev","ro"]},
                {"destination":"/dev/shm","type":"tmpfs","source":"shm","options":["nosuid","noexec","nodev","mode=1777","size=67108864"]},
                {"destination":"/dev/mqueue","type":"mqueue","source":"mqueue","options":["nosuid","noexec","nodev"]}
            ],
            "linux":{"namespaces":ns,
            "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]}
        })),
        // 4. Large capability bounding set
        to_delta(serde_json::json!({
            "ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"args":["/bin/true"],
                "env":["PATH=/bin"],"cwd":"/",
                "capabilities":{
                    "bounding":["CAP_CHOWN","CAP_DAC_OVERRIDE","CAP_FSETID","CAP_FOWNER",
                        "CAP_MKNOD","CAP_NET_RAW","CAP_SETGID","CAP_SETUID","CAP_SETFCAP",
                        "CAP_SETPCAP","CAP_NET_BIND_SERVICE","CAP_SYS_CHROOT","CAP_KILL",
                        "CAP_AUDIT_WRITE"],
                    "effective":["CAP_KILL","CAP_NET_BIND_SERVICE","CAP_AUDIT_WRITE"],
                    "permitted":["CAP_KILL","CAP_NET_BIND_SERVICE","CAP_AUDIT_WRITE"]
                }
            },
            "root":root_rw,"hostname":"fuzz",
            "linux":{"namespaces":ns,
            "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]}
        })),
        // 5. Empty capabilities block
        to_delta(serde_json::json!({
            "ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"args":["/bin/true"],
                "env":["PATH=/bin"],"cwd":"/",
                "capabilities":{"bounding":[],"effective":[],"permitted":[]}},
            "root":root_rw,"hostname":"fuzz",
            "linux":{"namespaces":ns,
            "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]}
        })),
        // 6. Extra rlimits
        to_delta(serde_json::json!({
            "ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"args":["/bin/true"],
                "env":["PATH=/bin"],"cwd":"/","noNewPrivileges":true,
                "rlimits":[
                    {"type":"RLIMIT_NOFILE","hard":4096,"soft":1024},
                    {"type":"RLIMIT_NPROC","hard":64,"soft":32},
                    {"type":"RLIMIT_AS","hard":536870912,"soft":536870912}
                ]},
            "root":root_rw,"hostname":"fuzz",
            "linux":{"namespaces":ns,
            "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]}
        })),
        // 7. noNewPrivileges = false
        to_delta(serde_json::json!({
            "ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"args":["/bin/true"],
                "env":["PATH=/bin"],"cwd":"/","noNewPrivileges":false},
            "root":root_rw,"hostname":"fuzz",
            "linux":{"namespaces":ns,
            "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]}
        })),
        // 8. Annotations block (runc must not crash on unknown annotations)
        to_delta(serde_json::json!({
            "ociVersion":"1.0.0","process":proc_min,"root":root_rw,"hostname":"fuzz",
            "annotations":{"com.example.key":"value","io.kubernetes.cri.sandbox-id":"abc123"},
            "linux":{"namespaces":ns,
            "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]}
        })),
        // 9. Empty args (exercises runc arg-validation path)
        to_delta(serde_json::json!({
            "ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"args":[],
                "env":["PATH=/bin"],"cwd":"/"},
            "root":root_rw,"hostname":"fuzz",
            "linux":{"namespaces":ns,
            "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]}
        })),
        // 10. Non-existent binary in args
        to_delta(serde_json::json!({
            "ociVersion":"1.0.0",
            "process":{"terminal":false,"user":{"uid":0,"gid":0},
                "args":["/bin/nonexistent","--flag"],"env":["PATH=/bin"],"cwd":"/"},
            "root":root_rw,"hostname":"fuzz",
            "linux":{"namespaces":ns,
            "uidMappings":[{"containerID":0,"hostID":uid,"size":1}],
            "gidMappings":[{"containerID":0,"hostID":gid,"size":1}]}
        })),
    ]
}

/// Rootfs-structure seed deltas — stress-test runc's filesystem handling.
fn rootfs_seeds(bin_true: &[u8]) -> Vec<FsDelta> {
    let mut seeds = vec![
        // Remove standard mount-point directories
        FsDelta::new(vec![FsOp::rmdir("/rootfs/proc")]),
        FsDelta::new(vec![FsOp::rmdir("/rootfs/dev")]),
        FsDelta::new(vec![FsOp::rmdir("/rootfs/sys")]),
        // Delete / corrupt the container binary
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
        // Corrupt passwd
        FsDelta::new(vec![FsOp::delete_file("/rootfs/etc/passwd")]),
        FsDelta::new(vec![FsOp::update_file("/rootfs/etc/passwd", b"".to_vec())]),
        // Timestamp edge cases
        FsDelta::new(vec![FsOp::set_times("/rootfs/bin/true", 0, 0, 0, 0)]),
        FsDelta::new(vec![FsOp::set_times(
            "/rootfs/etc/passwd",
            i32::MAX as i64,
            0,
            i32::MAX as i64,
            0,
        )]),
        // Deep nesting
        FsDelta::new(vec![
            FsOp::mkdir("/rootfs/a"),
            FsOp::mkdir("/rootfs/a/b"),
            FsOp::mkdir("/rootfs/a/b/c"),
            FsOp::create_file("/rootfs/a/b/c/d", b"deeply nested".to_vec()),
        ]),
        // Traversal-attempt content in a file
        FsDelta::new(vec![
            FsOp::mkdir("/rootfs/safe"),
            FsOp::create_file("/rootfs/safe/link.txt", b"../../../etc/passwd".to_vec()),
        ]),
        // Extra files runc might probe
        FsDelta::new(vec![FsOp::create_file("/rootfs/.dockerenv", b"".to_vec())]),
        FsDelta::new(vec![
            FsOp::mkdir("/rootfs/run"),
            FsOp::create_file("/rootfs/run/secrets", b"password=hunter2\n".to_vec()),
        ]),
    ];

    // If we have the real /bin/true, add a bit-flip variant
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
    eprintln!("Install with: sudo apt install libfuse3-dev");
    std::process::exit(1);
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let args = Args::parse();
    let pid = std::process::id();

    // Verify the target binary exists
    if !PathBuf::from(args.target.binary()).exists() {
        eprintln!(
            "ERROR: {} not found. Install with: sudo apt install {}",
            args.target.binary(),
            args.target.name()
        );
        std::process::exit(1);
    }

    println!(
        "=== fuzz_runc: target={} pid={} ===\n",
        args.target.name(),
        pid
    );
    if let Some(n) = args.dry_run {
        println!("  --dry-run mode: will exit after {n} iterations\n");
    }

    // Working directories
    let corpus_dir = PathBuf::from(format!("corpus_{}", args.target.name()));
    let solutions_dir = PathBuf::from(format!("solutions_{}", args.target.name()));
    let mountpoint = format!("/tmp/mpi-sp-fuse-{pid}");
    let bundle_dir = format!("/tmp/runc-bundle-{pid}");
    let state_dir = format!("/tmp/runc-state-{pid}");

    for d in &[&corpus_dir, &solutions_dir] {
        std::fs::create_dir_all(d).expect("failed to create output dir");
    }
    for d in &[&mountpoint, &bundle_dir, &state_dir] {
        std::fs::create_dir_all(d).expect("failed to create working dir");
    }

    // Read the real /bin/true to use as rootfs container binary
    let bin_true: Vec<u8> = std::fs::read("/bin/true")
        .or_else(|_| std::fs::read("/usr/bin/true"))
        .unwrap_or_default();

    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    // Build baseline config and initialise VFS
    let baseline_config = make_baseline_config(uid, gid);

    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");

    unsafe { init_vfs(vfs, &baseline_config, &bin_true) };
    unsafe { vfs_save_snapshot(vfs) };

    // Enumerate baseline paths (needed by mutators that must respect baseline layout)
    let baseline_file_paths = enumerate_vfs_file_paths(vfs);
    let baseline_dir_paths = enumerate_vfs_dir_paths(vfs);
    let baseline_all_paths = enumerate_vfs_all_paths(vfs);

    // Baseline file contents for UpdateExistingFile mutator
    let baseline_contents: Vec<(String, Vec<u8>)> = {
        let mut c = vec![
            (
                "/rootfs/etc/passwd".to_string(),
                b"root:x:0:0:root:/root:/bin/sh\nnobody:x:65534:65534:nobody:/:/usr/sbin/nologin\n"
                    .to_vec(),
            ),
            (
                "/rootfs/etc/hosts".to_string(),
                b"127.0.0.1 localhost\n::1 localhost\n".to_vec(),
            ),
            ("/config.json".to_string(), baseline_config.clone()),
        ];
        if !bin_true.is_empty() {
            c.push(("/rootfs/bin/true".to_string(), bin_true.clone()));
        }
        c
    };

    println!(
        "Baseline: {} file(s), {} dir(s), {} total paths",
        baseline_file_paths.len(),
        baseline_dir_paths.len(),
        baseline_all_paths.len()
    );

    // Start FUSE — exposes /config.json and /rootfs/ at $mountpoint
    start_fuse(vfs, &mountpoint);

    // Paths used by the harness closure every iteration
    let fuse_config_path = format!("{mountpoint}/config.json");
    let fuse_rootfs_path = format!("{mountpoint}/rootfs");
    let bundle_config = format!("{bundle_dir}/config.json");

    // Print AppArmor hint once (runc may refuse user namespaces otherwise)
    println!("\nNOTE: if runc says 'Operation not permitted', run once:");
    println!("  sudo sysctl kernel.unprivileged_userns_clone=1  (if needed)");
    println!("  sudo apparmor_parser -r /etc/apparmor.d/runc    (if apparmor is active)\n");

    // Build seed corpus: config variants + rootfs variants
    let mut initial: Vec<FsDelta> = Vec::new();
    initial.extend(config_seeds(uid, gid));
    initial.extend(rootfs_seeds(&bin_true));
    let seed_count = initial.len();
    // generic structural donors
    initial.push(FsDelta::new(vec![FsOp::update_file(
        "/config.json",
        baseline_config.clone(),
    )]));

    let live_corpus: LiveCorpus = Rc::new(RefCell::new(initial.clone()));

    println!(
        "Seed corpus: {} entries ({seed_count} typed seeds + {} donors)\n",
        initial.len(),
        initial.len() - seed_count
    );

    // Coverage map
    let map_size = unsafe {
        if MAX_EDGES_FOUND > 0 {
            MAX_EDGES_FOUND
        } else {
            EDGES_MAP_DEFAULT_SIZE
        }
    };
    println!("Coverage map: {map_size} guards");
    println!("NOTE: runc is an external process; coverage comes from VFS access patterns.");
    println!("      Crash detection (SIGSEGV/SIGABRT/panic) is the primary signal.\n");

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
        OnDiskCorpus::<FsDelta>::new(&corpus_dir).expect("on-disk corpus"),
        OnDiskCorpus::<FsDelta>::new(&solutions_dir).expect("solutions corpus"),
        &mut feedback,
        &mut objective,
    )
    .expect("StdState");

    let monitor = SimpleMonitor::new(|msg| println!("{msg}"));
    let mut mgr = SimpleEventManager::new(monitor);

    let mut stages = tuple_list!(StdMutationalStage::new(HavocScheduledMutator::new(
        tuple_list!(
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
                .with_baseline_contents(baseline_contents.clone()),
            ReplayWriteFile::new(baseline_file_paths.clone()),
        )
    )));
    let scheduler = QueueScheduler::new();
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    // ── Harness closure ───────────────────────────────────────────────────────
    //
    // Per-iteration:
    //   1. Reset VFS to clean snapshot
    //   2. Apply FsDelta (may mutate /config.json bytes and/or /rootfs/* entries)
    //   3. Read /config.json from FUSE; patch root.path to FUSE rootfs
    //   4. Write patched config.json to real bundle dir
    //   5. exec runc/crun; collect exit status + stderr
    //   6. Cleanup container state
    //   7. Classify: fatal signal or panic → Crash; else Ok
    //
    let target_bin = args.target.binary();
    let dry_run_max = args.dry_run;

    let mut harness = |input: &FsDelta| -> ExitKind {
        // ── step 1: reset
        unsafe { vfs_reset_to_snapshot(vfs) };

        // ── step 2: apply delta (ignore partial failures — normal for fuzzing)
        let _ = apply_delta(vfs, input);

        // ── step 3: read config from FUSE, patch root.path
        let raw = match std::fs::read(&fuse_config_path) {
            Ok(b) => b,
            Err(_) => baseline_config.clone(), // FUSE hiccup — use baseline
        };
        let patched = patch_root_path(&raw, &fuse_rootfs_path);

        // ── step 4: write bundle config.json
        if std::fs::write(&bundle_config, &patched).is_err() {
            return ExitKind::Ok;
        }

        // ── step 5: run target
        let cid = format!("fuzz-{pid}-{}", ITER.fetch_add(1, Ordering::Relaxed));

        let Ok(output) = Command::new(target_bin)
            .args(["--root", &state_dir, "run", "--bundle", &bundle_dir, &cid])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
        else {
            return ExitKind::Ok;
        };

        // ── step 6: always attempt cleanup (ignore errors)
        let _ = Command::new(target_bin)
            .args(["--root", &state_dir, "delete", "--force", &cid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output();

        // ── step 7: classify result
        // a) Fatal signal — definite crash
        if let Some(sig) = output.status.signal() {
            if [
                libc::SIGSEGV,
                libc::SIGABRT,
                libc::SIGBUS,
                libc::SIGFPE,
                libc::SIGILL,
            ]
            .contains(&sig)
            {
                return ExitKind::Crash;
            }
        }
        // b) Go/C runtime panic in stderr — treat as crash
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("panic:")
            || stderr.contains("runtime error:")
            || stderr.contains("fatal error:")
            || stderr.contains("SIGSEGV")
        {
            return ExitKind::Crash;
        }

        ExitKind::Ok
    };

    // Executor with 10 s per-iteration timeout (runc startup takes ~200–500 ms)
    let mut executor = InProcessExecutor::with_timeout(
        &mut harness,
        tuple_list!(edges_observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
        Duration::from_secs(10),
    )
    .expect("InProcessExecutor");

    // Seed all initial inputs
    let start_time = std::time::Instant::now();
    for delta in &initial {
        fuzzer
            .add_input(&mut state, &mut executor, &mut mgr, delta.clone())
            .expect("failed to add seed input");
    }

    println!("Starting fuzzing loop — Ctrl-C to stop");
    println!("  corpus  → {}/", corpus_dir.display());
    println!("  crashes → {}/\n", solutions_dir.display());

    let mut total_iters: u64 = 0;

    loop {
        let count_before = state.corpus().count();

        fuzzer
            .fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr)
            .expect("fuzzing iteration failed");

        total_iters += 1;

        // Emit a [UserStats] line every 2 s — same mechanism as fuzz_foobar.
        // This is what keeps the dashboard chart and terminal output alive
        // even when MaxMapFeedback never fires (runc is out-of-process).
        mgr.maybe_report_progress(&mut state, Duration::from_secs(2))
            .expect("failed to report progress");

        // Keep live_corpus up to date for SpliceDelta
        let count_after = state.corpus().count();
        for idx in count_before..count_after {
            let cid = CorpusId::from(idx);
            if let Ok(tc) = state.corpus().get(cid) {
                if let Some(input) = tc.borrow().input().clone() {
                    live_corpus.borrow_mut().push(input);
                }
            }
        }

        // --dry-run exit
        if let Some(max) = dry_run_max {
            if total_iters >= max {
                let elapsed = start_time.elapsed().as_secs();
                let exec_sec = total_iters as f64 / elapsed.max(1) as f64;
                println!(
                    "\n--dry-run: completed {total_iters} iterations \
                          in {elapsed}s ({exec_sec:.1} exec/s), exiting."
                );
                break;
            }
        }
    }
}
