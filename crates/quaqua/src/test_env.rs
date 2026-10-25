//! Process-global env, shared by every test in this binary.
//!
//! A test that mutates an env var another test reads must hold [`lock_env`] for
//! as long as that value has to stay put, and must restore the previous value
//! via [`TestEnvVarGuard`].
//!
//! Why: `worker::prompt::tests::test_worker_prompt_keeps_exit_code_for_unparseable_reviewer_verdict`
//! resolves its runs dir from `QUAQUA_RUNS_DIR` inside `prompt_worker`
//! (`default_runs_dir`). Another test that set that var to its own temp dir, or
//! restored it to unset, while the prompt test was starting sent the watcher to
//! `~/quaqua/runs`, so the run ended `Err((1, "runner error: no snapshot found
//! ..."))` instead of the expected `Ok(())`.

use std::ffi::OsStr;
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Serialises tests that read or write process-global env vars. A panicking
/// test must not wedge the rest, so poisoning is ignored.
pub fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Restores an environment variable when dropped, including on panic.
pub struct TestEnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl TestEnvVarGuard {
    pub fn set(key: &'static str, val: impl AsRef<OsStr>) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, val);
        Self { key, prev }
    }

    pub fn remove(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, prev }
    }
}

impl Drop for TestEnvVarGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(val) => std::env::set_var(self.key, val),
            None => std::env::remove_var(self.key),
        }
    }
}
