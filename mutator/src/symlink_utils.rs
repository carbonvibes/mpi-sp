use crate::delta::FsOp;
use crate::ffi::{enumerate_vfs_all_paths, enumerate_vfs_dir_paths, enumerate_vfs_symlink_paths, VfsT};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathKind {
    File,
    Dir,
    Symlink,
}

#[derive(Clone, Debug)]
pub struct PathInfo {
    pub path: String,
    pub kind: PathKind,
    /// direct children, non-recursive; empty for File/Symlink
    pub children: Vec<String>,
}

/// pre-computed index of the baseline VFS tree, so mutators don't need live VFS access
#[derive(Clone, Debug, Default)]
pub struct BaselineIndex {
    pub entries: Vec<PathInfo>,
}

impl BaselineIndex {
    /// call once at startup; `vfs` must be a valid initialized pointer
    pub fn build(vfs: *mut VfsT) -> Self {
        let all = enumerate_vfs_all_paths(vfs);
        let dirs = enumerate_vfs_dir_paths(vfs);
        let symlinks = enumerate_vfs_symlink_paths(vfs);

        let dir_set: std::collections::HashSet<&str> =
            dirs.iter().map(String::as_str).collect();
        let sym_set: std::collections::HashSet<&str> =
            symlinks.iter().map(String::as_str).collect();

        let mut entries: Vec<PathInfo> = all
            .iter()
            .map(|p| {
                let kind = if sym_set.contains(p.as_str()) {
                    PathKind::Symlink
                } else if dir_set.contains(p.as_str()) {
                    PathKind::Dir
                } else {
                    PathKind::File
                };
                PathInfo {
                    path: p.clone(),
                    kind,
                    children: vec![],
                }
            })
            .collect();

        // direct child if parent == dirname(path)
        for i in 0..entries.len() {
            let path = entries[i].path.clone();
            let parent = parent_path(&path);
            if let Some(parent_str) = parent {
                if let Some(pe) = entries.iter_mut().find(|e| e.path == parent_str) {
                    pe.children.push(path);
                }
            }
        }

        Self { entries }
    }

    pub fn get(&self, path: &str) -> Option<&PathInfo> {
        self.entries.iter().find(|e| e.path == path)
    }

    pub fn is_dir(&self, path: &str) -> bool {
        self.get(path)
            .map(|e| e.kind == PathKind::Dir)
            .unwrap_or(false)
    }

    pub fn is_file(&self, path: &str) -> bool {
        self.get(path)
            .map(|e| e.kind == PathKind::File)
            .unwrap_or(false)
    }

    pub fn children_of(&self, path: &str) -> Vec<String> {
        self.get(path)
            .map(|e| e.children.clone())
            .unwrap_or_default()
    }
}

fn parent_path(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    let trimmed = path.trim_end_matches('/');
    let last_slash = trimmed.rfind('/')?;
    if last_slash == 0 {
        Some("/".to_string())
    } else {
        Some(trimmed[..last_slash].to_string())
    }
}

/// expands into the FsOps needed to turn `path` into a symlink, deleting whatever
/// the baseline index says is there first (children deleted postorder for dirs)
pub fn replace_with_symlink(path: &str, target: &str, index: &BaselineIndex) -> Vec<FsOp> {
    match index.get(path) {
        None => {
            // not in baseline; just create (may EEXIST, fine)
            vec![FsOp::create_symlink(path, target)]
        }
        Some(info) => match info.kind {
            PathKind::File | PathKind::Symlink => {
                vec![
                    FsOp::delete_file(path),
                    FsOp::create_symlink(path, target),
                ]
            }
            PathKind::Dir => {
                let mut ops = Vec::new();
                delete_tree_ops(path, index, &mut ops);
                ops.push(FsOp::create_symlink(path, target));
                ops
            }
        },
    }
}

/// deletes dir contents depth-first (deepest first) then rmdirs it
fn delete_tree_ops(dir: &str, index: &BaselineIndex, ops: &mut Vec<FsOp>) {
    let children = index.children_of(dir);
    for child in &children {
        if let Some(info) = index.get(child) {
            match info.kind {
                PathKind::Dir => delete_tree_ops(child, index, ops),
                PathKind::File | PathKind::Symlink => ops.push(FsOp::delete_file(child)),
            }
        }
    }
    ops.push(FsOp::rmdir(dir));
}

pub fn replace_file_with_symlink(path: &str, target: &str) -> Vec<FsOp> {
    vec![FsOp::delete_file(path), FsOp::create_symlink(path, target)]
}

pub fn replace_empty_dir_with_symlink(path: &str, target: &str) -> Vec<FsOp> {
    vec![FsOp::rmdir(path), FsOp::create_symlink(path, target)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::FsOpKind;
    use crate::ffi::{vfs_create, vfs_create_file, vfs_destroy, vfs_mkdir, vfs_save_snapshot};

    unsafe fn make_test_vfs() -> *mut VfsT {
        let vfs = vfs_create();
        assert!(!vfs.is_null());
        // dir with children
        vfs_mkdir(vfs, c"/etc".as_ptr());
        vfs_create_file(vfs, c"/etc/passwd".as_ptr(), b"root:x:0\n".as_ptr(), 9);
        vfs_create_file(vfs, c"/etc/group".as_ptr(), b"root:x:0:\n".as_ptr(), 10);
        // empty dir
        vfs_mkdir(vfs, c"/proc".as_ptr());
        vfs_create_file(vfs, c"/input".as_ptr(), b"seed".as_ptr(), 4);
        vfs_save_snapshot(vfs);
        vfs
    }

    #[test]
    fn baseline_index_classifies_kinds_correctly() {
        let vfs = unsafe { make_test_vfs() };
        let idx = BaselineIndex::build(vfs);

        assert!(idx.is_dir("/etc"), "/etc should be Dir");
        assert!(idx.is_dir("/proc"), "/proc should be Dir");
        assert!(idx.is_file("/etc/passwd"), "/etc/passwd should be File");
        assert!(idx.is_file("/input"), "/input should be File");

        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn baseline_index_children_populated() {
        let vfs = unsafe { make_test_vfs() };
        let idx = BaselineIndex::build(vfs);

        let children = idx.children_of("/etc");
        assert!(
            children.contains(&"/etc/passwd".to_string()),
            "children_of /etc should include /etc/passwd"
        );
        assert!(
            children.contains(&"/etc/group".to_string()),
            "children_of /etc should include /etc/group"
        );

        let proc_children = idx.children_of("/proc");
        assert!(proc_children.is_empty(), "/proc children should be empty");

        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn replace_with_symlink_file_emits_delete_then_create() {
        let vfs = unsafe { make_test_vfs() };
        let idx = BaselineIndex::build(vfs);

        let ops = replace_with_symlink("/input", "../../input", &idx);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].kind, FsOpKind::DeleteFile);
        assert_eq!(ops[0].path, "/input");
        assert_eq!(ops[1].kind, FsOpKind::CreateSymlink);
        assert_eq!(ops[1].path, "/input");
        assert_eq!(ops[1].target, "../../input");

        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn replace_with_symlink_empty_dir_emits_rmdir_then_create() {
        let vfs = unsafe { make_test_vfs() };
        let idx = BaselineIndex::build(vfs);

        let ops = replace_with_symlink("/proc", "../../proc", &idx);
        assert_eq!(ops.len(), 2, "empty dir: just rmdir + create_symlink");
        assert_eq!(ops[0].kind, FsOpKind::Rmdir);
        assert_eq!(ops[0].path, "/proc");
        assert_eq!(ops[1].kind, FsOpKind::CreateSymlink);
        assert_eq!(ops[1].target, "../../proc");

        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn replace_with_symlink_nonempty_dir_deletes_children_first() {
        let vfs = unsafe { make_test_vfs() };
        let idx = BaselineIndex::build(vfs);

        let ops = replace_with_symlink("/etc", "../../etc", &idx);
        assert!(ops.len() >= 3, "non-empty dir needs child deletions + rmdir + symlink");

        let last = ops.last().unwrap();
        assert_eq!(last.kind, FsOpKind::CreateSymlink);
        assert_eq!(last.path, "/etc");
        assert_eq!(last.target, "../../etc");

        let rmdir = &ops[ops.len() - 2];
        assert_eq!(rmdir.kind, FsOpKind::Rmdir);
        assert_eq!(rmdir.path, "/etc");

        for op in &ops[..ops.len() - 2] {
            assert_eq!(op.kind, FsOpKind::DeleteFile);
            assert!(op.path.starts_with("/etc/"));
        }

        unsafe { vfs_destroy(vfs) };
    }

    #[test]
    fn replace_with_symlink_unknown_path_just_creates() {
        let vfs = unsafe { make_test_vfs() };
        let idx = BaselineIndex::build(vfs);

        let ops = replace_with_symlink("/nonexistent", "/proc/self/exe", &idx);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].kind, FsOpKind::CreateSymlink);
        assert_eq!(ops[0].target, "/proc/self/exe");

        unsafe { vfs_destroy(vfs) };
    }
}
