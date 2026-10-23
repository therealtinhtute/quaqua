//! One cross-process lock for grid `apply_ops`.
//!
//! Two processes can apply layout ops to the same herdr daemon: the CLI
//! (`quaqua grid reconcile`, in-process `worker spawn/cleanup/prompt` checks)
//! and the herdr plugin hook. `apply_ops` parks a pane in a `grid-scratch-*`
//! tab before an in-tab split, so two applies on one tab interleave the park
//! and the split and corrupt each other (anchor in the scratch tab). Both
//! sides take this lock; `hook::acquire_lock` is the same primitive on an
//! explicit path.
//!
//! The lock is reentrant per process: a caller that already holds it (the
//! hook's apply path, or a check that runs inside it) gets a counted nested
//! guard instead of blocking on its own lock directory.

use crate::util::is_pid_alive;
use std::fs::{create_dir, create_dir_all, remove_dir, remove_file};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

/// `<state_dir>/lock` and `<state_dir>/lock/pid`, the paths the hook uses.
pub fn paths(state_dir: &Path) -> (PathBuf, PathBuf) {
    let lock_dir = state_dir.join("lock");
    let pid_file = lock_dir.join("pid");
    (lock_dir, pid_file)
}

/// Guards held by this process. A nested acquire must not touch the directory
/// (it would block on itself); it only bumps the count.
static DEPTH: Mutex<usize> = Mutex::new(0);

/// One lock test at a time: the depth counter and the state-dir override are
/// process-wide, so tests that exercise the lock must not overlap.
#[cfg(test)]
pub(crate) fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct ApplyLock {
    lock_dir: PathBuf,
    pid_file: PathBuf,
    /// The guard that created the directory; only it removes it.
    owns_dir: bool,
}

impl Drop for ApplyLock {
    fn drop(&mut self) {
        let mut depth = DEPTH.lock().unwrap_or_else(|e| e.into_inner());
        *depth = depth.saturating_sub(1);
        drop(depth);
        if self.owns_dir {
            let _ = remove_file(&self.pid_file);
            let _ = remove_dir(&self.lock_dir);
        }
    }
}

fn nested(lock_dir: &Path, pid_file: &Path) -> Option<ApplyLock> {
    let mut depth = DEPTH.lock().unwrap_or_else(|e| e.into_inner());
    if *depth > 0 {
        *depth += 1;
        return Some(ApplyLock {
            lock_dir: lock_dir.to_path_buf(),
            pid_file: pid_file.to_path_buf(),
            owns_dir: false,
        });
    }
    None
}

/// Remove a lock left behind by a process that no longer exists.
fn reclaim_dead(lock_dir: &Path, pid_file: &Path) {
    let holder = match std::fs::read_to_string(pid_file) {
        Ok(s) => s,
        Err(_) => return,
    };
    let holder = holder.trim();
    if !holder.is_empty() && !is_pid_alive(holder) {
        let _ = remove_file(pid_file);
        let _ = remove_dir(lock_dir);
    }
}

/// Take the lock if it is free. `Ok(None)` means another process holds it.
pub fn try_acquire_paths(lock_dir: &Path, pid_file: &Path) -> Result<Option<ApplyLock>, String> {
    if let Some(guard) = nested(lock_dir, pid_file) {
        return Ok(Some(guard));
    }
    if let Some(parent) = lock_dir.parent() {
        create_dir_all(parent)
            .map_err(|e| format!("cannot create lock parent {}: {e}", parent.display()))?;
    }
    match create_dir(lock_dir) {
        Ok(()) => {
            let _ = std::fs::write(pid_file, std::process::id().to_string());
            let mut depth = DEPTH.lock().unwrap_or_else(|e| e.into_inner());
            *depth += 1;
            drop(depth);
            Ok(Some(ApplyLock {
                lock_dir: lock_dir.to_path_buf(),
                pid_file: pid_file.to_path_buf(),
                owns_dir: true,
            }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            reclaim_dead(lock_dir, pid_file);
            Ok(None)
        }
        Err(e) => Err(format!(
            "cannot create grid apply lock {}: {e}",
            lock_dir.display()
        )),
    }
}

/// Wait for the lock up to the same ~5s the hook's acquisition allows.
pub fn acquire_paths(lock_dir: &Path, pid_file: &Path) -> Result<ApplyLock, String> {
    let mut attempts = 0;
    loop {
        if let Some(guard) = try_acquire_paths(lock_dir, pid_file)? {
            return Ok(guard);
        }
        attempts += 1;
        if attempts >= 100 {
            return Err(
                "[herdr-quaqua] could not acquire grid apply lock after timeout".to_string(),
            );
        }
        // A lock directory without a pid file is a racer between `create_dir`
        // and its pid write, unless it survives several polls.
        if attempts > 5 && !pid_file.is_file() {
            let _ = remove_file(pid_file);
            let _ = remove_dir(lock_dir);
            continue;
        }
        sleep(Duration::from_millis(50));
    }
}

/// Non-blocking acquire in the herdr plugin state dir (grid check polls).
pub fn try_acquire() -> Result<Option<ApplyLock>, String> {
    let sdir = super::hook::state_dir();
    let (lock_dir, pid_file) = paths(&sdir);
    try_acquire_paths(&lock_dir, &pid_file)
}

/// Blocking acquire in the herdr plugin state dir (grid repair).
pub fn acquire() -> Result<ApplyLock, String> {
    let sdir = super::hook::state_dir();
    let (lock_dir, pid_file) = paths(&sdir);
    acquire_paths(&lock_dir, &pid_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("quaqua-grid-lock-{name}-{}", std::process::id()));
        let _ = remove_dir(&dir);
        create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_apply_lock_is_reentrant_within_one_process() {
        let _serial = test_serial();
        // The hook holds the lock while it applies; a caller underneath it
        // must not block on the same lock directory.
        let dir = temp_dir("reentrant");
        let (lock_dir, pid_file) = paths(&dir);
        let outer = acquire_paths(&lock_dir, &pid_file).expect("first acquire");
        let inner = acquire_paths(&lock_dir, &pid_file)
            .expect("a caller that already holds the lock must not block on it");
        drop(inner);
        assert!(lock_dir.is_dir(), "the outer guard still owns the lock");
        drop(outer);
        assert!(!lock_dir.exists(), "the outer drop releases the lock");
        let _ = remove_dir(&dir);
    }

    #[test]
    fn test_apply_lock_is_released_by_the_outermost_guard() {
        let _serial = test_serial();
        let dir = temp_dir("release");
        let (lock_dir, pid_file) = paths(&dir);
        {
            let _outer = acquire_paths(&lock_dir, &pid_file).unwrap();
            assert!(pid_file.is_file());
        }
        assert!(!lock_dir.exists());
        // And it can be taken again after the release.
        let again = acquire_paths(&lock_dir, &pid_file).expect("re-acquire");
        drop(again);
        let _ = remove_dir(&dir);
    }
}
