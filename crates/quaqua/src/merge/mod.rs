//! Lead merge queue (`quaqua merge <slug> <branch>...`).
//! Implements piggery refinery merge queue.

use crate::config::load_team_config;
use crate::git::{Git, GitOps};
use crate::workspace::workspace_root_checked;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const USAGE: &str = "\
# Usage: quaqua merge [--check <cmd>] [--repo <dir>] <slug> <branch>...
#   Merge one branch at a time into the project integration branch quaqua/<slug>-integ.
#   Runs project check after each merge. On failure or conflict, undoes via reset --hard
#   and returns rework to Peer. Never pushes and never rewrites integ beyond undo.
#
# Exit codes:
#   0: all branches merged cleanly
#   1: operational error (bad arguments, missing repo/branch, dirty worktree)
#   2: check failed with branch — returning rework to Peer (undo via reset --hard)
#   3: check fails on integ base without the branch — ask CoS and hold queue
#   4: merge conflict — returning rework to Peer (undo via merge --abort)";

pub struct MergeOptions {
    pub slug: String,
    pub branches: Vec<String>,
    pub check_cmd: Option<String>,
    pub repo: Option<PathBuf>,
}

pub fn parse_merge_args(args: &[String]) -> Result<MergeOptions, (i32, String)> {
    let mut check_cmd: Option<String> = None;
    let mut repo: Option<PathBuf> = None;
    let mut pos = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--check" {
            if i + 1 >= args.len() {
                return Err((1, format!("{}\n", USAGE)));
            }
            check_cmd = Some(args[i + 1].clone());
            i += 2;
        } else if let Some(v) = arg.strip_prefix("--check=") {
            check_cmd = Some(v.to_string());
            i += 1;
        } else if arg == "--repo" {
            if i + 1 >= args.len() {
                return Err((1, format!("{}\n", USAGE)));
            }
            repo = Some(PathBuf::from(&args[i + 1]));
            i += 2;
        } else if let Some(v) = arg.strip_prefix("--repo=") {
            repo = Some(PathBuf::from(v));
            i += 1;
        } else if arg.starts_with('-') {
            return Err((1, format!("{}\n", USAGE)));
        } else {
            pos.push(arg.clone());
            i += 1;
        }
    }

    if pos.len() < 2 {
        return Err((1, format!("{}\n", USAGE)));
    }

    Ok(MergeOptions {
        slug: pos[0].clone(),
        branches: pos[1..].to_vec(),
        check_cmd,
        repo,
    })
}

fn resolve_project_repo(slug: &str, explicit_repo: Option<&Path>) -> Result<PathBuf, (i32, String)> {
    if let Some(r) = explicit_repo {
        if r.is_dir() {
            return Ok(r.to_path_buf());
        } else {
            return Err((1, format!("repo path '{}' does not exist or is not a directory\n", r.display())));
        }
    }

    let root = workspace_root_checked()?;
    let index = root.join("projects/INDEX.md");
    if let Ok(content) = std::fs::read_to_string(&index) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('|') {
                let parts: Vec<&str> = trimmed.split('|').map(|s| s.trim()).collect();
                if parts.len() >= 3 && parts[1] == slug && parts[2] != "repo" {
                    let r = parts[2];
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                    let p = if let Some(stripped) = r.strip_prefix("~/") {
                        format!("{}/{}", home, stripped)
                    } else if r == "~" {
                        home
                    } else {
                        r.to_string()
                    };
                    let pb = PathBuf::from(&p);
                    if pb.is_dir() {
                        return Ok(pb);
                    } else {
                        return Err((1, format!("project repo '{}' for slug '{slug}' does not exist\n", pb.display())));
                    }
                }
            }
        }
        return Err((1, format!("unknown project slug '{slug}' — no row in {}\n", index.display())));
    }

    Err((1, format!("cannot resolve project repo for slug '{slug}'\n")))
}

fn find_worktree_for_branch(repo: &Path, branch_name: &str) -> Option<PathBuf> {
    let git = Git::new();
    let out = git.run_in(repo, &["worktree", "list", "--porcelain"]).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut cur_wt: Option<PathBuf> = None;
    let branch_ref = format!("refs/heads/{branch_name}");
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            cur_wt = Some(PathBuf::from(path.trim()));
        } else if let Some(b) = line.strip_prefix("branch ") {
            if b.trim() == branch_ref {
                return cur_wt;
            }
        }
    }
    None
}

struct MergeWorktreeGuard<'a> {
    repo: &'a Path,
    target_dir: PathBuf,
    is_temp_worktree: bool,
    orig_branch: Option<String>,
    integ: String,
}

impl<'a> Drop for MergeWorktreeGuard<'a> {
    fn drop(&mut self) {
        let git = Git::new();
        if self.is_temp_worktree {
            let _ = git.run_in(self.repo, &["worktree", "remove", "--force", &self.target_dir.to_string_lossy()]);
            let _ = std::fs::remove_dir_all(&self.target_dir);
        } else if let Some(orig) = &self.orig_branch {
            if orig != &self.integ && !orig.is_empty() && orig != "HEAD" {
                let _ = git.run_in(self.repo, &["checkout", "--quiet", orig]);
            }
        }
    }
}

fn run_check(repo: &Path, check_cmd: Option<&str>) -> bool {
    let cmd_str = if let Some(cmd) = check_cmd {
        cmd.to_string()
    } else if let Ok(env_cmd) = std::env::var("QUAQUA_CHECK_CMD") {
        env_cmd
    } else if let Ok(env_cmd2) = std::env::var("MERGE_CHECK_CMD") {
        env_cmd2
    } else {
        return true;
    };

    if cmd_str.trim().is_empty() {
        return true;
    }

    let res = Command::new("sh")
        .arg("-c")
        .arg(&cmd_str)
        .current_dir(repo)
        .status();

    res.map(|s| s.success()).unwrap_or(false)
}

pub fn handle_merge(args: &[String], git: &Git) -> Result<(), (i32, String)> {
    let opts = parse_merge_args(args)?;
    let cfg = load_team_config()?;
    let repo = resolve_project_repo(&opts.slug, opts.repo.as_deref())?;

    let integ_pattern = if cfg.merge.integ_branch.is_empty() {
        "quaqua/{slug}-integ"
    } else {
        &cfg.merge.integ_branch
    };
    let integ = integ_pattern.replace("{slug}", &opts.slug);

    // Verify integration branch exists
    let integ_ref = format!("refs/heads/{}", integ);
    if !git.run_in(&repo, &["rev-parse", "--verify", "--quiet", &integ_ref]).map(|o| o.status.success()).unwrap_or(false) {
        return Err((1, format!("integration branch '{integ}' does not exist in {}\n", repo.display())));
    }

    // Determine working worktree without mutating the primary checkout unnecessarily
    let existing_wt = find_worktree_for_branch(&repo, &integ);
    let guard = if let Some(wt) = existing_wt {
        // integ is already checked out in a worktree: use it directly
        // Verify its worktree is clean
        let st_out = git.run_in(&wt, &["status", "--porcelain"])
            .map_err(|e| (1, format!("failed to check status: {e}\n")))?;
        let dirty = String::from_utf8_lossy(&st_out.stdout).trim().to_string();
        if !dirty.is_empty() {
            return Err((1, format!("worktree in {} is dirty — cannot run merge queue\n", wt.display())));
        }
        MergeWorktreeGuard {
            repo: &repo,
            target_dir: wt,
            is_temp_worktree: false,
            orig_branch: None,
            integ: integ.clone(),
        }
    } else {
        // Try creating a temporary worktree on integ
        let tmp_dir = std::env::temp_dir().join(format!("quaqua-merge-{}-{}", opts.slug, std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let add_out = git.run_in(&repo, &["worktree", "add", &tmp_dir.to_string_lossy(), &integ]);

        match add_out {
            Ok(o) if o.status.success() => {
                MergeWorktreeGuard {
                    repo: &repo,
                    target_dir: tmp_dir,
                    is_temp_worktree: true,
                    orig_branch: None,
                    integ: integ.clone(),
                }
            }
            _ => {
                // Fallback: use primary repo worktree with checkout guard
                let st_out = git.run_in(&repo, &["status", "--porcelain"])
                    .map_err(|e| (1, format!("failed to check status: {e}\n")))?;
                let dirty = String::from_utf8_lossy(&st_out.stdout).trim().to_string();
                if !dirty.is_empty() {
                    return Err((1, format!("worktree in {} is dirty — cannot run merge queue\n", repo.display())));
                }

                let orig_branch = git.run_in(&repo, &["rev-parse", "--abbrev-ref", "HEAD"])
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();

                let co_out = git.run_in(&repo, &["checkout", "--quiet", &integ])
                    .map_err(|e| (1, format!("failed to checkout {integ}: {e}\n")))?;
                if !co_out.status.success() {
                    let err = String::from_utf8_lossy(&co_out.stderr).to_string();
                    return Err((1, format!("failed to checkout {integ}: {err}\n")));
                }

                MergeWorktreeGuard {
                    repo: &repo,
                    target_dir: repo.clone(),
                    is_temp_worktree: false,
                    orig_branch: Some(orig_branch),
                    integ: integ.clone(),
                }
            }
        }
    };

    let target = &guard.target_dir;

    if opts.check_cmd.is_none() && std::env::var("QUAQUA_CHECK_CMD").is_err() && std::env::var("MERGE_CHECK_CMD").is_err() {
        eprintln!("warning: no check command configured; merging without checks");
    }

    for branch in &opts.branches {
        // 1. Note the integ commit
        let rev_out = git.run_in(target, &["rev-parse", "HEAD"])
            .map_err(|e| (1, format!("failed to rev-parse HEAD: {e}\n")))?;
        let head_before = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();

        // 2. Try merging
        let mut merge_cmd = Command::new("git");
        merge_cmd.arg("-C").arg(target).arg("merge").arg("--no-edit").arg(branch);
        if std::env::var("GIT_AUTHOR_NAME").is_err() {
            merge_cmd.env("GIT_AUTHOR_NAME", "quaqua");
            merge_cmd.env("GIT_AUTHOR_EMAIL", "quaqua@local");
            merge_cmd.env("GIT_COMMITTER_NAME", "quaqua");
            merge_cmd.env("GIT_COMMITTER_EMAIL", "quaqua@local");
        }
        let merge_out = merge_cmd.output()
            .map_err(|e| (1, format!("failed to run git merge: {e}\n")))?;

        if !merge_out.status.success() {
            // Conflict -> merge --abort (exit code 4 for conflict rework)
            let _ = git.run_in(target, &["merge", "--abort"]);
            return Err((
                4,
                format!("error: merge conflict merging '{branch}' into '{integ}' — returning rework to Peer\n"),
            ));
        }

        // 3. Run project check
        let check_pass = run_check(target, opts.check_cmd.as_deref());
        if check_pass {
            let new_rev = git.run_in(target, &["rev-parse", "HEAD"])
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            println!("merged {branch} into {integ} ({new_rev})");
        } else {
            // Undo merge: git reset --hard <head_before>
            let reset_out = git.run_in(target, &["reset", "--hard", &head_before])
                .map_err(|e| (1, format!("failed to reset --hard: {e}\n")))?;
            if !reset_out.status.success() {
                let err = String::from_utf8_lossy(&reset_out.stderr).to_string();
                return Err((1, format!("error: reset --hard failed: {err}\n")));
            }

            // Check if check fails without the branch too
            let base_check_pass = run_check(target, opts.check_cmd.as_deref());
            if !base_check_pass {
                return Err((
                    3,
                    format!("error: check fails on integ base without '{branch}' — ask CoS and hold queue\n"),
                ));
            } else {
                return Err((
                    2,
                    format!("error: check failed with branch '{branch}' — undoing merge and returning rework to Peer\n"),
                ));
            }
        }
    }

    Ok(())
}
