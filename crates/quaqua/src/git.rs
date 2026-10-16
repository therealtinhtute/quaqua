//! Git execution helpers. Commands execute the `git` binary found in PATH.

use std::path::Path;
use std::process::{Command, Output};

#[derive(Debug, Clone, Default)]
pub struct Git;

impl Git {
    pub fn new() -> Self {
        Self
    }

    /// Run `git -C <dir> <args...>` and return standard Output.
    pub fn run_in<P: AsRef<Path>>(&self, dir: P, args: &[&str]) -> std::io::Result<Output> {
        Command::new("git")
            .arg("-C")
            .arg(dir.as_ref())
            .args(args)
            .output()
    }

    /// Run `git <args...>` without `-C` and return standard Output.
    pub fn run(&self, args: &[&str]) -> std::io::Result<Output> {
        Command::new("git").args(args).output()
    }

    /// Check if `<base>` resolves to a commit in `<repo>`.
    pub fn rev_parse_commit<P: AsRef<Path>>(&self, repo: P, base: &str) -> bool {
        let commit_ref = format!("{}^{{commit}}", base);
        match self.run_in(
            repo,
            &["rev-parse", "--verify", "--quiet", "--end-of-options", &commit_ref],
        ) {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }

    /// Get list of registered worktrees from `git -C <repo> worktree list --porcelain`.
    pub fn worktree_list_porcelain<P: AsRef<Path>>(&self, repo: P) -> std::io::Result<Vec<String>> {
        let out = self.run_in(repo, &["worktree", "list", "--porcelain"])?;
        if !out.status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                String::from_utf8_lossy(&out.stderr).to_string(),
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut list = Vec::new();
        for line in text.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                list.push(path.trim().to_string());
            }
        }
        Ok(list)
    }

    /// Add worktree: `git -C <repo> worktree add -b quaqua/<name> <wt> [<base>]`.
    pub fn worktree_add<P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        repo: P,
        branch: &str,
        wt: Q,
        base: Option<&str>,
    ) -> std::io::Result<Output> {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(repo.as_ref())
            .arg("worktree")
            .arg("add")
            .arg("-b")
            .arg(branch)
            .arg(wt.as_ref());
        if let Some(b) = base {
            cmd.arg(b);
        }
        cmd.output()
    }

    /// Remove worktree: `git -C <repo> worktree remove <wt>`.
    pub fn worktree_remove<P: AsRef<Path>, Q: AsRef<Path>>(&self, repo: P, wt: Q) -> std::io::Result<Output> {
        Command::new("git")
            .arg("-C")
            .arg(repo.as_ref())
            .arg("worktree")
            .arg("remove")
            .arg(wt.as_ref())
            .output()
    }

    /// Get remote names from `git -C <repo> remote`.
    pub fn remote_list<P: AsRef<Path>>(&self, repo: P) -> std::io::Result<Vec<String>> {
        let out = self.run_in(repo, &["remote"])?;
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect())
    }

    /// Check if common config contains core.bare=true.
    pub fn is_core_bare<P: AsRef<Path>>(&self, repo: P) -> bool {
        match self.run_in(repo, &["config", "--local", "--bool", "--get", "core.bare"]) {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
                text == "true"
            }
            Err(_) => false,
        }
    }

    /// Check if common config list contains `core.worktree=`.
    pub fn has_core_worktree<P: AsRef<Path>>(&self, repo: P) -> bool {
        match self.run_in(repo, &["config", "--local", "--list"]) {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                text.lines().any(|l| l.starts_with("core.worktree="))
            }
            Err(_) => false,
        }
    }

    /// Enable worktreeConfig: `git -C <repo> config extensions.worktreeConfig true`.
    pub fn enable_worktree_config<P: AsRef<Path>>(&self, repo: P) -> std::io::Result<Output> {
        self.run_in(repo, &["config", "extensions.worktreeConfig", "true"])
    }

    /// Set worktree-specific pushurl:
    /// `git -C <wt> config --worktree --replace-all remote.<r>.pushurl ""`
    /// `git -C <wt> config --worktree --add remote.<r>.pushurl "<msg>"`
    pub fn set_worktree_pushurl_blocked<P: AsRef<Path>>(&self, wt: P, remote: &str) -> std::io::Result<()> {
        let key = format!("remote.{}.pushurl", remote);
        let _ = self.run_in(&wt, &["config", "--worktree", "--replace-all", &key, ""])?;
        let _ = self.run_in(
            &wt,
            &[
                "config",
                "--worktree",
                "--add",
                &key,
                "PUSH DISABLED - task workers never push (AGENTS.md handoff rule)",
            ],
        )?;
        Ok(())
    }

    /// Check if directory is inside a git work tree.
    pub fn is_git_dir<P: AsRef<Path>>(&self, cwd: P) -> bool {
        match self.run_in(cwd, &["rev-parse", "--git-dir"]) {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }

    /// Compute status porcelain + diff HEAD stat hash and diffstat summary.
    pub fn diff_and_status<P: AsRef<Path>>(&self, cwd: P) -> (String, String) {
        let st_out = self.run_in(&cwd, &["status", "--porcelain=v1"]).ok();
        let diff_out = self.run_in(&cwd, &["diff", "HEAD", "--stat"]).ok();
        let short_out = self.run_in(&cwd, &["diff", "HEAD", "--shortstat"]).ok();

        let st_text = st_out
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let diff_text = diff_out
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        let combined = format!("{}{}", st_text, diff_text);
        let dh = crate::hash::md5_hex(combined.as_bytes());

        let short = short_out
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let untracked = st_text
            .lines()
            .filter(|l| l.starts_with("??"))
            .count();

        let mut diffstat = if short.is_empty() {
            "clean".to_string()
        } else {
            short
        };
        if untracked > 0 {
            diffstat.push_str(&format!(", {} untracked", untracked));
        }

        (dh, diffstat)
    }
}
