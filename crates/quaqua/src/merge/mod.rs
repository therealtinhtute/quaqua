//! Lead merge queue (`quaqua merge <slug> <branch>...`).
//! Implements piggery refinery merge queue.

use crate::config::load_team_config;
use crate::git::{Git, GitOps};
use crate::lead::spawn::find_quaqua_root;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const USAGE: &str = "\
# Usage: quaqua merge [--check <cmd>] [--repo <dir>] <slug> <branch>...
#   Merge one branch at a time into the project integration branch quaqua/<slug>-integ.
#   Runs project check after each merge. On failure or conflict, undoes via reset --hard
#   and returns rework to Peer. Never pushes and never rewrites integ beyond undo.";

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
        }
    }

    let root = find_quaqua_root()?;
    let index = root.join("projects/INDEX.md");
    if index.is_file() {
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
                        return Ok(PathBuf::from(p));
                    }
                }
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        return Ok(cwd);
    }
    Err((1, format!("cannot resolve project repo for slug '{slug}'\n")))
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

    // Checkout integration branch
    let co_out = git.run_in(&repo, &["checkout", "--quiet", &integ])
        .map_err(|e| (1, format!("failed to checkout {integ}: {e}\n")))?;
    if !co_out.status.success() {
        let err = String::from_utf8_lossy(&co_out.stderr).to_string();
        return Err((1, format!("failed to checkout {integ}: {err}\n")));
    }

    // Verify worktree is clean
    let st_out = git.run_in(&repo, &["status", "--porcelain"])
        .map_err(|e| (1, format!("failed to check status: {e}\n")))?;
    let dirty = String::from_utf8_lossy(&st_out.stdout).trim().to_string();
    if !dirty.is_empty() {
        return Err((1, format!("worktree in {} is dirty — cannot run merge queue\n", repo.display())));
    }

    for branch in &opts.branches {
        // 1. Note the integ commit
        let rev_out = git.run_in(&repo, &["rev-parse", "HEAD"])
            .map_err(|e| (1, format!("failed to rev-parse HEAD: {e}\n")))?;
        let head_before = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();

        // 2. Try merging
        let merge_out = git.run_in(&repo, &["merge", "--no-edit", branch])
            .map_err(|e| (1, format!("failed to run git merge: {e}\n")))?;

        if !merge_out.status.success() {
            // Conflict -> merge --abort
            let _ = git.run_in(&repo, &["merge", "--abort"]);
            return Err((
                1,
                format!("error: merge conflict merging '{branch}' into '{integ}' — returning rework to Peer\n"),
            ));
        }

        // 3. Run project check
        let check_pass = run_check(&repo, opts.check_cmd.as_deref());
        if check_pass {
            let new_rev = git.run_in(&repo, &["rev-parse", "HEAD"])
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            println!("merged {branch} into {integ} ({new_rev})");
        } else {
            // Undo merge: git reset --hard <head_before>
            let _ = git.run_in(&repo, &["reset", "--hard", &head_before]);

            // Check if check fails without the branch too
            let base_check_pass = run_check(&repo, opts.check_cmd.as_deref());
            if !base_check_pass {
                return Err((
                    1,
                    format!("error: check fails on integ base without '{branch}' — ask CoS and hold queue\n"),
                ));
            } else {
                return Err((
                    1,
                    format!("error: check failed with branch '{branch}' — undoing merge and returning rework to Peer\n"),
                ));
            }
        }
    }

    Ok(())
}
