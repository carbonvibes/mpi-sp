use std::{
    cell::RefCell,
    env,
    ffi::CString,
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    process::{Command, Stdio},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

use libafl::{
    corpus::{Corpus, CorpusId, OnDiskCorpus},
    events::SimpleEventManager,
    executors::{ExitKind, InProcessExecutor},
    feedbacks::{CrashFeedback, MaxMapFeedback},
    fuzzer::{Evaluator, Fuzzer, StdFuzzer},
    monitors::SimpleMonitor,
    mutators::HavocScheduledMutator,
    schedulers::QueueScheduler,
    stages::StdMutationalStage,
    state::{HasCorpus, StdState},
    HasMetadata,
};
use libafl::observers::cmp::CmpValuesMetadata;
use libafl::observers::{HitcountsMapObserver, Observer, StdMapObserver};
use libafl_bolts::{current_nanos, rands::StdRand, tuples::tuple_list, Named};
use libafl_targets::{EDGES_MAP, EDGES_MAP_DEFAULT_SIZE};
use libafl_targets::coverage::MAX_EDGES_FOUND;
use libafl_targets::cmps::{CmpLogObserver, CMPLOG_ENABLED};
use serde::{Deserialize, Serialize};

// Newtype so CmpLogObserver satisfies the Serialize+Deserialize bounds on InProcessExecutor.
// The CmpLog map is global, so serializing as an empty map and reconstructing on deserialize is fine.
#[derive(Debug)]
struct SerializableCmpLogObserver {
    inner: CmpLogObserver,
}

impl SerializableCmpLogObserver {
    fn new(name: &'static str, add_meta: bool) -> Self {
        Self { inner: CmpLogObserver::new(name, add_meta) }
    }
}

impl Serialize for SerializableCmpLogObserver {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        s.serialize_map(Some(0))?.end()
    }
}

impl<'de> Deserialize<'de> for SerializableCmpLogObserver {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let _ = serde::de::IgnoredAny::deserialize(d)?;
        Ok(Self::new("cmplog", true))
    }
}

impl Named for SerializableCmpLogObserver {
    fn name(&self) -> &std::borrow::Cow<'static, str> {
        self.inner.name()
    }
}

impl<I, S> Observer<I, S> for SerializableCmpLogObserver
where
    S: HasMetadata,
{
    fn pre_exec(&mut self, state: &mut S, input: &I) -> Result<(), libafl::Error> {
        self.inner.pre_exec(state, input)
    }

    fn post_exec(
        &mut self,
        state: &mut S,
        input: &I,
        exit_kind: &ExitKind,
    ) -> Result<(), libafl::Error> {
        self.inner.post_exec(state, input, exit_kind)
    }
}

use fs_mutator::{
    delta::{generate_seed_corpus, initial_corpus_pool, FsDelta, FsOp},
    ffi::{
        apply_delta, enumerate_vfs_all_paths, enumerate_vfs_dir_paths,
        enumerate_vfs_file_paths, vfs_create, vfs_create_file, vfs_mkdir,
        vfs_reset_to_snapshot, vfs_save_snapshot, VfsT,
    },
    mutators::{
        AddFileOp, ByteFlipFileContent, DestructiveMutator, FsDeltaI2SMutator, LiveCorpus,
        MutatePath, RemoveOp, ReplaceFileContent, ReplayWriteFile, SpliceDelta,
        UpdateExistingFile,
    },
};

#[cfg(has_fuse3)]
use fs_mutator::ffi::{fuse_vfs_lib_init, fuse_vfs_lib_is_mounted, fuse_vfs_lib_run};

static RUNC_ITER: AtomicU64 = AtomicU64::new(0);

extern "C" {
    fn fuzz_foobar_from_path(path: *const std::os::raw::c_char);

    #[cfg(has_libarchive)]
    fn fuzz_libarchive_from_path(path: *const std::os::raw::c_char);
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Campaign { Foobar, Libarchive, Runc }

impl Campaign {
    fn from_arg(s: &str) -> Self {
        match s {
            "libarchive" => Self::Libarchive,
            "runc"       => Self::Runc,
            _            => Self::Foobar,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Foobar     => "foobar",
            Self::Libarchive => "libarchive",
            Self::Runc       => "runc",
        }
    }
}

fn libarchive_seeds() -> Vec<FsDelta> {
    let tar_empty = vec![0u8; 1024];

    let zip_empty: Vec<u8> = vec![
        0x50, 0x4b, 0x05, 0x06,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    let gzip_empty: Vec<u8> = vec![
        0x1f, 0x8b, 0x08, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
        0x03, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ];

    let xz_header: Vec<u8> = vec![
        0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00,
        0x00, 0x04, 0xe6, 0xd6, 0xb4, 0x46,
    ];

    let bzip2_magic: Vec<u8> = vec![
        0x42, 0x5a, 0x68, 0x39,
        0x17, 0x72, 0x45, 0x38, 0x50, 0x90,
        0x00, 0x00, 0x00, 0x00,
    ];

    vec![
        FsDelta::new(vec![FsOp::update_file("/input", tar_empty)]),
        FsDelta::new(vec![FsOp::update_file("/input", zip_empty)]),
        FsDelta::new(vec![FsOp::update_file("/input", gzip_empty)]),
        FsDelta::new(vec![FsOp::update_file("/input", xz_header)]),
        FsDelta::new(vec![FsOp::update_file("/input", bzip2_magic)]),
    ]
}

fn runc_rootfs_seeds(bin_true: &[u8]) -> Vec<FsDelta> {
    let mut seeds = Vec::new();

    seeds.push(FsDelta::new(vec![FsOp::rmdir("/rootfs/proc")]));
    seeds.push(FsDelta::new(vec![FsOp::rmdir("/rootfs/dev")]));
    seeds.push(FsDelta::new(vec![FsOp::rmdir("/rootfs/sys")]));
    seeds.push(FsDelta::new(vec![FsOp::delete_file("/rootfs/bin/true")]));

    seeds.push(FsDelta::new(vec![
        FsOp::update_file("/rootfs/bin/true", b"not an elf".to_vec()),
    ]));
    seeds.push(FsDelta::new(vec![
        FsOp::update_file("/rootfs/bin/true", b"\x7fELF".to_vec()),
    ]));
    if !bin_true.is_empty() {
        let mut corrupt = bin_true.to_vec();
        corrupt[4] ^= 0xff;
        seeds.push(FsDelta::new(vec![FsOp::update_file("/rootfs/bin/true", corrupt)]));
    }

    seeds.push(FsDelta::new(vec![FsOp::update_file("/rootfs/etc/passwd",
        b"root:x:0:0:root:/root:/bin/sh\nnobody:x:65534:65534:nobody:/:/usr/sbin/nologin\n".to_vec())]));
    seeds.push(FsDelta::new(vec![FsOp::delete_file("/rootfs/etc/passwd")]));
    seeds.push(FsDelta::new(vec![FsOp::update_file("/rootfs/etc/passwd", b"".to_vec())]));

    seeds.push(FsDelta::new(vec![
        FsOp::mkdir("/rootfs/usr"),
        FsOp::mkdir("/rootfs/usr/bin"),
        FsOp::create_file("/rootfs/usr/bin/env", b"#!/bin/sh\nexec \"$@\"\n".to_vec()),
    ]));
    seeds.push(FsDelta::new(vec![FsOp::create_file("/rootfs/.dockerenv", b"".to_vec())]));
    seeds.push(FsDelta::new(vec![
        FsOp::mkdir("/rootfs/run"),
        FsOp::create_file("/rootfs/run/secrets", b"password=hunter2\n".to_vec()),
    ]));

    seeds.push(FsDelta::new(vec![
        FsOp::mkdir("/rootfs/a"),
        FsOp::mkdir("/rootfs/a/b"),
        FsOp::mkdir("/rootfs/a/b/c"),
        FsOp::create_file("/rootfs/a/b/c/d", b"deeply nested".to_vec()),
    ]));

    seeds.push(FsDelta::new(vec![
        FsOp::mkdir("/rootfs/safe"),
        FsOp::create_file("/rootfs/safe/file.txt", b"../../../etc/passwd".to_vec()),
    ]));

    seeds.push(FsDelta::new(vec![FsOp::truncate("/rootfs/bin/true", 0)]));

    seeds.push(FsDelta::new(vec![
        FsOp::set_times("/rootfs/bin/true", 0, 0, 0, 0),
    ]));
    seeds.push(FsDelta::new(vec![
        FsOp::set_times("/rootfs/etc/passwd", i32::MAX as i64, 0, i32::MAX as i64, 0),
    ]));

    seeds
}

unsafe fn populate_baseline(vfs: *mut VfsT) {
    vfs_create_file(vfs, c"/input".as_ptr(), b"seed".as_ptr(), 4);
    vfs_mkdir(vfs, c"/etc".as_ptr());
    vfs_create_file(
        vfs, c"/etc/config".as_ptr(),
        b"[settings]\nverbose=0\n".as_ptr(), 20,
    );
}

unsafe fn populate_runc_rootfs(vfs: *mut VfsT, bin_true: &[u8]) {
    vfs_mkdir(vfs, c"/rootfs".as_ptr());
    vfs_mkdir(vfs, c"/rootfs/bin".as_ptr());
    vfs_mkdir(vfs, c"/rootfs/proc".as_ptr());
    vfs_mkdir(vfs, c"/rootfs/dev".as_ptr());
    vfs_mkdir(vfs, c"/rootfs/sys".as_ptr());
    vfs_mkdir(vfs, c"/rootfs/tmp".as_ptr());
    vfs_mkdir(vfs, c"/rootfs/etc".as_ptr());
    vfs_mkdir(vfs, c"/rootfs/var".as_ptr());

    if !bin_true.is_empty() {
        vfs_create_file(
            vfs, c"/rootfs/bin/true".as_ptr(),
            bin_true.as_ptr(), bin_true.len(),
        );
    }

    let passwd = b"root:x:0:0:root:/root:/bin/sh\n";
    vfs_create_file(vfs, c"/rootfs/etc/passwd".as_ptr(), passwd.as_ptr(), passwd.len());

    let hosts = b"127.0.0.1 localhost\n::1 localhost\n";
    vfs_create_file(vfs, c"/rootfs/etc/hosts".as_ptr(), hosts.as_ptr(), hosts.len());

    let hostname = b"fuzz\n";
    vfs_create_file(vfs, c"/rootfs/etc/hostname".as_ptr(), hostname.as_ptr(), hostname.len());

    let resolv = b"nameserver 8.8.8.8\n";
    vfs_create_file(vfs, c"/rootfs/etc/resolv.conf".as_ptr(), resolv.as_ptr(), resolv.len());
}

fn generate_runc_config(rootfs_path: &str, uid: u32, gid: u32) -> String {
    format!(r#"{{
  "ociVersion": "1.0.0",
  "process": {{
    "terminal": false,
    "user": {{"uid": 0, "gid": 0}},
    "args": ["/bin/true"],
    "env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
    "cwd": "/",
    "noNewPrivileges": true
  }},
  "root": {{
    "path": "{rootfs_path}",
    "readonly": false
  }},
  "hostname": "fuzz",
  "linux": {{
    "uidMappings": [{{"containerID": 0, "hostID": {uid}, "size": 1}}],
    "gidMappings": [{{"containerID": 0, "hostID": {gid}, "size": 1}}],
    "namespaces": [
      {{"type": "pid"}},
      {{"type": "mount"}},
      {{"type": "user"}}
    ]
  }}
}}"#)
}

#[cfg(has_fuse3)]
fn start_fuse(vfs: *mut VfsT, mountpoint: &str) {
    unsafe { fuse_vfs_lib_init(vfs) };

    let mount_cstr = CString::new(mountpoint).expect("mountpoint has nul byte");

    thread::spawn(move || {
        unsafe { fuse_vfs_lib_run(mount_cstr.as_ptr()) };
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if unsafe { fuse_vfs_lib_is_mounted() } != 0 { break; }
        if std::time::Instant::now() > deadline {
            eprintln!("FUSE mount timed out on {mountpoint}");
            std::process::exit(1);
        }
        thread::sleep(Duration::from_millis(5));
    }
    println!("FUSE mounted at {mountpoint}");
}

#[cfg(not(has_fuse3))]
fn start_fuse(_vfs: *mut VfsT, _mountpoint: &str) {
    eprintln!("ERROR: fuse3 not available at build time.");
    eprintln!("Install with: apt install libfuse3-dev  then rebuild.");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let campaign = args.get(1).map(|s| Campaign::from_arg(s)).unwrap_or(Campaign::Foobar);
    let pid = std::process::id();

    if campaign == Campaign::Libarchive {
        #[cfg(not(has_libarchive))]
        {
            eprintln!("libarchive campaign unavailable: libarchive-dev not found at build time.");
            std::process::exit(1);
        }
    }

    if campaign == Campaign::Runc {
        if !PathBuf::from("/usr/bin/runc").exists() {
            eprintln!("ERROR: /usr/bin/runc not found — install with: sudo apt install runc");
            std::process::exit(1);
        }
    }

    println!("=== fuzz_libafl: campaign={} ===\n", campaign.name());

    let corpus_dir    = PathBuf::from(format!("corpus_{}", campaign.name()));
    let solutions_dir = PathBuf::from(format!("solutions_{}", campaign.name()));
    std::fs::create_dir_all(&corpus_dir).ok();
    std::fs::create_dir_all(&solutions_dir).ok();

    let mountpoint = format!("/tmp/mpi-sp-fuse-{pid}");
    std::fs::create_dir_all(&mountpoint).expect("failed to create FUSE mountpoint");

    let bin_true: Vec<u8> = if campaign == Campaign::Runc {
        std::fs::read("/bin/true")
            .or_else(|_| std::fs::read("/usr/bin/true"))
            .unwrap_or_default()
    } else {
        vec![]
    };

    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");

    match campaign {
        Campaign::Runc => unsafe { populate_runc_rootfs(vfs, &bin_true) },
        _              => unsafe { populate_baseline(vfs) },
    }
    unsafe { vfs_save_snapshot(vfs) };

    let baseline_file_paths = enumerate_vfs_file_paths(vfs);
    let baseline_dir_paths  = enumerate_vfs_dir_paths(vfs);
    let baseline_all_paths  = enumerate_vfs_all_paths(vfs);

    let baseline_contents: Vec<(String, Vec<u8>)> = match campaign {
        Campaign::Runc => {
            let mut c = vec![
                ("/rootfs/etc/passwd".to_string(),
                 b"root:x:0:0:root:/root:/bin/sh\n".to_vec()),
                ("/rootfs/etc/hosts".to_string(),
                 b"127.0.0.1 localhost\n::1 localhost\n".to_vec()),
            ];
            if !bin_true.is_empty() {
                c.push(("/rootfs/bin/true".to_string(), bin_true.clone()));
            }
            c
        }
        _ => vec![
            ("/input".to_string(),      b"seed".to_vec()),
            ("/etc/config".to_string(), b"[settings]\nverbose=0\n".to_vec()),
        ],
    };

    println!(
        "Baseline: {} file(s), {} dir(s), {} total",
        baseline_file_paths.len(), baseline_dir_paths.len(), baseline_all_paths.len(),
    );

    start_fuse(vfs, &mountpoint);

    let runc_bundle_dir = format!("/tmp/runc-bundle-{pid}");
    let runc_state_dir  = format!("/tmp/runc-state-{pid}");

    if campaign == Campaign::Runc {
        std::fs::create_dir_all(&runc_bundle_dir).expect("failed to create runc bundle dir");
        std::fs::create_dir_all(&runc_state_dir).expect("failed to create runc state dir");

        let rootfs_fuse_path = format!("{mountpoint}/rootfs");
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        let config_json = generate_runc_config(&rootfs_fuse_path, uid, gid);
        std::fs::write(format!("{runc_bundle_dir}/config.json"), &config_json)
            .expect("failed to write runc config.json");

        println!("runc bundle:  {runc_bundle_dir}/config.json");
        println!("runc rootfs:  {rootfs_fuse_path}  (via FUSE)");
        println!("runc state:   {runc_state_dir}");
        println!();
        println!("NOTE: runc requires user namespace support.");
        println!("If you see \"Operation not permitted\", run once (with root):");
        println!("  sudo sed -i 's|/usr/sbin/runc|/usr/bin/runc|g' /etc/apparmor.d/runc");
        println!("  sudo apparmor_parser -r /etc/apparmor.d/runc");
        println!();
    }

    let fuse_input_path = CString::new(format!("{mountpoint}/input"))
        .expect("path has nul byte");
    let fuse_input_ptr = fuse_input_path.as_ptr();

    let initial: Vec<FsDelta>;
    let seed_count: usize;

    match campaign {
        Campaign::Runc => {
            let mut v = generate_seed_corpus(&baseline_file_paths);
            v.extend(runc_rootfs_seeds(&bin_true));
            seed_count = v.len();
            v.extend(initial_corpus_pool());
            initial = v;
        }
        _ => {
            let mut v = generate_seed_corpus(&baseline_file_paths);
            seed_count = v.len();
            v.extend(initial_corpus_pool());
            if campaign == Campaign::Libarchive {
                v.extend(libarchive_seeds());
            }
            initial = v;
        }
    }

    let live_corpus: LiveCorpus = Rc::new(RefCell::new(initial.clone()));

    println!(
        "Corpus: {} entries ({} seed families + {} donors)\n",
        initial.len(), seed_count, initial.len() - seed_count,
    );

    let map_size = unsafe {
        if MAX_EDGES_FOUND > 0 { MAX_EDGES_FOUND } else { EDGES_MAP_DEFAULT_SIZE }
    };
    println!("Coverage map: {map_size} guards active");

    #[allow(static_mut_refs)]
    let edges_observer = unsafe {
        HitcountsMapObserver::new(StdMapObserver::from_mut_ptr(
            "edges",
            EDGES_MAP.as_mut_ptr(),
            map_size,
        ))
    };

    unsafe { CMPLOG_ENABLED = 1; }
    let cmplog_observer = SerializableCmpLogObserver::new("cmplog", true);

    let mut feedback  = MaxMapFeedback::new(&edges_observer);
    let mut objective = CrashFeedback::new();

    let mut state = StdState::new(
        StdRand::with_seed(current_nanos()),
        OnDiskCorpus::<FsDelta>::new(&corpus_dir).expect("failed to create on-disk corpus"),
        OnDiskCorpus::<FsDelta>::new(&solutions_dir).expect("failed to create solutions corpus"),
        &mut feedback,
        &mut objective,
    )
    .expect("failed to create StdState");

    state.add_metadata(CmpValuesMetadata::new());

    let monitor = SimpleMonitor::new(|msg| println!("{msg}"));
    let mut mgr = SimpleEventManager::new(monitor);

    // i2s_stage runs FsDeltaI2SMutator in isolation so its substitution isn't overwritten
    // by havoc before the harness executes. havoc_stage handles all structural mutations.
    let i2s_scheduled = HavocScheduledMutator::new(tuple_list!(FsDeltaI2SMutator::new()));
    let i2s_stage     = StdMutationalStage::new(i2s_scheduled);

    let mutators = tuple_list!(
        ByteFlipFileContent::new(),
        ReplaceFileContent::new(),
        AddFileOp::new(),
        RemoveOp::new(),
        MutatePath::with_baseline(baseline_all_paths.clone()),
        SpliceDelta::new(live_corpus.clone()),
        DestructiveMutator::with_baseline(
            baseline_file_paths.clone(),
            baseline_dir_paths.clone(),
            baseline_all_paths.clone(),
        ),
        UpdateExistingFile::new(baseline_file_paths.clone())
            .with_baseline_contents(baseline_contents.clone()),
        ReplayWriteFile::new(baseline_file_paths.clone()),
    );
    let scheduled    = HavocScheduledMutator::new(mutators);
    let havoc_stage  = StdMutationalStage::new(scheduled);
    let mut stages   = tuple_list!(i2s_stage, havoc_stage);

    let scheduler  = QueueScheduler::new();
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    let campaign_copy      = campaign;
    let runc_bundle_clone  = runc_bundle_dir.clone();
    let runc_state_clone   = runc_state_dir.clone();

    let mut harness = |input: &FsDelta| -> ExitKind {
        unsafe { vfs_reset_to_snapshot(vfs) };
        apply_delta(vfs, input).ok();

        match campaign_copy {
            Campaign::Foobar => {
                unsafe { fuzz_foobar_from_path(fuse_input_ptr) };
                ExitKind::Ok
            }

            Campaign::Libarchive => {
                #[cfg(has_libarchive)]
                { unsafe { fuzz_libarchive_from_path(fuse_input_ptr) }; ExitKind::Ok }
                #[cfg(not(has_libarchive))]
                unreachable!("libarchive not available — should have exited earlier")
            }

            Campaign::Runc => {
                let cid = format!("fuzz-{pid}-{}", RUNC_ITER.fetch_add(1, Ordering::Relaxed));

                let Ok(output) = Command::new("/usr/bin/runc")
                    .args([
                        "--root",   &runc_state_clone,
                        "run",
                        "--bundle", &runc_bundle_clone,
                        &cid,
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .output()
                else {
                    return ExitKind::Ok;
                };

                let _ = Command::new("/usr/bin/runc")
                    .args(["--root", &runc_state_clone, "delete", "--force", &cid])
                    .output();

                if let Some(sig) = output.status.signal() {
                    if [libc::SIGSEGV, libc::SIGABRT, libc::SIGBUS, libc::SIGFPE]
                        .contains(&sig)
                    {
                        return ExitKind::Crash;
                    }
                }
                ExitKind::Ok
            }
        }
    };

    let timeout = match campaign {
        Campaign::Runc => Duration::from_secs(10),
        _              => Duration::from_secs(5),
    };
    let mut executor = InProcessExecutor::with_timeout(
        &mut harness,
        tuple_list!(edges_observer, cmplog_observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
        timeout,
    )
    .expect("failed to create InProcessExecutor");

    for delta in &initial {
        fuzzer
            .add_input(&mut state, &mut executor, &mut mgr, delta.clone())
            .expect("failed to add seed input");
    }

    println!("Starting fuzzing loop — Ctrl-C to stop\n");
    if campaign == Campaign::Runc {
        println!("runc campaign:");
        println!("  rootfs mutations: all 9 mutators are relevant");
        println!("    AddFileOp/Mkdir   → new dirs/files runc may try to access");
        println!("    DeleteFile/Rmdir  → missing mount-points or executables");
        println!("    ByteFlip/Replace  → corrupted ELF binary in /rootfs/bin/");
        println!("    Truncate          → zero-length executable → kernel ENOEXEC");
        println!("    SetTimes          → timestamp edge cases");
        println!("  coverage: FUSE-layer SanCov (getattr/open/read/readdir per file)");
        println!("  crashes:  runc SIGSEGV/SIGABRT → saved to solutions_{}/\n",
                 campaign.name());
    }

    loop {
        let count_before = state.corpus().count();

        fuzzer
            .fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr)
            .expect("fuzzing iteration failed");

        let count_after = state.corpus().count();
        for idx in count_before..count_after {
            let cid = CorpusId::from(idx);
            if let Ok(tc) = state.corpus().get(cid) {
                if let Some(input) = tc.borrow().input().clone() {
                    live_corpus.borrow_mut().push(input);
                }
            }
        }
    }
}
