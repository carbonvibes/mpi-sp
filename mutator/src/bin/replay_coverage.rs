//! replay_coverage — run every saved corpus entry through a coverage-instrumented
//! crun binary via the same FUSE VFS pipeline the fuzzer uses. Each run writes a
//! .profraw; merge with llvm-profdata and report with llvm-cov afterward.
//!
//! MUST be run as root inside a mount namespace:
//!   sudo unshare -m ./replay_coverage <corpus_dir> <grammar.py> <crun-harness-cov> [--profile-dir /tmp/cov_profiles]
//!
//! corpus_dir   — dir of `combined_N` binary corpus files
//! grammar.py   — Nautilus grammar (NautilusInput → JSON)
//! crun-harness-cov — coverage-instrumented crun binary

use std::{
    ffi::CString,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use fs_mutator::{
    delta::FsDelta,
    ffi::{
        apply_delta, vfs_create, vfs_create_file, vfs_mkdir, vfs_reset_to_snapshot,
        vfs_save_snapshot, VfsT,
    },
};
use libafl::{
    generators::NautilusContext,
    inputs::{Input, NautilusBytesConverter, NautilusInput, ToTargetBytes},
};
use serde::{Deserialize, Serialize};

#[cfg(has_fuse3)]
use fs_mutator::ffi::{fuse_vfs_lib_init, fuse_vfs_lib_is_mounted, fuse_vfs_lib_run};

const LLVM_PROFDATA: &str =
    "/nix/store/jp45dqzv8mjpqsvhj99c93pc4vlmhy16-llvm-21.1.8/bin/llvm-profdata";
const LLVM_COV: &str =
    "/nix/store/jp45dqzv8mjpqsvhj99c93pc4vlmhy16-llvm-21.1.8/bin/llvm-cov";

#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
struct CombinedInput {
    config: NautilusInput,
    rootfs: FsDelta,
}

impl Input for CombinedInput {
    fn generate_name(&self, _idx: Option<libafl::corpus::CorpusId>) -> String {
        "replay_cov".into()
    }
}

unsafe fn init_vfs(vfs: *mut VfsT) {
    for dir in &[
        c"/bin", c"/proc", c"/dev", c"/sys", c"/tmp", c"/etc", c"/var", c"/run",
        c"/usr", c"/usr/bin", c"/app", c"/home", c"/home/user",
    ] {
        vfs_mkdir(vfs, dir.as_ptr());
    }
    static BIN_TRUE: &[u8] = include_bytes!("../../static/true");
    macro_rules! mkfile {
        ($path:expr, $content:expr) => {
            vfs_create_file(vfs, $path.as_ptr(), $content.as_ptr(), $content.len())
        };
    }
    mkfile!(c"/bin/true",      BIN_TRUE);
    mkfile!(c"/bin/sh",        BIN_TRUE);
    mkfile!(c"/bin/bash",      BIN_TRUE);
    mkfile!(c"/app/app",       BIN_TRUE);
    mkfile!(c"/usr/bin/nginx", BIN_TRUE);
    mkfile!(c"/etc/passwd",
        b"root:x:0:0:root:/root:/bin/sh\nnobody:x:65534:65534:nobody:/:/usr/sbin/nologin\n");
    mkfile!(c"/etc/group",
        b"root:x:0:\ndaemon:x:1:\nbin:x:2:\nnobody:x:65534:\n");
    mkfile!(c"/etc/hosts",      b"127.0.0.1 localhost\n::1 localhost\n");
    mkfile!(c"/etc/hostname",   b"fuzz\n");
    mkfile!(c"/etc/resolv.conf", b"nameserver 8.8.8.8\n");
}

#[cfg(has_fuse3)]
fn start_fuse(vfs: *mut VfsT, mountpoint: &str) {
    unsafe { fuse_vfs_lib_init(vfs) };
    let mp = CString::new(mountpoint).expect("mountpoint nul");
    thread::spawn(move || unsafe { fuse_vfs_lib_run(mp.as_ptr()) });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if unsafe { fuse_vfs_lib_is_mounted() } != 0 { break; }
        if Instant::now() > deadline {
            eprintln!("ERROR: FUSE mount timed out");
            std::process::exit(1);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(not(has_fuse3))]
fn start_fuse(_vfs: *mut VfsT, _mountpoint: &str) {
    eprintln!("ERROR: built without libfuse3-dev");
    std::process::exit(1);
}

// forces root.path to the FUSE mount and injects a mount namespace so container
// mounts don't escape to the host. matches fuzz_combined_afl.rs.
fn override_rootfs_path(json: &[u8], fuse_rootfs: &str) -> Option<Vec<u8>> {
    let mut v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let obj = v.as_object_mut()?;

    {
        let linux = obj.entry("linux").or_insert_with(|| serde_json::json!({}));
        if let Some(l) = linux.as_object_mut() {
            let ns = l.entry("namespaces").or_insert_with(|| serde_json::json!([]));
            if let Some(arr) = ns.as_array_mut() {
                let has_mount = arr
                    .iter()
                    .any(|n| n.get("type").and_then(|t| t.as_str()) == Some("mount"));
                if !has_mount {
                    arr.push(serde_json::json!({"type": "mount"}));
                }
            }
        }
    }

    let root = obj
        .entry("root")
        .or_insert_with(|| serde_json::json!({"readonly": false}));
    if let Some(r) = root.as_object_mut() {
        r.insert("path".into(), serde_json::Value::String(fuse_rootfs.into()));
    }
    serde_json::to_vec(&v).ok()
}

fn main() {
    pyo3::prepare_freethreaded_python();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!(
            "Usage: sudo unshare -m {} <corpus_dir> <grammar.py> <crun-harness-cov> [--profile-dir DIR] [--before-epoch TS]",
            args[0]
        );
        std::process::exit(1);
    }

    let corpus_dir  = PathBuf::from(&args[1]);
    let grammar_path = PathBuf::from(&args[2]);
    let harness      = &args[3];

    let profile_dir = args.windows(2)
        .find(|w| w[0] == "--profile-dir")
        .map(|w| PathBuf::from(&w[1]))
        .unwrap_or_else(|| PathBuf::from("/tmp/cov_profiles"));

    // only replay entries with mtime (= save time) at or before this epoch;
    // selects the first N hours of a campaign.
    let before_epoch: Option<u64> = args.windows(2)
        .find(|w| w[0] == "--before-epoch")
        .and_then(|w| w[1].parse().ok());

    std::fs::create_dir_all(&profile_dir).expect("failed to create profile dir");

    // harness creates rootfs/ here each run
    let workdir = PathBuf::from(format!("/tmp/cov_workdir_{}", std::process::id()));
    std::fs::create_dir_all(&workdir).expect("failed to create workdir");

    eprintln!("[cov] corpus   : {}", corpus_dir.display());
    eprintln!("[cov] grammar  : {}", grammar_path.display());
    eprintln!("[cov] harness  : {harness}");
    eprintln!("[cov] profiles : {}", profile_dir.display());

    let context: &'static NautilusContext =
        Box::leak(Box::new(NautilusContext::from_file(100, &grammar_path).unwrap_or_else(|e| {
            eprintln!("Failed to load grammar: {e}");
            std::process::exit(1);
        })));

    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");
    unsafe { init_vfs(vfs) };
    unsafe { vfs_save_snapshot(vfs) };

    let mountpoint = format!("/tmp/cov_fuse_{}", std::process::id());
    std::fs::create_dir_all(&mountpoint).expect("failed to create FUSE mountpoint");
    start_fuse(vfs, &mountpoint);
    eprintln!("[cov] FUSE mounted at {mountpoint}");

    // binary entries are `combined_N` (no extension); skip .json sidecars
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&corpus_dir)
        .expect("cannot read corpus dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("combined_") && !n.contains('.'))
                .unwrap_or(false)
        })
        .collect();

    entries.sort_by_key(|p| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("combined_"))
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(usize::MAX)
    });

    if let Some(cutoff) = before_epoch {
        let before = entries.len();
        entries.retain(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() <= cutoff)
                .unwrap_or(true) // keep if mtime unreadable
        });
        eprintln!(
            "[cov] --before-epoch {cutoff}: kept {}/{} entries (dropped {})",
            entries.len(),
            before,
            before - entries.len()
        );
    }

    eprintln!("[cov] {} corpus entries to replay", entries.len());
    if entries.is_empty() {
        eprintln!("ERROR: no corpus entries found in {}", corpus_dir.display());
        std::process::exit(1);
    }

    let mut ok = 0usize;
    let mut skip = 0usize;
    let mut timeout = 0usize;

    for (i, path) in entries.iter().enumerate() {
        let input = match CombinedInput::from_file(path) {
            Ok(inp) => inp,
            Err(_) => { skip += 1; continue; }
        };

        unsafe { vfs_reset_to_snapshot(vfs) };
        let _ = apply_delta(vfs, &input.rootfs);

        let mut conv = NautilusBytesConverter::new(context);
        let raw = conv.to_target_bytes(&input.config);

        let cfg = match override_rootfs_path(&*raw, &mountpoint) {
            Some(c) => c,
            None => { skip += 1; continue; }
        };

        let cfg_path = workdir.join(format!("config_{i}.json"));
        if std::fs::write(&cfg_path, &cfg).is_err() { skip += 1; continue; }

        // index, not %p, since %p needs LLVM runtime support
        let prof_file = profile_dir.join(format!("crun_{i}.profraw"));

        let status = Command::new(harness)
            .arg(&cfg_path)
            .current_dir(&workdir)
            .env("LLVM_PROFILE_FILE", &prof_file)
            .output();

        match status {
            Ok(_) => ok += 1,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => { timeout += 1; }
            Err(_) => { timeout += 1; }
        }

        if (i + 1) % 50 == 0 {
            eprintln!(
                "[cov] {}/{} — ok={ok} skip={skip} timeout={timeout}",
                i + 1, entries.len()
            );
        }
    }

    eprintln!("\n[cov] Done: ok={ok} skip={skip} timeout={timeout}");
    eprintln!("[cov] Profile files in: {}", profile_dir.display());
    eprintln!("\nNext steps:");
    eprintln!(
        "  {LLVM_PROFDATA} merge -sparse {}/*.profraw -o /tmp/cov_merged.profdata",
        profile_dir.display()
    );
    eprintln!(
        "  {LLVM_COV} report {harness} -instr-profile=/tmp/cov_merged.profdata -ignore-filename-regex='vendor|libocispec'"
    );
    eprintln!(
        "  {LLVM_COV} show {harness} -instr-profile=/tmp/cov_merged.profdata -format=html -output-dir=/tmp/cov_report"
    );
    eprintln!("  python3 -m http.server 8099 --directory /tmp/cov_report");

    std::fs::remove_dir_all(&workdir).ok();
    // FUSE torn down on process exit
}
