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

fn nz(v: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(v.max(1)).unwrap()
}

fn pick<'a, R: Rand, T>(rng: &mut R, slice: &'a [T]) -> &'a T {
    &slice[rng.below(nz(slice.len()))]
}

static MOUNT_DESTINATIONS: &[&str] = &["/proc", "/dev", "/sys", "/tmp"];

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

static PARENT_COMPONENTS: &[&str] = &[
    "/etc",
    "/bin",
    "/lib",
    "/usr",
    "/dev",
    "/var",
    "/run",
];

static EXEC_PATHS: &[&str] = &[
    "/bin/target",
    "/usr/local/bin/target",
    "/bin/exploit",
    "/sbin/target",
];

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
            let depth = 2 + state.rand_mut().below(nz(4));
            format!("{}{}", "../".repeat(depth), &path[1..])
        } else if r < 60 {
            (*pick(state.rand_mut(), ABSOLUTE_TARGETS)).to_string()
        } else if r < 80 {
            let wrong = if path == "/proc" { "/etc/passwd" } else { "/bin/true" };
            wrong.to_string()
        } else if r < 95 {
            "/nonexistent".to_string()
        } else {
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

        let dst = *pick(state.rand_mut(), MOUNT_DESTINATIONS);
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

        let target = if state.rand_mut().below(nz(100)) < 40 {
            (*pick(state.rand_mut(), ABSOLUTE_TARGETS)).to_string()
        } else {
            let depth = 2 + state.rand_mut().below(nz(7));
            let suffix = *pick(state.rand_mut(), RELATIVE_TARGETS);
            format!("{}{}", "../".repeat(depth), suffix)
        };

        let path = if state.rand_mut().below(nz(100)) < 60 || self.index.entries.is_empty() {
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
            // self-loop
            let path = "/fuzz_loop";
            if input.ops.len() + 1 > MAX_OPS {
                return Ok(MutationResult::Skipped);
            }
            input.ops.push(FsOp::create_symlink(path, path));
        } else if r < 40 {
            // two-cycle
            if input.ops.len() + 2 > MAX_OPS {
                return Ok(MutationResult::Skipped);
            }
            input.ops.push(FsOp::create_symlink("/fuzz_a", "/fuzz_b"));
            input.ops.push(FsOp::create_symlink("/fuzz_b", "/fuzz_a"));
        } else if r < 70 {
            // chain of N symlinks ending at a crun-relevant anchor
            let n = *pick(state.rand_mut(), CHAIN_LENGTHS);
            let anchors = ["/proc", "/dev", "/bin/true"];
            let anchor = *pick(state.rand_mut(), &anchors);
            let available = MAX_OPS.saturating_sub(input.ops.len());
            if n > available {
                return Ok(MutationResult::Skipped);
            }
            // s0 -> s1 -> ... -> anchor
            for i in (1..n).rev() {
                input
                    .ops
                    .push(FsOp::create_symlink(format!("/s{}", i), format!("/s{}", i + 1)));
            }
            input.ops.push(FsOp::create_symlink("/s0", anchor));
        } else if r < 85 {
            // long target near PATH_MAX
            let long_target = format!("{}{}", "../".repeat(20), "proc/self/exe");
            if input.ops.len() + 1 > MAX_OPS {
                return Ok(MutationResult::Skipped);
            }
            input
                .ops
                .push(FsOp::create_symlink("/fuzz_long", &long_target));
        } else {
            // repeated slashes in target
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
