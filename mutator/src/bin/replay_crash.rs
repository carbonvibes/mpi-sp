//! replay_crash — replay a crash file against the crun harness via the same
//! FUSE VFS pipeline the fuzzer uses.
//!
//! Binary mode (saved LibAFL crash corpus entry):
//!     sudo unshare -m ./replay_crash <crash_file> [grammar.py] [harness]
//!
//! JSON mode (file ends in .json): config.json is taken from the same dir as
//! ops.json unless a second .json arg is given. No grammar.py needed.
//!     sudo unshare -m ./replay_crash <ops.json> [config.json] [harness]
//!     sudo unshare -m ./replay_crash /tmp/c3_0/last_input.json

use std::ffi::CString;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use fs_mutator::delta::FsDelta;
use fs_mutator::ffi::{apply_delta, vfs_create, vfs_create_file, vfs_mkdir, VfsT};
use libafl::{
    generators::NautilusContext,
    inputs::{Input, NautilusBytesConverter, NautilusInput, ToTargetBytes},
};
use serde::{Deserialize, Serialize};

#[cfg(has_fuse3)]
use fs_mutator::ffi::{
    fuse_vfs_lib_init, fuse_vfs_lib_is_mounted, fuse_vfs_lib_run, fuse_vfs_lib_stop,
};

const DEFAULT_GRAMMAR: &str =
    "/nix/store/w6332yzld9fj1q6mymki1kmcvk2ks9y2-crun-fuzzer-0.0.1/share/grammar.py";
const DEFAULT_HARNESS: &str =
    "/nix/store/nm1sr5r2gzckh90y68avwa6fzp8hq83i-crun-harness-1.23.1/bin/crun";

// layout must match fuzz_combined_afl.rs and decode_crash.rs
#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
struct CombinedInput {
    config: NautilusInput,
    rootfs: FsDelta,
}

impl Input for CombinedInput {
    fn generate_name(&self, _idx: Option<libafl::corpus::CorpusId>) -> String {
        "replay".into()
    }
}

// must stay identical to init_vfs() in fuzz_combined_afl.rs
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
    mkfile!(c"/etc/hosts", b"127.0.0.1 localhost\n::1 localhost\n");
    mkfile!(c"/etc/hostname", b"fuzz\n");
    mkfile!(c"/etc/resolv.conf", b"nameserver 8.8.8.8\n");
}

#[cfg(has_fuse3)]
fn start_fuse(vfs: *mut VfsT, mountpoint: &str) {
    unsafe { fuse_vfs_lib_init(vfs) };
    let mp = CString::new(mountpoint).expect("mountpoint contains nul byte");
    thread::spawn(move || unsafe { fuse_vfs_lib_run(mp.as_ptr()) });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if unsafe { fuse_vfs_lib_is_mounted() } != 0 {
            break;
        }
        if Instant::now() > deadline {
            eprintln!("ERROR: FUSE mount timed out at {mountpoint}");
            std::process::exit(1);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(not(has_fuse3))]
fn start_fuse(_vfs: *mut VfsT, _mountpoint: &str) {
    eprintln!("ERROR: built without libfuse3-dev — cannot mount FUSE.");
    std::process::exit(1);
}

fn override_rootfs_path(json: &[u8], fuse_rootfs: &str) -> Option<Vec<u8>> {
    let mut v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let obj = v.as_object_mut()?;

    // inject a mount namespace, else crun mounts in the caller's namespace and a
    // symlink escape + generated mount can shadow the FUSE mountpoint.
    {
        let linux = obj
            .entry("linux")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(l) = linux.as_object_mut() {
            let ns = l
                .entry("namespaces")
                .or_insert_with(|| serde_json::json!([]));
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
        r.insert(
            "path".to_string(),
            serde_json::Value::String(fuse_rootfs.to_string()),
        );
    }
    serde_json::to_vec(&v).ok()
}

#[cfg(has_fuse3)]
fn stop_fuse(mountpoint: &str) {
    unsafe { fuse_vfs_lib_stop() };
    thread::sleep(Duration::from_millis(100));
    Command::new("fusermount3")
        .args(["-u", mountpoint])
        .output()
        .ok();
}

#[cfg(not(has_fuse3))]
fn stop_fuse(_mountpoint: &str) {}

fn main() {
    let mut args_iter = std::env::args().skip(1);
    let input_path = args_iter.next().unwrap_or_else(|| {
        eprintln!("Usage: sudo unshare -m replay_crash <crash_file|ops.json> [grammar.py|config.json] [harness]");
        std::process::exit(1);
    });

    let json_mode = input_path.ends_with(".json");

    let (delta, cfg_raw, harness, label) = if json_mode {
        // FsDelta from ops.json, OCI config from sibling config.json (or 2nd .json arg)
        let second = args_iter.next();
        let (cfg_path, harness) = match second {
            Some(s) if s.ends_with(".json") => (s, args_iter.next().unwrap_or_else(|| DEFAULT_HARNESS.to_string())),
            Some(s)                          => {
                // second arg is the harness binary, not a config
                let dir = std::path::Path::new(&input_path)
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("config.json");
                (dir.to_string_lossy().into_owned(), s)
            }
            None => {
                let dir = std::path::Path::new(&input_path)
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("config.json");
                (dir.to_string_lossy().into_owned(), DEFAULT_HARNESS.to_string())
            }
        };

        let ops_json = std::fs::read_to_string(&input_path).unwrap_or_else(|e| {
            eprintln!("Failed to read {input_path}: {e}");
            std::process::exit(1);
        });
        let delta: FsDelta = serde_json::from_str(&ops_json).unwrap_or_else(|e| {
            eprintln!("Failed to parse ops from {input_path}: {e}");
            std::process::exit(1);
        });

        let cfg_raw = std::fs::read(&cfg_path).unwrap_or_else(|e| {
            eprintln!("Failed to read config from {cfg_path}: {e}");
            std::process::exit(1);
        });

        eprintln!("[replay] JSON mode — ops: {input_path}  config: {cfg_path}");
        let label = std::path::Path::new(&input_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        (delta, cfg_raw, harness, label)
    } else {
        // binary mode: decode CombinedInput, render config via grammar
        let grammar_path = args_iter.next().unwrap_or_else(|| DEFAULT_GRAMMAR.to_string());
        let harness = args_iter.next().unwrap_or_else(|| DEFAULT_HARNESS.to_string());

        let context: &'static NautilusContext =
            Box::leak(Box::new(
                NautilusContext::from_file(100, &grammar_path).unwrap_or_else(|e| {
                    eprintln!("Failed to load grammar from {grammar_path}: {e}");
                    std::process::exit(1);
                }),
            ));

        let input = CombinedInput::from_file(&input_path).unwrap_or_else(|e| {
            eprintln!("Failed to decode {input_path}: {e}");
            std::process::exit(1);
        });

        let mut conv = NautilusBytesConverter::new(context);
        let raw = conv.to_target_bytes(&input.config);
        let label = std::path::Path::new(&input_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        (input.rootfs, raw.to_vec(), harness, label)
    };

    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");
    unsafe { init_vfs(vfs) };

    let dr = apply_delta(vfs, &delta);
    let (ok, fail) = match &dr {
        Ok(r) => (r.succeeded, r.failed),
        Err(_) => (0, delta.ops.len()),
    };
    eprintln!("[replay] {} ops: {} succeeded, {} failed", delta.ops.len(), ok, fail);

    let mountpoint = format!("/tmp/replay_fuse_{}", std::process::id());
    std::fs::create_dir_all(&mountpoint).expect("failed to create FUSE mountpoint dir");
    start_fuse(vfs, &mountpoint);
    eprintln!("[replay] FUSE mounted at {mountpoint}");

    let cfg_bytes = override_rootfs_path(&cfg_raw, &mountpoint)
        .unwrap_or_else(|| cfg_raw.clone());
    let config_path = format!("/tmp/replay_config_{}.json", std::process::id());
    std::fs::write(&config_path, &cfg_bytes).expect("failed to write config.json");
    eprintln!("[replay] config.json → {config_path}");
    eprintln!("[replay] config contents:\n{}", String::from_utf8_lossy(&cfg_bytes));

    let output = Command::new(&harness)
        .arg(&config_path)
        .output()
        .expect("failed to spawn harness");

    if !output.stderr.is_empty() {
        eprintln!("[replay] crun stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    }

    if let Some(sig) = output.status.signal() {
        println!("[{label}] ✓ CRASH confirmed — killed by signal {sig}");
    } else {
        println!("[{label}] ✗ No crash — exit code {}", output.status.code().unwrap_or(-1));
    }

    std::fs::remove_file(&config_path).ok();
    stop_fuse(&mountpoint);
    std::fs::remove_dir(&mountpoint).ok();
}
