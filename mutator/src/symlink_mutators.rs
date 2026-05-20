use std::borrow::Cow;
use std::sync::Arc;

use libafl::{
    corpus::CorpusId,
    mutators::{MutationResult, Mutator},
    state::HasRand,
    Error,
};
use libafl_bolts::{rands::Rand, Named};

use crate::{
    delta::{FsDelta, FsOp},
    mutators::MAX_OPS,
    symlink_utils::{replace_with_symlink, BaselineIndex},
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn nz(v: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(v.max(1)).unwrap()
}

fn pick<'a, R: Rand, T>(rng: &mut R, slice: &'a [T]) -> &'a T {
    &slice[rng.below(nz(slice.len()))]
}

/// Mount destinations crun always sets up pre-pivot.
static MOUNT_DESTINATIONS: &[&str] = &["/proc", "/dev", "/sys", "/tmp"];

/// Relative escape targets (suffix after `../`-chain).
static RELATIVE_TARGETS: &[&str] = &[
    "etc/passwd",
    "etc/shadow",
    "proc/self/exe",
    "proc/self/mem",
    "proc/sysrq-trigger",
    "dev/sda",
    "dev/zero",
    "run/containerd",
    "var/run/docker.sock",
    "proc/self/fd",
];

/// Absolute symlink targets — direct escapes if crun resolves from host context.
static ABSOLUTE_TARGETS: &[&str] = &[
    "/proc",
    "/proc/self",
    "/proc/self/exe",
    "/proc/self/fd",
    "/proc/self/fd/0",
    "/dev",
    "/dev/null",
    "/sys",
    "/etc/passwd",
    "/bin/sh",
];

/// Parent components crun is likely to access during container setup.
static PARENT_COMPONENTS: &[&str] = &[
    "/etc",
    "/bin",
    "/lib",
    "/usr",
    "/dev",
    "/var",
    "/run",
];

/// Synthetic executable paths to create as symlinks.
static EXEC_PATHS: &[&str] = &[
    "/bin/target",
    "/usr/local/bin/target",
    "/bin/exploit",
    "/sbin/target",
];

/// Interesting targets for an executable-path symlink.
static EXEC_SYMLINK_TARGETS: &[&str] = &[
    "/proc/self/exe",
    "/proc/self/mem",
    "/proc/self/fd/0",
    "/dev/zero",
    "/dev/null",
    "../../usr/bin/python3",
    "../../bin/sh",
    "/nonexistent",
];

// ── 1. MountDestinationSymlinkMutator ────────────────────────────────────────

/// Replaces crun's mandatory mount destinations (proc, dev, sys, tmp) with
/// symlinks using `replace_with_symlink`. Fires on every corpus entry.
pub struct MountDestinationSymlinkMutator {
    pub index: Arc<BaselineIndex>,
}

impl MountDestinationSymlinkMutator {
    pub fn new(index: Arc<BaselineIndex>) -> Self {
        Self { index }
    }
}

impl Named for MountDestinationSymlinkMutator {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("MountDestinationSymlinkMutator");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for MountDestinationSymlinkMutator
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }

        let path = *pick(state.rand_mut(), MOUNT_DESTINATIONS);
        let r = state.rand_mut().below(nz(100));

        let target = if r < 35 {
            // Relative escape to same-named host path
            let depth = 2 + state.rand_mut().below(nz(4));
            format!("{}{}", "../".repeat(depth), &path[1..])
        } else if r < 60 {
            // Absolute target
            (*pick(state.rand_mut(), ABSOLUTE_TARGETS)).to_string()
        } else if r < 80 {
            // Cross-type: symlink to wrong-type path
            let wrong = if path == "/proc" { "/etc/passwd" } else { "/bin/true" };
            wrong.to_string()
        } else if r < 95 {
            // Dangling
            "/nonexistent".to_string()
        } else {
            // Special proc/dev special file
            (*pick(state.rand_mut(), &["/dev/null", "/proc/self/fd"])).to_string()
        };

        let new_ops = replace_with_symlink(path, &target, &self.index);
        let available = MAX_OPS.saturating_sub(input.ops.len());
        if new_ops.len() > available {
            return Ok(MutationResult::Skipped);
        }
        input.ops.extend(new_ops);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ── 2. MountOptionSymlinkMutator ─────────────────────────────────────────────

/// Creates symlinks at paths involved in bind mounts (rootfs/destination side).
/// Tests crun's dest-nofollow and symlink-aware bind mount handling.
///
/// Source-side options (copy-symlink, src-nofollow) require a host-side symlink
/// as mount.source — that coordination belongs in a CombinedInput mutator.
/// This mutator handles the rootfs (container) destination side.
pub struct MountOptionSymlinkMutator {
    pub index: Arc<BaselineIndex>,
}

impl MountOptionSymlinkMutator {
    pub fn new(index: Arc<BaselineIndex>) -> Self {
        Self { index }
    }
}

impl Named for MountOptionSymlinkMutator {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("MountOptionSymlinkMutator");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for MountOptionSymlinkMutator
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }

        // Pick a destination path for the bind mount
        let dst = *pick(state.rand_mut(), MOUNT_DESTINATIONS);

        // Pick a symlink target — bias toward interesting special files
        let target = *pick(state.rand_mut(), &[
            "/proc/self/fd",
            "/proc/self/exe",
            "/dev/null",
            "../../tmp",
            "/nonexistent",
        ]);

        let new_ops = replace_with_symlink(dst, target, &self.index);
        let available = MAX_OPS.saturating_sub(input.ops.len());
        if new_ops.len() > available {
            return Ok(MutationResult::Skipped);
        }
        input.ops.extend(new_ops);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ── 3. ExecutableSymlinkMutator ───────────────────────────────────────────────

/// Creates a symlink at a synthetic executable path. Explores the "config says
/// run X, rootfs has X as a symlink" scenario (rootfs side only; config
/// coordination via override_args requires a CombinedInput mutator).
pub struct ExecutableSymlinkMutator {
    pub index: Arc<BaselineIndex>,
}

impl ExecutableSymlinkMutator {
    pub fn new(index: Arc<BaselineIndex>) -> Self {
        Self { index }
    }
}

impl Named for ExecutableSymlinkMutator {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("ExecutableSymlinkMutator");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for ExecutableSymlinkMutator
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }

        let exec_path = *pick(state.rand_mut(), EXEC_PATHS);
        let target = *pick(state.rand_mut(), EXEC_SYMLINK_TARGETS);

        let new_ops = replace_with_symlink(exec_path, target, &self.index);
        let available = MAX_OPS.saturating_sub(input.ops.len());
        if new_ops.len() > available {
            return Ok(MutationResult::Skipped);
        }
        input.ops.extend(new_ops);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ── 4. ParentComponentSymlinkMutator ─────────────────────────────────────────

/// Replaces a *non-leaf* path component with a symlink — the historically
/// dangerous class for container escapes (e.g. /etc → ../../etc means
/// /etc/passwd resolves on the host).
pub struct ParentComponentSymlinkMutator {
    pub index: Arc<BaselineIndex>,
}

impl ParentComponentSymlinkMutator {
    pub fn new(index: Arc<BaselineIndex>) -> Self {
        Self { index }
    }
}

impl Named for ParentComponentSymlinkMutator {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("ParentComponentSymlinkMutator");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for ParentComponentSymlinkMutator
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }

        let component = *pick(state.rand_mut(), PARENT_COMPONENTS);
        let depth = 2 + state.rand_mut().below(nz(3));

        // 50% relative escape, 50% absolute target
        let target = if state.rand_mut().below(nz(2)) == 0 {
            format!("{}{}", "../".repeat(depth), &component[1..])
        } else {
            (*pick(state.rand_mut(), ABSOLUTE_TARGETS)).to_string()
        };

        let new_ops = replace_with_symlink(component, &target, &self.index);
        let available = MAX_OPS.saturating_sub(input.ops.len());
        if new_ops.len() > available {
            return Ok(MutationResult::Skipped);
        }
        input.ops.extend(new_ops);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ── 5. SymlinkEscapeMutator ───────────────────────────────────────────────────

/// Generates symlink targets crafted to escape the rootfs, both relative
/// (via `../`-chains) and absolute (direct host paths). Applied at a random
/// existing path or at a synthetic new path.
pub struct SymlinkEscapeMutator {
    pub index: Arc<BaselineIndex>,
}

impl SymlinkEscapeMutator {
    pub fn new(index: Arc<BaselineIndex>) -> Self {
        Self { index }
    }
}

impl Named for SymlinkEscapeMutator {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("SymlinkEscapeMutator");
        &N
    }
}

impl<S> Mutator<FsDelta, S> for SymlinkEscapeMutator
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }

        // 40% absolute, 60% relative
        let target = if state.rand_mut().below(nz(100)) < 40 {
            (*pick(state.rand_mut(), ABSOLUTE_TARGETS)).to_string()
        } else {
            let depth = 2 + state.rand_mut().below(nz(7)); // 2–8 hops
            let suffix = *pick(state.rand_mut(), RELATIVE_TARGETS);
            format!("{}{}", "../".repeat(depth), suffix)
        };

        // 60% synthetic path, 40% replace an existing path from index
        let path = if state.rand_mut().below(nz(100)) < 60 || self.index.entries.is_empty() {
            // Synthetic path at a crun-relevant location
            let base = *pick(state.rand_mut(), &["/bin", "/etc", "/lib", "/proc", "/dev"]);
            format!("{}/escape", base)
        } else {
            let entry = pick(state.rand_mut(), &self.index.entries);
            entry.path.clone()
        };

        let new_ops = replace_with_symlink(&path, &target, &self.index);
        let available = MAX_OPS.saturating_sub(input.ops.len());
        if new_ops.len() > available {
            return Ok(MutationResult::Skipped);
        }
        input.ops.extend(new_ops);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

// ── 6. LoopAndDepthMutator ────────────────────────────────────────────────────

/// Creates symlink loops and chains. Useful for ELOOP robustness testing.
/// Lower mutation weight than the crun-specific mutators above.
pub struct LoopAndDepthMutator;

impl LoopAndDepthMutator {
    pub fn new() -> Self {
        Self
    }
}

impl Named for LoopAndDepthMutator {
    fn name(&self) -> &Cow<'static, str> {
        static N: Cow<'static, str> = Cow::Borrowed("LoopAndDepthMutator");
        &N
    }
}

static CHAIN_LENGTHS: &[usize] = &[1, 5, 10, 39, 40, 41];

impl<S> Mutator<FsDelta, S> for LoopAndDepthMutator
where
    S: HasRand,
{
    fn mutate(&mut self, state: &mut S, input: &mut FsDelta) -> Result<MutationResult, Error> {
        if input.ops.len() >= MAX_OPS {
            return Ok(MutationResult::Skipped);
        }

        let r = state.rand_mut().below(nz(100));

        if r < 20 {
            // Self-loop
            let path = "/fuzz_loop";
            if input.ops.len() + 1 > MAX_OPS {
                return Ok(MutationResult::Skipped);
            }
            input.ops.push(FsOp::create_symlink(path, path));
        } else if r < 40 {
            // Two-cycle: /a -> /b, /b -> /a
            if input.ops.len() + 2 > MAX_OPS {
                return Ok(MutationResult::Skipped);
            }
            input.ops.push(FsOp::create_symlink("/fuzz_a", "/fuzz_b"));
            input.ops.push(FsOp::create_symlink("/fuzz_b", "/fuzz_a"));
        } else if r < 70 {
            // Chain of N symlinks anchored at a crun-relevant path
            let n = *pick(state.rand_mut(), CHAIN_LENGTHS);
            let anchors = ["/proc", "/dev", "/bin/true"];
            let anchor = *pick(state.rand_mut(), &anchors);
            let available = MAX_OPS.saturating_sub(input.ops.len());
            if n > available {
                return Ok(MutationResult::Skipped);
            }
            // s0 -> s1 -> s2 -> ... -> anchor
            for i in (1..n).rev() {
                input
                    .ops
                    .push(FsOp::create_symlink(format!("/s{}", i), format!("/s{}", i + 1)));
            }
            input.ops.push(FsOp::create_symlink("/s0", anchor));
        } else if r < 85 {
            // Long target string near PATH_MAX
            let long_target = format!("{}{}", "../".repeat(20), "proc/self/exe");
            if input.ops.len() + 1 > MAX_OPS {
                return Ok(MutationResult::Skipped);
            }
            input
                .ops
                .push(FsOp::create_symlink("/fuzz_long", &long_target));
        } else {
            // Repeated slashes in target
            if input.ops.len() + 1 > MAX_OPS {
                return Ok(MutationResult::Skipped);
            }
            input
                .ops
                .push(FsOp::create_symlink("/fuzz_slash", "////proc//self//exe"));
        }

        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::FsOpKind;
    use crate::ffi::{vfs_create, vfs_create_file, vfs_destroy, vfs_mkdir, vfs_save_snapshot};
    use libafl::state::HasRand;
    use libafl_bolts::rands::StdRand;

    struct MockState {
        rng: StdRand,
    }
    impl HasRand for MockState {
        type Rand = StdRand;
        fn rand(&self) -> &StdRand {
            &self.rng
        }
        fn rand_mut(&mut self) -> &mut StdRand {
            &mut self.rng
        }
    }

    unsafe fn make_test_vfs() -> *mut crate::ffi::VfsT {
        let vfs = vfs_create();
        vfs_mkdir(vfs, c"/proc".as_ptr());
        vfs_mkdir(vfs, c"/dev".as_ptr());
        vfs_mkdir(vfs, c"/sys".as_ptr());
        vfs_mkdir(vfs, c"/tmp".as_ptr());
        vfs_mkdir(vfs, c"/etc".as_ptr());
        vfs_create_file(vfs, c"/etc/passwd".as_ptr(), b"root\n".as_ptr(), 5);
        vfs_mkdir(vfs, c"/bin".as_ptr());
        vfs_create_file(vfs, c"/bin/true".as_ptr(), b"".as_ptr(), 0);
        vfs_save_snapshot(vfs);
        vfs
    }

    fn make_state() -> MockState {
        MockState {
            rng: StdRand::with_seed(42),
        }
    }

    #[test]
    fn mount_dest_mutator_appends_ops() {
        let vfs = unsafe { make_test_vfs() };
        let idx = Arc::new(BaselineIndex::build(vfs));
        let mut m = MountDestinationSymlinkMutator::new(idx);
        let mut state = make_state();
        let mut input = FsDelta::new(vec![FsOp::create_symlink("/x", "/y")]);

        let result = m.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, MutationResult::Mutated);
        assert!(
            input.ops.last().unwrap().kind == FsOpKind::CreateSymlink,
            "last op must be CreateSymlink"
        );
        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn parent_component_mutator_appends_ops() {
        let vfs = unsafe { make_test_vfs() };
        let idx = Arc::new(BaselineIndex::build(vfs));
        let mut m = ParentComponentSymlinkMutator::new(idx);
        let mut state = make_state();
        let mut input = FsDelta::new(vec![FsOp::create_symlink("/x", "/y")]);

        let result = m.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, MutationResult::Mutated);
        assert!(input.ops.len() >= 2);
        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn escape_mutator_produces_symlink() {
        let vfs = unsafe { make_test_vfs() };
        let idx = Arc::new(BaselineIndex::build(vfs));
        let mut m = SymlinkEscapeMutator::new(idx);
        let mut state = make_state();
        let mut input = FsDelta::new(vec![FsOp::create_symlink("/x", "/y")]);

        let result = m.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, MutationResult::Mutated);
        let last = input.ops.last().unwrap();
        assert_eq!(last.kind, FsOpKind::CreateSymlink);
        assert!(!last.target.is_empty(), "escape target must not be empty");
        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn loop_mutator_creates_symlinks() {
        let mut m = LoopAndDepthMutator::new();
        let mut state = make_state();
        let mut input = FsDelta::new(vec![FsOp::create_symlink("/x", "/y")]);

        let result = m.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, MutationResult::Mutated);
        assert!(input.ops.len() >= 2);
        // All new ops from loop mutator must be CreateSymlink
        for op in &input.ops[1..] {
            assert_eq!(op.kind, FsOpKind::CreateSymlink);
        }
    }

    #[test]
    fn exec_mutator_targets_synthetic_exec_path() {
        let vfs = unsafe { make_test_vfs() };
        let idx = Arc::new(BaselineIndex::build(vfs));
        let mut m = ExecutableSymlinkMutator::new(idx);
        let mut state = make_state();
        let mut input = FsDelta::new(vec![FsOp::create_symlink("/x", "/y")]);

        let result = m.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, MutationResult::Mutated);
        let last = input.ops.last().unwrap();
        assert_eq!(last.kind, FsOpKind::CreateSymlink);
        // Target must be one of the interesting exec targets
        assert!(!last.target.is_empty());
        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn mutators_skip_at_max_ops() {
        let vfs = unsafe { make_test_vfs() };
        let idx = Arc::new(BaselineIndex::build(vfs));
        let mut m = MountDestinationSymlinkMutator::new(Arc::clone(&idx));
        let mut state = make_state();
        let ops: Vec<FsOp> = (0..MAX_OPS)
            .map(|i| FsOp::create_symlink(format!("/x{}", i), "/y"))
            .collect();
        let mut input = FsDelta::new(ops);

        let result = m.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, MutationResult::Skipped);
        unsafe { vfs_destroy(vfs) };
    }
}
