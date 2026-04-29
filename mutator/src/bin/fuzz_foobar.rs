use std::{
    cell::RefCell,
    ffi::CString,
    path::PathBuf,
    rc::Rc,
    thread,
    time::Duration,
};

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
use libafl::observers::{HitcountsMapObserver, StdMapObserver};
use libafl_bolts::{current_nanos, rands::StdRand, tuples::tuple_list};
use libafl_targets::{EDGES_MAP, EDGES_MAP_DEFAULT_SIZE};
use libafl_targets::coverage::MAX_EDGES_FOUND;

use fs_mutator::{
    delta::{generate_seed_corpus, initial_corpus_pool, FsDelta},
    ffi::{
        apply_delta, enumerate_vfs_all_paths, enumerate_vfs_dir_paths,
        enumerate_vfs_file_paths, vfs_create, vfs_create_file, vfs_mkdir,
        vfs_reset_to_snapshot, vfs_save_snapshot, VfsT,
    },
    mutators::{
        AddFileOp, ByteFlipFileContent, DestructiveMutator, LiveCorpus,
        MutatePath, RemoveOp, ReplaceFileContent, ReplayWriteFile, SpliceDelta,
        UpdateExistingFile,
    },
};

#[cfg(has_fuse3)]
use fs_mutator::ffi::{fuse_vfs_lib_init, fuse_vfs_lib_is_mounted, fuse_vfs_lib_run};

extern "C" {
    fn fuzz_foobar_from_path(path: *const std::os::raw::c_char);
}

unsafe fn populate_baseline(vfs: *mut VfsT) {
    vfs_create_file(vfs, c"/input".as_ptr(), b"seed".as_ptr(), 4);
    vfs_mkdir(vfs, c"/etc".as_ptr());
    vfs_create_file(
        vfs, c"/etc/config".as_ptr(),
        b"[settings]\nverbose=0\n".as_ptr(), 20,
    );
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
    let pid = std::process::id();

    println!("=== fuzz_foobar ===\n");

    let corpus_dir    = PathBuf::from("corpus_foobar");
    let solutions_dir = PathBuf::from("solutions_foobar");
    std::fs::create_dir_all(&corpus_dir).ok();
    std::fs::create_dir_all(&solutions_dir).ok();

    let mountpoint = format!("/tmp/mpi-sp-fuse-{pid}");
    std::fs::create_dir_all(&mountpoint).expect("failed to create FUSE mountpoint");

    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");

    unsafe { populate_baseline(vfs) };
    unsafe { vfs_save_snapshot(vfs) };

    let baseline_file_paths = enumerate_vfs_file_paths(vfs);
    let baseline_dir_paths  = enumerate_vfs_dir_paths(vfs);
    let baseline_all_paths  = enumerate_vfs_all_paths(vfs);

    let baseline_contents: Vec<(String, Vec<u8>)> = vec![
        ("/input".to_string(),      b"seed".to_vec()),
        ("/etc/config".to_string(), b"[settings]\nverbose=0\n".to_vec()),
    ];

    println!(
        "Baseline: {} file(s), {} dir(s), {} total",
        baseline_file_paths.len(), baseline_dir_paths.len(), baseline_all_paths.len(),
    );

    start_fuse(vfs, &mountpoint);

    let fuse_input_path = CString::new(format!("{mountpoint}/input"))
        .expect("path has nul byte");
    let fuse_input_ptr = fuse_input_path.as_ptr();

    let mut v = generate_seed_corpus(&baseline_file_paths);
    let seed_count = v.len();
    v.extend(initial_corpus_pool());
    let initial = v;

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

    let monitor = SimpleMonitor::new(|msg| println!("{msg}"));
    let mut mgr = SimpleEventManager::new(monitor);

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
    let scheduled   = HavocScheduledMutator::new(mutators);
    let havoc_stage = StdMutationalStage::new(scheduled);
    let mut stages  = tuple_list!(havoc_stage);

    let scheduler  = QueueScheduler::new();
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    let mut harness = |input: &FsDelta| -> ExitKind {
        unsafe { vfs_reset_to_snapshot(vfs) };
        apply_delta(vfs, input).ok();
        unsafe { fuzz_foobar_from_path(fuse_input_ptr) };
        ExitKind::Ok
    };

    let mut executor = InProcessExecutor::with_timeout(
        &mut harness,
        tuple_list!(edges_observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
        Duration::from_secs(5),
    )
    .expect("failed to create InProcessExecutor");

    for delta in &initial {
        fuzzer
            .add_input(&mut state, &mut executor, &mut mgr, delta.clone())
            .expect("failed to add seed input");
    }

    println!("Starting fuzzing loop — Ctrl-C to stop\n");

    loop {
        let count_before = state.corpus().count();

        fuzzer
            .fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr)
            .expect("fuzzing iteration failed");

        mgr.maybe_report_progress(&mut state, Duration::from_secs(2))
            .expect("failed to report progress");

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
