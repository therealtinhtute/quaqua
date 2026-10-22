//! Read-only reviewers by detection (H4): a review worker's worktree HEAD and
//! `git status --porcelain` are recorded at spawn and compared when its report
//! is accepted. Any change voids the verdict (exit 6, like a malformed report).
//! Harness flags only reduce attempts; this check is the guarantee.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::state::atomic_write;

pub fn baseline_path(runs_dir: &Path, name: &str) -> PathBuf {
    runs_dir.join(format!(".{name}.readonly"))
}

/// HEAD + porcelain of `wt`, excluding `runs_dir` when it lives inside `wt`
/// (the reviewer's own report must never count as a change).
fn snapshot(wt: &Path, runs_dir: &Path) -> Result<String, String> {
    let git = |args: &[&str]| -> Result<String, String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(wt)
            .args(args)
            .output()
            .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
        if !out.status.success() {
            return Err(format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let head = git(&["rev-parse", "HEAD"])?;
    let mut status_args = vec!["status".to_string(), "--porcelain".to_string()];
    let (wt_c, runs_c) = (wt.canonicalize(), runs_dir.canonicalize());
    if let (Ok(w), Ok(r)) = (wt_c, runs_c) {
        if let Ok(rel) = r.strip_prefix(&w) {
            if !rel.as_os_str().is_empty() {
                status_args.push("--".to_string());
                status_args.push(".".to_string());
                status_args.push(format!(":(exclude){}", rel.display()));
            }
        }
    }
    let refs: Vec<&str> = status_args.iter().map(String::as_str).collect();
    let porcelain = git(&refs)?;
    Ok(format!("HEAD {}\n{porcelain}", head.trim()))
}

/// Record the baseline for review worker `name` in worktree `wt`. When the
/// worktree cannot be read, an UNRECORDED baseline is written so acceptance
/// fails closed, and the error is returned for the caller to warn.
pub fn record(runs_dir: &Path, name: &str, wt: &Path) -> Result<(), String> {
    let (body, res) = match snapshot(wt, runs_dir) {
        Ok(snap) => (snap, Ok(())),
        Err(e) => (format!("{UNRECORDED} {e}"), Err(e)),
    };
    atomic_write(
        &baseline_path(runs_dir, name),
        format!("{}\n{body}", wt.display()),
    )
    .map_err(|e| e.to_string())?;
    res
}

const UNRECORDED: &str = "UNRECORDED";

/// Returns the baseline worktree path and HEAD SHA recorded at spawn, if valid.
pub fn baseline_info(runs_dir: &Path, name: &str) -> Option<(PathBuf, String)> {
    let saved = std::fs::read_to_string(baseline_path(runs_dir, name)).ok()?;
    let (wt, before) = saved.split_once('\n')?;
    if before.starts_with(UNRECORDED) {
        return None;
    }
    let head_line = before.lines().next()?;
    let sha = head_line.strip_prefix("HEAD ")?.trim().to_string();
    Some((PathBuf::from(wt), sha))
}

/// Returns the baseline HEAD SHA recorded at spawn, if valid.
pub fn baseline_sha(runs_dir: &Path, name: &str) -> Option<String> {
    baseline_info(runs_dir, name).map(|(_, sha)| sha)
}

/// Compare the worktree with its recorded baseline. No baseline → `Ok` (not a
/// review worker, or spawned before H4). A worktree that cannot be read is a
/// failure: fail closed.
pub fn check(runs_dir: &Path, name: &str) -> Result<(), String> {
    let Ok(saved) = std::fs::read_to_string(baseline_path(runs_dir, name)) else {
        return Ok(());
    };
    let Some((wt, before)) = saved.split_once('\n') else {
        return Err("corrupt read-only baseline".to_string());
    };
    if before.starts_with(UNRECORDED) {
        return Err(format!("no baseline recorded at spawn ({before})")
            .trim_end()
            .to_string());
    }
    let now = snapshot(Path::new(wt), runs_dir)?;
    if now == before {
        return Ok(());
    }
    let head = |s: &str| s.lines().next().unwrap_or("").to_string();
    if head(&now) != head(before) {
        Err(format!(
            "worktree HEAD moved ({} -> {})",
            head(before),
            head(&now)
        ))
    } else {
        Err("worktree changed (git status --porcelain differs)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("quaqua-ro-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn repo(tag: &str) -> PathBuf {
        let d = tmp(tag);
        git(&d, &["init", "-q"]);
        git(&d, &["config", "user.email", "t@t"]);
        git(&d, &["config", "user.name", "t"]);
        std::fs::write(d.join("a.txt"), "a\n").unwrap();
        git(&d, &["add", "."]);
        git(&d, &["commit", "-qm", "init"]);
        d
    }

    #[test]
    fn readonly_review_clean_is_accepted_and_report_in_runs_ignored() {
        let wt = repo("clean");
        let runs = wt.join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        record(&runs, "rev", &wt).unwrap();
        std::fs::write(runs.join("rev.md"), "Verdict: approve\n").unwrap();
        assert_eq!(check(&runs, "rev"), Ok(()));
    }

    #[test]
    fn readonly_review_dirty_worktree_is_rejected() {
        let wt = repo("dirty");
        let runs = tmp("dirty-runs");
        record(&runs, "rev", &wt).unwrap();
        std::fs::write(wt.join("a.txt"), "edited\n").unwrap();
        assert!(check(&runs, "rev").unwrap_err().contains("porcelain"));
    }

    #[test]
    fn readonly_review_moved_head_is_rejected() {
        let wt = repo("head");
        let runs = tmp("head-runs");
        record(&runs, "rev", &wt).unwrap();
        git(&wt, &["commit", "-q", "--allow-empty", "-m", "sneaky"]);
        assert!(check(&runs, "rev").unwrap_err().contains("HEAD moved"));
    }

    #[test]
    fn readonly_unreadable_worktree_at_spawn_fails_closed() {
        let runs = tmp("unrec");
        assert!(record(&runs, "rev", &runs.join("missing")).is_err());
        assert!(check(&runs, "rev").unwrap_err().contains("no baseline"));
    }

    #[test]
    fn readonly_no_baseline_is_ok() {
        let runs = tmp("none");
        assert_eq!(check(&runs, "builder"), Ok(()));
    }
}
