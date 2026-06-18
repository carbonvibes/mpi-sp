use std::borrow::Cow;

use libafl::{executors::ExitKind, observers::Observer, Error};
use libafl_bolts::Named;
use serde::{Deserialize, Serialize};

use crate::{
    ffi::{
        fuse_log_clear, fuse_log_get, fuse_log_set_active, FUSE_LOG_CREATE, FUSE_LOG_ENOENT,
        FUSE_LOG_MKDIR, FUSE_LOG_PATH_MAX, FUSE_LOG_RENAME_FROM, FUSE_LOG_RENAME_TO,
        FUSE_LOG_RMDIR, FUSE_LOG_SYMLINK, FUSE_LOG_UNLINK, FUSE_LOG_WRITE,
    },
    guidance::{update_live, MutationGuidance},
};

/// Observer that gates per-iteration FUSE access logging.
///
/// In `pre_exec`: clears the log and marks the target as running so FUSE
/// callbacks start appending entries.
/// In `post_exec`: stops logging, drains the log into `MutationGuidance`,
/// and publishes it via `guidance::update_live` so mutators can read it.
///
/// The observer must be listed BEFORE other observers in `tuple_list!` so
/// its `pre_exec` fires first and its `post_exec` fires last.
#[derive(Debug, Serialize, Deserialize)]
pub struct FuseLogObserver {
    name: Cow<'static, str>,
}

impl FuseLogObserver {
    pub fn new() -> Self {
        Self {
            name: Cow::Borrowed("fuse_log"),
        }
    }
}

impl Named for FuseLogObserver {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<I, S> Observer<I, S> for FuseLogObserver {
    fn pre_exec(&mut self, _state: &mut S, _input: &I) -> Result<(), Error> {
        unsafe {
            fuse_log_clear();
            fuse_log_set_active(1);
        }
        Ok(())
    }

    fn post_exec(
        &mut self,
        _state: &mut S,
        _input: &I,
        _exit_kind: &ExitKind,
    ) -> Result<(), Error> {
        unsafe { fuse_log_set_active(0) };

        let log = unsafe { &*fuse_log_get() };
        let n = log.n.max(0) as usize;

        let mut guidance = MutationGuidance::none();

        for i in 0..n.min(512) {
            let entry = &log.entries[i];
            let path = {
                let bytes = &entry.path[..FUSE_LOG_PATH_MAX];
                let end = bytes
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(FUSE_LOG_PATH_MAX);
                String::from_utf8_lossy(&bytes[..end]).into_owned()
            };

            match entry.kind {
                FUSE_LOG_ENOENT => guidance.enoent_paths.push(path),
                FUSE_LOG_WRITE
                | FUSE_LOG_CREATE
                | FUSE_LOG_RENAME_TO
                | FUSE_LOG_MKDIR
                | FUSE_LOG_SYMLINK => guidance.write_paths.push(path),
                FUSE_LOG_UNLINK | FUSE_LOG_RMDIR | FUSE_LOG_RENAME_FROM => {
                    guidance.recreate_paths.push(path)
                }
                _ => {}
            }
        }

        update_live(guidance);
        Ok(())
    }
}
