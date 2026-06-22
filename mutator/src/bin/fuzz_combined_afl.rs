
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
    str::FromStr,
    thread,
    time::Duration,
};

use libafl::{
    BloomInputFilter, HasMetadata, StdFuzzerBuilder,
    corpus::{Corpus, CorpusId, OnDiskCorpus, Testcase},
    events::{ProgressReporter, SimpleEventManager},
    executors::{Executor, ExitKind, HasObservers, HasTimeout, StdChildArgs, forkserver::ForkserverExecutor},
    feedback_or,
    feedbacks::{CrashFeedback, MaxMapFeedback, TimeFeedback,
                NautilusChunksMetadata},
    fuzzer::{Evaluator, ExecuteInputResult, Fuzzer},
    generators::{Generator, NautilusContext, NautilusGenerator},
    inputs::{Input, NautilusBytesConverter, NautilusInput, ToTargetBytes},
    monitors::SimpleMonitor,
    mutators::{
        HavocScheduledMutator, MutationResult, Mutator,
        NautilusRandomMutator, NautilusRecursionMutator, NautilusSpliceMutator,
    },
    observers::{CanTrack, HitcountsMapObserver, ObserversTuple, StdMapObserver, TimeObserver},
    schedulers::{IndexesLenTimeMinimizerScheduler, QueueScheduler},
    stages::{AflStatsStage, StdMutationalStage},
    state::{HasCorpus, HasSolutions, StdState},
    Error,
};
use libafl::nautilus::grammartec::tree::TreeLike;
use libafl_bolts::{
    AsSliceMut, HasLen, Named, StdTargetArgs, Truncate, current_nanos,
    ownedref::OwnedSlice,
    rands::StdRand,
    shmem::{ShMem, ShMemProvider, UnixShMemProvider},
    tuples::{Handled, RefIndexable, tuple_list},
};
use nix::sys::signal::Signal;
use serde::{Deserialize, Serialize};

use std::sync::Arc;

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

#[cfg(has_fuse3)]
use fs_mutator::libafl_glue::fuse_log_observer::FuseLogObserver;

const MAP_SIZE: usize = 65536;

#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
pub struct CombinedInput {
    pub config: NautilusInput,
    pub rootfs: FsDelta,
}

impl Input for CombinedInput {
    fn generate_name(&self, idx: Option<CorpusId>) -> String {
        format!("combined_{}", idx.map(usize::from).unwrap_or(0))
    }
}

// op count + tree size as a proxy for input length, required by the minimizer scheduler
impl HasLen for CombinedInput {
    fn len(&self) -> usize {
        self.rootfs.len().saturating_add(self.config.tree.size())
    }
}


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


// Re-runs each crash and only forwards it if it reproduces. Saved inputs aren't
// self-contained reproducers (they depend on live FUSE state + fork residue), so
// a one-off signal death gets dropped instead of landing in crashes/.
struct CalibratedExecutor<E> {
    inner:     E,
    reruns:    usize,
    threshold: usize,
    // current input's rendered config, re-read on a crash (see is_real_crash)
    config_path: PathBuf,
}

// ForkserverExecutor only hands back ExitKind, which collapses every signal
// death into Crash — dig the actual signal out of its raw wait status.
trait TermSignal {
    fn term_signal(&self) -> Option<i32>;
}

impl<I, OT, S, SHM> TermSignal for ForkserverExecutor<I, OT, S, SHM>
where
    OT: ObserversTuple<I, S>,
    SHM: ShMem,
{
    fn term_signal(&self) -> Option<i32> {
        let status = self.forkserver().status();
        if libc::WIFSIGNALED(status) {
            Some(libc::WTERMSIG(status))
        } else {
            None
        }
    }
}

impl<E> CalibratedExecutor<E> {
    fn new(inner: E, reruns: usize, threshold: usize, config_path: PathBuf) -> Self {
        let total = reruns + 1;
        Self { inner, reruns, threshold: threshold.clamp(1, total), config_path }
    }
}

// Did the config itself ask for the kill behind `sig`? Then it's not a crun bug.
// A signal the config didn't request is kept — could be a real DoS.
fn config_requested_kill(config_path: &Path, sig: i32) -> bool {
    let Ok(bytes) = std::fs::read(config_path) else { return false };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return false };

    // this rlimit set to a 0..=2 budget (-1 is unlimited, doesn't count)
    let rlimit_near_zero = |rtype: &str| -> bool {
        v.pointer("/process/rlimits")
            .and_then(|r| r.as_array())
            .map_or(false, |rls| {
                rls.iter().any(|rl| {
                    rl.get("type").and_then(|t| t.as_str()) == Some(rtype)
                        && [rl.get("hard"), rl.get("soft")]
                            .iter()
                            .filter_map(|x| x.and_then(|v| v.as_i64()))
                            .any(|n| (0..=2).contains(&n))
                })
            })
    };

    match sig {
        // RLIMIT_CPU hard => SIGKILL, soft => SIGXCPU
        s if s == libc::SIGKILL || s == libc::SIGXCPU => rlimit_near_zero("RLIMIT_CPU"),
        s if s == libc::SIGXFSZ => rlimit_near_zero("RLIMIT_FSIZE"),
        // seccomp KILL default action
        s if s == libc::SIGSYS => matches!(
            v.pointer("/linux/seccomp/defaultAction").and_then(|a| a.as_str()),
            Some("SCMP_ACT_KILL" | "SCMP_ACT_KILL_PROCESS" | "SCMP_ACT_KILL_THREAD")
        ),
        _ => false,
    }
}

impl<E: TermSignal> CalibratedExecutor<E> {
    fn is_real_crash(&self, ek: ExitKind) -> bool {
        if ek != ExitKind::Crash {
            return false;
        }
        // a resource/seccomp signal counts as real only if the config didn't ask for it
        match self.inner.term_signal() {
            Some(s)
                if s == libc::SIGKILL
                    || s == libc::SIGXCPU
                    || s == libc::SIGXFSZ
                    || s == libc::SIGSYS =>
            {
                !config_requested_kill(&self.config_path, s)
            }
            _ => true,
        }
    }
}

impl<E, EM, I, S, Z> Executor<EM, I, S, Z> for CalibratedExecutor<E>
where
    E: Executor<EM, I, S, Z> + TermSignal,
{
    fn run_target(
        &mut self,
        fuzzer: &mut Z,
        state:  &mut S,
        mgr:    &mut EM,
        input:  &I,
    ) -> Result<ExitKind, Error> {
        let first = self.inner.run_target(fuzzer, state, mgr, input)?;
        if first != ExitKind::Crash {
            return Ok(first);
        }

        // config-requested kill: drop without wasting reruns
        if !self.is_real_crash(first) {
            eprintln!("[signal-filter] crash DROPPED — config-requested kill, not a crun bug");
            return Ok(ExitKind::Ok);
        }

        // re-run; only count fault-signal crashes (an input can flake between a
        // real fault and a config kill across runs)
        let mut crashes = 1usize;
        for _ in 0..self.reruns {
            let ek = self.inner.run_target(fuzzer, state, mgr, input)?;
            if self.is_real_crash(ek) {
                crashes += 1;
            }
        }

        let total = self.reruns + 1;
        if crashes >= self.threshold {
            eprintln!("[calibrate] crash CONFIRMED ({crashes}/{total} runs) — saving");
            Ok(ExitKind::Crash)
        } else {
            eprintln!("[calibrate] flaky crash DROPPED ({crashes}/{total} runs) — not saving");
            Ok(ExitKind::Ok)
        }
    }
}

impl<E> HasObservers for CalibratedExecutor<E>
where
    E: HasObservers,
{
    type Observers = E::Observers;
    fn observers(&self) -> RefIndexable<&Self::Observers, Self::Observers> {
        self.inner.observers()
    }
    fn observers_mut(&mut self) -> RefIndexable<&mut Self::Observers, Self::Observers> {
        self.inner.observers_mut()
    }
}

impl<E> HasTimeout for CalibratedExecutor<E>
where
    E: HasTimeout,
{
    fn timeout(&self) -> Duration {
        self.inner.timeout()
    }
    fn set_timeout(&mut self, timeout: Duration) {
        self.inner.set_timeout(timeout);
    }
}

struct CombinedConverter {
    context:         &'static NautilusContext,
    vfs:             *mut VfsT,
    config_path:     PathBuf,
    fuse_rootfs:     String,
    fallback_cfg:    Vec<u8>,
    last_input_path: PathBuf,
}

unsafe impl Send for CombinedConverter {}
unsafe impl Sync for CombinedConverter {}

impl ToTargetBytes<CombinedInput> for CombinedConverter {
    fn to_target_bytes<'a>(&mut self, input: &'a CombinedInput) -> OwnedSlice<'a, u8> {
        unsafe { vfs_reset_to_snapshot(self.vfs) };
        let _ = apply_delta(self.vfs, &input.rootfs);

        let mut bytes_conv = NautilusBytesConverter::new(self.context);
        let raw = bytes_conv.to_target_bytes(&input.config);

        let cfg = override_rootfs_path(&*raw, &self.fuse_rootfs)
            .unwrap_or_else(|| self.fallback_cfg.clone());

        let _ = std::fs::write(&self.config_path, &cfg);

        // keep current so a mid-run forkserver death leaves the exact input behind
        if let Ok(json) = serde_json::to_string_pretty(&input.rootfs) {
            let _ = std::fs::write(&self.last_input_path, json);
        }

        OwnedSlice::from(vec![0u8])
    }
}

/// Force "root.path" to the FUSE mountpoint. Returns None only if the JSON is completely unparseable.
fn override_rootfs_path(json: &[u8], fuse_rootfs: &str) -> Option<Vec<u8>> {
    let mut v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let obj = v.as_object_mut()?;

    // force a mount namespace — without one crun mounts in our namespace, where a
    // symlink+mount can shadow the FUSE mountpoint and take down the fuzzer
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

/// Writes a JSON sidecar next to each corpus entry for the dashboard to read.
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

/// Fallback OCI config used when the Nautilus-generated JSON can't be parsed.
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

unsafe fn init_vfs(vfs: *mut VfsT, bin_true: &[u8]) {
    for dir in &[
        c"/bin", c"/proc", c"/dev", c"/sys", c"/tmp", c"/etc", c"/var", c"/run",
        c"/usr", c"/usr/bin", c"/app", c"/home", c"/home/user",
    ] {
        vfs_mkdir(vfs, dir.as_ptr());
    }
    macro_rules! mkfile {
        ($path:expr, $content:expr) => {
            vfs_create_file(vfs, $path.as_ptr(), $content.as_ptr(), $content.len())
        };
    }
    // Every binary the grammar can reference gets the same static exit(0) blob
    // so find_executable succeeds regardless of which process path is generated.
    if !bin_true.is_empty() {
        mkfile!(c"/bin/true",       bin_true);
        mkfile!(c"/bin/sh",         bin_true);
        mkfile!(c"/bin/bash",       bin_true);
        mkfile!(c"/app/app",        bin_true);
        mkfile!(c"/usr/bin/nginx",  bin_true);
    }
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
        // corrupt ELF class field (offset 4)
        let mut c4 = bin_true.to_vec();
        c4[4] ^= 0xff;
        seeds.push(FsDelta::new(vec![FsOp::update_file("/bin/true", c4)]));

        // corrupt ELF endianness field (offset 5)
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

// raw rmdir + create_symlink silently fails on dirs with children.

fn crun_symlink_seeds(index: &BaselineIndex) -> Vec<FsDelta> {
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


    seeds.push(FsDelta::new(replace_with_symlink("/etc/passwd", "../../etc/passwd",      index)));
    seeds.push(FsDelta::new(replace_with_symlink("/etc/passwd", "/etc/passwd",           index)));
    seeds.push(FsDelta::new(replace_with_symlink("/etc/passwd", "../../../etc/shadow",   index)));
    seeds.push(FsDelta::new(replace_with_symlink("/etc/group",  "../../etc/group",       index)));


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

fn is_forkserver_death(e: &Error) -> bool {
    let s = format!("{e:?}");
    s.contains("Unable to communicate with fork server")
        || s.contains("Failed to start forkserver")
}

// Kill any crun processes whose cwd matches ours — fork-children that outlived the forkserver.
fn kill_stray_crun_in_cwd() {
    use nix::{sys::signal::Signal, unistd::Pid};
    let Ok(cwd) = std::env::current_dir() else { return };
    let Ok(rd) = std::fs::read_dir("/proc") else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some(pid_str) = name.to_str() else { continue };
        if !pid_str.bytes().all(|b| b.is_ascii_digit()) { continue; }
        let Ok(proc_cwd) = std::fs::read_link(format!("/proc/{pid_str}/cwd")) else { continue };
        if proc_cwd != cwd { continue; }
        let comm = std::fs::read_to_string(format!("/proc/{pid_str}/comm"))
            .unwrap_or_default();
        if comm.trim() != "crun" { continue; }
        let Ok(pid) = pid_str.parse::<i32>() else { continue };
        let _ = nix::sys::signal::kill(Pid::from_raw(pid), Signal::SIGKILL);
    }
}

fn main() {
    pyo3::prepare_freethreaded_python();

    let args: Vec<String> = std::env::args().collect();
    let resume = args.iter().any(|a| a == "--resume");
    let sync_dir: Option<PathBuf> = args.iter()
        .position(|a| a == "--sync-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    let instance_id: u32 = args.iter()
        .position(|a| a == "--instance")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // import from --sync-dir but never export (ASAN/UBSan read the base corpus)
    let no_export = args.iter().any(|a| a == "--no-export");
    let positional: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with("--")).collect();
    if positional.len() < 2 {
        eprintln!("Usage: {} <crun-afl-binary> <grammar.py> [--resume] [--sync-dir <path>] [--instance <N>]", args[0]);
        eprintln!("  Run as root from /tmp/campaign3/");
        eprintln!("  --resume            reload existing corpus instead of starting fresh");
        eprintln!("  --sync-dir <path>   shared dir for cross-instance corpus exchange");
        eprintln!("  --instance <N>      instance ID (0–N) used to name sync exports");
        eprintln!("  --no-export         import from --sync-dir but never export (read-only consumer)");
        std::process::exit(1);
    }
    let crun_binary  = positional[0];
    let grammar_path = PathBuf::from(positional[1]);
    let pid          = std::process::id();

    println!("=== fuzz_combined_afl: Campaign 3 — Nautilus config + FUSE rootfs ===");
    println!("  crun     : {crun_binary}");
    println!("  grammar  : {}", grammar_path.display());
    println!("  instance : {instance_id}");
    if let Some(ref sd) = sync_dir {
        println!("  sync-dir : {} ({})", sd.display(),
                 if no_export { "read-only: import, no export" } else { "read+write" });
    }

    let cwd = std::env::current_dir()
        .expect("cannot determine CWD — run from /tmp/campaign3/");
    let corpus_dir      = cwd.join("corpus");
    let solutions_dir   = cwd.join("crashes");
    let mountpoint      = format!("/tmp/campaign3-fuse-{pid}");
    let config_path     = cwd.join("config.json");
    let last_input_path = cwd.join("last_input.json");

    for d in &[&corpus_dir, &solutions_dir] {
        std::fs::create_dir_all(d).unwrap_or_else(|e| {
            eprintln!("ERROR: cannot create {}: {e}", d.display());
            std::process::exit(1);
        });
    }
    if let Some(ref sd) = sync_dir {
        std::fs::create_dir_all(sd).unwrap_or_else(|e| {
            eprintln!("ERROR: cannot create sync dir {}: {e}", sd.display());
            std::process::exit(1);
        });
    }
    std::fs::create_dir_all(&mountpoint).expect("failed to create FUSE mountpoint");

    // raw exit(0) syscall, gcc -static -nostartfiles -nostdlib -Os
    static BIN_TRUE: &[u8] = include_bytes!("../../static/true");

    let vfs = unsafe { vfs_create() };
    assert!(!vfs.is_null(), "vfs_create() returned null");
    unsafe { init_vfs(vfs, BIN_TRUE) };
    unsafe { vfs_save_snapshot(vfs) };

    let baseline_file_paths = enumerate_vfs_file_paths(vfs);
    let baseline_dir_paths  = enumerate_vfs_dir_paths(vfs);
    let baseline_all_paths  = enumerate_vfs_all_paths(vfs);
    let baseline_index      = Arc::new(BaselineIndex::build(vfs));
    let baseline_contents: Vec<(String, Vec<u8>)> = {
        let mut c = vec![("/etc/passwd".to_string(),
                          b"root:x:0:0:root:/root:/bin/sh\n".to_vec())];
        c.push(("/bin/true".to_string(), BIN_TRUE.to_vec()));
        c
    };

    start_fuse(vfs, &mountpoint);
    println!("  rootfs  : {mountpoint}");

    // Box::leak gives 'static lifetime so all components can borrow it freely
    let context: &'static NautilusContext =
        Box::leak(Box::new(NautilusContext::from_file(100, grammar_path).unwrap()));

    let fallback_cfg = make_fallback_config(&mountpoint);

    // shmem must outlive every executor rebuild — observers map it by raw ptr, so
    // the executor can be replaced on restart without disturbing the coverage map.
    let mut shmem_provider = UnixShMemProvider::new().unwrap();
    let mut shmem = shmem_provider.new_shmem(MAP_SIZE).unwrap();
    unsafe { shmem.write_to_env("__AFL_SHM_ID").unwrap() };
    let shmem_ptr: *mut u8 = shmem.as_slice_mut().as_mut_ptr();

    let edges_observer = unsafe {
        HitcountsMapObserver::new(
            StdMapObserver::from_mut_ptr("shared_mem", shmem_ptr, MAP_SIZE)
        ).track_indices()
    };
    let time_observer  = TimeObserver::new("time");
    let map_feedback   = MaxMapFeedback::new(&edges_observer);

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


    let fuse_log_observer = FuseLogObserver::new();

    // edge-coverage only. TimeFeedback just attaches timing, never admits.
    // (FsAccessFeedback used to live here but bloated the corpus without buying
    // edges; mutation guidance still comes from FuseLogObserver, not feedback.)
    let mut feedback = feedback_or!(
        MaxMapFeedback::new(&edges_observer),
        TimeFeedback::new(&time_observer),
    );
    let mut objective = CrashFeedback::new();

    let mut state = StdState::new(
        StdRand::with_seed(current_nanos()),
        OnDiskCorpus::<CombinedInput>::new(&corpus_dir).expect("corpus dir"),
        OnDiskCorpus::<CombinedInput>::new(&solutions_dir).expect("solutions dir"),
        &mut feedback,
        &mut objective,
    )
    .expect("StdState");

    let _ = state.metadata_or_insert_with::<NautilusChunksMetadata>(|| {
        NautilusChunksMetadata::new("/tmp/".into())
    });
    state.add_metadata(tokens.clone());

    let monitor = SimpleMonitor::new(|s| {
        println!("{s}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    });
    let mut mgr = SimpleEventManager::new(monitor);

    let observer_ref = edges_observer.handle();
    let scheduler    = IndexesLenTimeMinimizerScheduler::new(&edges_observer, QueueScheduler::new());

    let converter = CombinedConverter {
        context,
        vfs,
        config_path:     config_path.clone(),
        fuse_rootfs:     mountpoint.clone(),
        fallback_cfg:    fallback_cfg.clone(),
        last_input_path: last_input_path.clone(),
    };

    let mut fuzzer = StdFuzzerBuilder::new()
        .input_filter(BloomInputFilter::default())
        .target_bytes_converter(converter)
        .scheduler(scheduler)
        .feedback(feedback)
        .objective(objective)
        .build();

    // save a crash only if it reproduces >= threshold of (reruns+1) runs.
    // CRASH_CAL_THRESHOLD=1 disables the gate.
    let crash_cal_reruns: usize = std::env::var("CRASH_CAL_RERUNS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    let crash_cal_threshold: usize = std::env::var("CRASH_CAL_THRESHOLD")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    println!(
        "  crash calibration: reruns={crash_cal_reruns}, threshold={crash_cal_threshold} \
         (save iff >={crash_cal_threshold}/{} runs crash)",
        crash_cal_reruns + 1
    );

    let mut executor = ForkserverExecutor::builder()
        .program(crun_binary)
        .arg(config_path.to_str().expect("config path not UTF-8"))
        .debug_child(false)
        .coverage_map_size(MAP_SIZE)
        .timeout(Duration::from_millis(1200))
        .kill_signal(Signal::SIGKILL)
        // fuse_log_observer must be first so pre_exec fires before the target
        // starts and post_exec fires after it exits, publishing guidance.
        .build(tuple_list!(fuse_log_observer, time_observer, edges_observer))
        .expect("ForkserverExecutor build failed");

    if let Some(dynamic_map_size) = executor.coverage_map_size() {
        executor.observers_mut()[&observer_ref]
            .as_mut()
            .truncate(dynamic_map_size);
    }

    // wrap the forkserver in the calibration gate
    let mut executor = CalibratedExecutor::new(executor, crash_cal_reruns, crash_cal_threshold, config_path.clone());

    let mut r_seeds = rootfs_seeds(BIN_TRUE);
    r_seeds.extend(crun_symlink_seeds(&baseline_index));
    let live_corpus: LiveCorpus = Rc::new(RefCell::new(r_seeds.clone()));

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
        RootfsMutator::new(MountDestinationSymlinkMutator::new(Arc::clone(&baseline_index))),
        RootfsMutator::new(MountDestinationSymlinkMutator::new(Arc::clone(&baseline_index))),
        RootfsMutator::new(MountOptionSymlinkMutator::new(Arc::clone(&baseline_index))),
        RootfsMutator::new(ExecutableSymlinkMutator::new(Arc::clone(&baseline_index))),
        RootfsMutator::new(ParentComponentSymlinkMutator::new(Arc::clone(&baseline_index))),
        RootfsMutator::new(SymlinkEscapeMutator::new(Arc::clone(&baseline_index))),
        RootfsMutator::new(LoopAndDepthMutator::new()),
    );
    let scheduled   = HavocScheduledMutator::new(mutators);
    let havoc_stage = StdMutationalStage::new(scheduled);
    let mut stages  = tuple_list!(havoc_stage, afl_stats_stage);

    // seed both dimensions: N generated configs × empty rootfs,
    // plus the full rootfs seed set paired with the first generated config
    let mut generator = NautilusGenerator::new(context);
    let mut initial_configs: Vec<NautilusInput> = (0..32)
        .filter_map(|_| generator.generate(&mut state).ok())
        .collect();
    if initial_configs.is_empty() {
        panic!("NautilusGenerator failed to produce any configs — check grammar.py");
    }
    let baseline_config = initial_configs[0].clone();


    let mut seeds: Vec<CombinedInput> = initial_configs
        .drain(..)
        .map(|c| CombinedInput { config: c, rootfs: FsDelta::new(vec![]) })
        .collect();


    for r in r_seeds {
        seeds.push(CombinedInput { config: baseline_config.clone(), rootfs: r });
    }

    if resume {
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&corpus_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("combined_") && !n.contains('.'))
                    .unwrap_or(false)
            })
            .collect();
        paths.sort_by_key(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.strip_prefix("combined_"))
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(usize::MAX)
        });
        let mut resumed = 0usize;
        for path in &paths {
            let Ok(input) = CombinedInput::from_file(path) else { continue };
            let mut tc = Testcase::new(input.clone());
            *tc.file_path_mut() = Some(path.clone());
            if state.corpus_mut().add(tc).is_ok() {
                live_corpus.borrow_mut().push(input.rootfs.clone());
                if let Some(meta) = state.metadata_map_mut().get_mut::<NautilusChunksMetadata>() {
                    meta.cks.add_tree(input.config.tree.clone(), &context.ctx);
                }
                resumed += 1;
            }
        }
        println!("Resumed: {resumed} corpus entries reloaded from prior run");
    }

    if state.must_load_initial_inputs() {
        for seed in &seeds {
            let _ = fuzzer.add_input(&mut state, &mut executor, &mut mgr, seed.clone());
        }
    }

    // NautilusChunksMetadata must be populated manually — NautilusFeedback can't be used
    // with CombinedInput, so we replicate what NautilusFeedback.append_metadata does.
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


    // Track which sync-dir filenames this instance has already processed so we
    // never re-import our own exports or double-import a foreign entry.
    let mut seen_foreign: HashSet<String> = HashSet::new();
    let mut sync_tick: u64 = 0;

    let mut solutions_before = state.solutions().count();
    loop {
        let before = state.corpus().count();

        match fuzzer.fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr) {
            Ok(_) => {}
            Err(ref e) if is_forkserver_death(e) => {
                eprintln!(
                    "[forkserver] died ({e}) — killing stray crun, rebuilding \
                     (virgin map intact, no coverage loss)..."
                );
                kill_stray_crun_in_cwd();
                thread::sleep(Duration::from_millis(500));

                // shmem outlives the executor, so the map + __AFL_SHM_ID survive this
                drop(executor);

                let new_time_observer = TimeObserver::new("time");
                let new_edges_observer = unsafe {
                    HitcountsMapObserver::new(
                        StdMapObserver::from_mut_ptr("shared_mem", shmem_ptr, MAP_SIZE)
                    ).track_indices()
                };

                let mut rebuilt = ForkserverExecutor::builder()
                    .program(crun_binary)
                    .arg(config_path.to_str().expect("config path not UTF-8"))
                    .debug_child(false)
                    .coverage_map_size(MAP_SIZE)
                    .timeout(Duration::from_millis(1200))
                    .kill_signal(Signal::SIGKILL)
                    .build(tuple_list!(FuseLogObserver::new(), new_time_observer, new_edges_observer))
                    .expect("ForkserverExecutor rebuild failed");

                if let Some(dynamic_map_size) = rebuilt.coverage_map_size() {
                    rebuilt.observers_mut()[&observer_ref]
                        .as_mut()
                        .truncate(dynamic_map_size);
                }

                executor = CalibratedExecutor::new(rebuilt, crash_cal_reruns, crash_cal_threshold, config_path.clone());

                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let _ = std::fs::OpenOptions::new()
                    .create(true).append(true).open("restarts.log")
                    .and_then(|mut f| { use std::io::Write; writeln!(f, "{ts}") });

                eprintln!("[forkserver] restarted — continuing");
                continue;
            }
            Err(e) => panic!("fuzz_one failed: {e:?}"),
        }

        mgr.maybe_report_progress(&mut state, Duration::from_secs(2))
            .expect("progress report failed");


        let solutions_after = state.solutions().count();
        for idx in solutions_before..solutions_after {
            let cid = CorpusId::from(idx);
            if let Ok(input) = state.solutions().cloned_input_for_id(cid) {
                write_corpus_sidecar(&solutions_dir, idx, &input, context);
            }
        }
        solutions_before = solutions_after;

        // Sync new corpus entries into LiveCorpus and NautilusChunksMetadata,
        // and export each new discovery to the shared sync dir.
        let after = state.corpus().count();
        for idx in before..after {
            let cid = CorpusId::from(idx);
            if let Ok(input) = state.corpus().cloned_input_for_id(cid) {
                write_corpus_sidecar(&corpus_dir, idx, &input, context);
                live_corpus.borrow_mut().push(input.rootfs.clone());
                if let Some(meta) = state.metadata_map_mut().get_mut::<NautilusChunksMetadata>() {
                    meta.cks.add_tree(input.config.tree.clone(), &context.ctx);
                }
                // export for peers to import (skipped when --no-export)
                if let Some(sd) = sync_dir.as_ref().filter(|_| !no_export) {
                    let fname = format!("i{instance_id}_{idx}.sync");
                    let path  = sd.join(&fname);
                    if let Ok(json) = serde_json::to_vec(&input) {
                        let _ = std::fs::write(&path, json);
                    }
                    seen_foreign.insert(fname);  // don't re-import our own
                }
            }
        }

        // every 256 iters, pull in peers' new entries
        sync_tick += 1;
        if sync_tick % 256 == 0 {
            if let Some(ref sd) = sync_dir {
                let mut new_files: Vec<_> = std::fs::read_dir(sd)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        name.ends_with(".sync") && !seen_foreign.contains(&name)
                    })
                    .collect();
                new_files.sort_by_key(|e| e.file_name());

                // cap per tick so a backlog doesn't stall fuzzing
                for entry in new_files.into_iter().take(32) {
                    let fname = entry.file_name().to_string_lossy().into_owned();
                    seen_foreign.insert(fname.clone());
                    let Ok(bytes) = std::fs::read(entry.path()) else { continue };
                    let Ok(input) = serde_json::from_slice::<CombinedInput>(&bytes) else {
                        eprintln!("[sync] failed to deserialize {fname}");
                        continue;
                    };
                    // re-run locally; keep only if it hits a new edge here
                    match fuzzer.evaluate_input(&mut state, &mut executor, &mut mgr, &input) {
                        Ok((ExecuteInputResult::Corpus, Some(cid))) => {
                            let idx = usize::from(cid);
                            if let Ok(imported) = state.corpus().cloned_input_for_id(cid) {
                                write_corpus_sidecar(&corpus_dir, idx, &imported, context);
                                live_corpus.borrow_mut().push(imported.rootfs.clone());
                                if let Some(meta) = state.metadata_map_mut().get_mut::<NautilusChunksMetadata>() {
                                    meta.cks.add_tree(imported.config.tree.clone(), &context.ctx);
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("[sync] evaluate error for {fname}: {e}"),
                    }
                }
            }
        }
    }
}
