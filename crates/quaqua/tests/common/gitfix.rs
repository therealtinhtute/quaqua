//! Hermetic git fixtures: a test repo must not inherit the host's git config,
//! and it must never be the checkout the test happens to run in.
//!
//! Why: CI has no global config and a shallow, detached checkout, while a dev
//! host may set `init.defaultBranch`, `commit.gpgsign`, `core.hooksPath`, or a
//! `GIT_DIR` from a surrounding hook. A card that said `repo: .` resolved to
//! whatever checkout `cargo test` ran in, so the stale-card tests read the
//! host's history and `lead spawn` wrote `quaqua/<slug>-integ` into it
//! (PR #154 CI run 40295850651).

#![allow(dead_code)]

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

/// Keep `cmd` — git itself, or a quaqua binary that shells out to git — off the
/// host's global and system git config, and out of an ambient repository.
pub fn isolate(cmd: &mut Command) -> &mut Command {
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    for key in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
        cmd.env_remove(key);
    }
    cmd
}

/// A `git` command that will not read the host's config.
pub fn cmd(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    isolate(&mut cmd);
    cmd
}

/// `git -C <dir> <args>`, isolated from the host config.
pub fn git(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = cmd("git");
    cmd.arg("-C").arg(dir).args(args);
    cmd.output().expect("run git")
}

/// `git` that must succeed: a fixture whose repo is broken should say so here,
/// not as a confusing assertion three steps later.
pub fn git_ok(dir: &Path, args: &[&str]) -> Output {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Create a repo on `main` with a local identity and one empty commit.
/// Returns the commit sha. `main` is explicit because CI has no
/// `init.defaultBranch` and the default used to come from the host.
pub fn init_repo(dir: &Path) -> String {
    std::fs::create_dir_all(dir).unwrap();
    git_ok(dir, &["init", "-q", "-b", "main"]);
    git_ok(dir, &["config", "user.name", "Quaqua Test"]);
    git_ok(dir, &["config", "user.email", "test@quaqua.local"]);
    git_ok(dir, &["commit", "-qm", "initial commit", "--allow-empty"]);
    head(dir)
}

/// The full sha at `HEAD`.
pub fn head(dir: &Path) -> String {
    let out = git_ok(dir, &["rev-parse", "HEAD"]);
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
