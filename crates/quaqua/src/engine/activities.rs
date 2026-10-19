//! Real activity implementations for the Quaqua workflow engine (E3).
//!
//! Provides [`SystemActivities`], executing typed side-effecting operations
//! over existing [`Herdr`] and [`Git`] adapters, filesystem reports, and cost checks.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::engine::activity::{
    Activity, ActivityError, ActivityRequest, ActivityResult, CostRequest, CostResult, GitRequest,
    GitResult, GridRequest, HerdrEventRequest, HerdrRequest, HerdrResult, ReportFileRequest,
    ReportFileResult,
};
use crate::git::{Git, GitOps};
use crate::herdr::{Herdr, HerdrOps};

/// Production activities implementation executing real operations against
/// herdr, git, and local filesystem resources.
#[derive(Debug, Clone)]
pub struct SystemActivities {
    pub herdr: Herdr,
    pub git: Git,
    pub repo_dir: PathBuf,
}

impl SystemActivities {
    /// Creates a new `SystemActivities` using default Herdr and Git adapters
    /// rooted at the process current working directory.
    pub fn new() -> Self {
        Self::with_repo_dir(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// Creates a new `SystemActivities` rooted at the specified repository path.
    pub fn with_repo_dir(repo_dir: impl Into<PathBuf>) -> Self {
        Self {
            herdr: Herdr::new(),
            git: Git::new(),
            repo_dir: repo_dir.into(),
        }
    }

    /// Creates a new `SystemActivities` with explicit adapter instances and working directory.
    pub fn with_adapters(herdr: Herdr, git: Git, repo_dir: impl Into<PathBuf>) -> Self {
        Self {
            herdr,
            git,
            repo_dir: repo_dir.into(),
        }
    }

    fn execute_herdr(&self, req: &HerdrRequest) -> Result<ActivityResult, ActivityError> {
        match req {
            HerdrRequest::Prompt { name, task } => {
                let out = self.herdr.agent_prompt_nowait(name, task).map_err(|e| {
                    ActivityError::ExecutionFailed(format!("herdr agent prompt io error: {e}"))
                })?;
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let err = if stderr.trim().is_empty() {
                        stdout
                    } else {
                        stderr
                    };
                    return Err(ActivityError::ExecutionFailed(format!(
                        "herdr agent prompt failed: {err}"
                    )));
                }
                Ok(ActivityResult::Herdr(HerdrResult::Prompted {
                    name: name.clone(),
                }))
            }
            HerdrRequest::SendKeys { name, keys } => {
                let out = self.herdr.agent_send_keys(name, keys).map_err(|e| {
                    ActivityError::ExecutionFailed(format!("herdr agent send-keys io error: {e}"))
                })?;
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let err = if stderr.trim().is_empty() {
                        stdout
                    } else {
                        stderr
                    };
                    return Err(ActivityError::ExecutionFailed(format!(
                        "herdr agent send-keys failed: {err}"
                    )));
                }
                Ok(ActivityResult::Herdr(HerdrResult::KeysSent {
                    name: name.clone(),
                }))
            }
            HerdrRequest::ReadAgent { name, lines } => {
                let lines_str = lines.to_string();
                let out = self
                    .herdr
                    .run(&[
                        "agent", "read", name, "--source", "visible", "--lines", &lines_str,
                    ])
                    .map_err(|e| {
                        ActivityError::ExecutionFailed(format!("herdr agent read io error: {e}"))
                    })?;
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let err = if stderr.trim().is_empty() {
                        stdout
                    } else {
                        stderr
                    };
                    return Err(ActivityError::ExecutionFailed(format!(
                        "herdr agent read failed: {err}"
                    )));
                }
                let content = String::from_utf8_lossy(&out.stdout).to_string();
                Ok(ActivityResult::Herdr(HerdrResult::AgentOutput { content }))
            }
            HerdrRequest::Stop { name } => {
                let (ok, output) = self.herdr.agent_get(name).map_err(|e| {
                    ActivityError::ExecutionFailed(format!("herdr agent get io error: {e}"))
                })?;

                if !ok || output.contains("agent_not_found") {
                    return Err(ActivityError::NotFound(format!(
                        "agent {name} not found: {output}"
                    )));
                }

                // If output contains a pane_id, close the pane.
                let pane_id = serde_json::from_str::<serde_json::Value>(&output)
                    .ok()
                    .and_then(|v| {
                        v.get("result")
                            .and_then(|r| r.get("agent"))
                            .and_then(|a| a.get("pane_id"))
                            .and_then(|p| p.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                    });

                let pane = pane_id.ok_or_else(|| {
                    ActivityError::ExecutionFailed(format!(
                        "cannot stop agent {name}: no valid pane_id found in herdr output: {output}"
                    ))
                })?;

                let close_out = self.herdr.pane_close(&pane).map_err(|e| {
                    ActivityError::ExecutionFailed(format!("herdr pane close io error: {e}"))
                })?;
                if !close_out.status.success() {
                    let err = String::from_utf8_lossy(&close_out.stderr);
                    return Err(ActivityError::ExecutionFailed(format!(
                        "herdr pane close failed: {err}"
                    )));
                }

                Ok(ActivityResult::Herdr(HerdrResult::Stopped {
                    name: name.clone(),
                }))
            }
            HerdrRequest::Spawn { .. } => Err(ActivityError::ExecutionFailed(
                "HerdrRequest::Spawn deferred to W2 pipeline orchestration (P2/K1)".to_string(),
            )),
            HerdrRequest::Cleanup { .. } => Err(ActivityError::ExecutionFailed(
                "HerdrRequest::Cleanup deferred to W2 pipeline orchestration (P1/K1)".to_string(),
            )),
        }
    }

    fn execute_git(&self, req: &GitRequest) -> Result<ActivityResult, ActivityError> {
        match req {
            GitRequest::WorktreeAdd { path, branch, base } => {
                let out = self
                    .git
                    .worktree_add(&self.repo_dir, branch, Path::new(path), base.as_deref())
                    .map_err(|e| {
                        ActivityError::ExecutionFailed(format!("git worktree add io error: {e}"))
                    })?;
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    return Err(ActivityError::ExecutionFailed(format!(
                        "git worktree add failed: {stderr}"
                    )));
                }
                Ok(ActivityResult::Git(GitResult::WorktreeAdded {
                    path: path.clone(),
                }))
            }
            GitRequest::WorktreeRemove { path } => {
                let out = self
                    .git
                    .worktree_remove(&self.repo_dir, Path::new(path))
                    .map_err(|e| {
                        ActivityError::ExecutionFailed(format!("git worktree remove io error: {e}"))
                    })?;
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    return Err(ActivityError::ExecutionFailed(format!(
                        "git worktree remove failed: {stderr}"
                    )));
                }
                Ok(ActivityResult::Git(GitResult::WorktreeRemoved {
                    path: path.clone(),
                }))
            }
            GitRequest::BranchCreate { name, base } => {
                let out = self
                    .git
                    .run_in(&self.repo_dir, &["branch", name, base])
                    .map_err(|e| {
                        ActivityError::ExecutionFailed(format!("git branch io error: {e}"))
                    })?;
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    return Err(ActivityError::ExecutionFailed(format!(
                        "git branch create failed: {stderr}"
                    )));
                }
                Ok(ActivityResult::Git(GitResult::BranchCreated {
                    name: name.clone(),
                }))
            }
            GitRequest::Commit { message } => {
                let out = self
                    .git
                    .run_in(&self.repo_dir, &["commit", "-m", message])
                    .map_err(|e| {
                        ActivityError::ExecutionFailed(format!("git commit io error: {e}"))
                    })?;
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    return Err(ActivityError::ExecutionFailed(format!(
                        "git commit failed: {stderr}"
                    )));
                }
                let sha_out = self
                    .git
                    .run_in(&self.repo_dir, &["rev-parse", "HEAD"])
                    .map_err(|e| {
                        ActivityError::ExecutionFailed(format!("git rev-parse io error: {e}"))
                    })?;
                if !sha_out.status.success() {
                    let stderr = String::from_utf8_lossy(&sha_out.stderr);
                    return Err(ActivityError::ExecutionFailed(format!(
                        "git rev-parse HEAD failed: {stderr}"
                    )));
                }
                let sha = String::from_utf8_lossy(&sha_out.stdout).trim().to_string();
                Ok(ActivityResult::Git(GitResult::Committed { sha }))
            }
            GitRequest::Merge { branch } => {
                let out = self
                    .git
                    .run_in(&self.repo_dir, &["merge", "--no-ff", branch])
                    .map_err(|e| {
                        ActivityError::ExecutionFailed(format!("git merge io error: {e}"))
                    })?;
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    return Err(ActivityError::ExecutionFailed(format!(
                        "git merge failed: {stderr}"
                    )));
                }
                let sha_out = self
                    .git
                    .run_in(&self.repo_dir, &["rev-parse", "HEAD"])
                    .map_err(|e| {
                        ActivityError::ExecutionFailed(format!("git rev-parse io error: {e}"))
                    })?;
                if !sha_out.status.success() {
                    let stderr = String::from_utf8_lossy(&sha_out.stderr);
                    return Err(ActivityError::ExecutionFailed(format!(
                        "git rev-parse HEAD failed: {stderr}"
                    )));
                }
                let sha = String::from_utf8_lossy(&sha_out.stdout).trim().to_string();
                Ok(ActivityResult::Git(GitResult::Merged { sha }))
            }
            GitRequest::CurrentSha => {
                let out = self
                    .git
                    .run_in(&self.repo_dir, &["rev-parse", "HEAD"])
                    .map_err(|e| {
                        ActivityError::ExecutionFailed(format!("git rev-parse io error: {e}"))
                    })?;
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    return Err(ActivityError::ExecutionFailed(format!(
                        "git rev-parse HEAD failed: {stderr}"
                    )));
                }
                let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
                Ok(ActivityResult::Git(GitResult::Sha(sha)))
            }
        }
    }

    fn execute_report_file(
        &self,
        req: &ReportFileRequest,
    ) -> Result<ActivityResult, ActivityError> {
        match req {
            ReportFileRequest::Read { path } => match std::fs::read_to_string(path) {
                Ok(content) => Ok(ActivityResult::ReportFile(ReportFileResult::Content(
                    content,
                ))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ActivityError::NotFound(
                    format!("report file not found: {path}"),
                )),
                Err(e) => Err(ActivityError::ExecutionFailed(format!(
                    "cannot read report file {path}: {e}"
                ))),
            },
            ReportFileRequest::Write { path, content } => {
                let p = Path::new(path);
                if let Some(parent) = p.parent() {
                    if !parent.as_os_str().is_empty() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                }
                std::fs::write(p, content).map_err(|e| {
                    ActivityError::ExecutionFailed(format!("cannot write report file {path}: {e}"))
                })?;
                Ok(ActivityResult::ReportFile(ReportFileResult::Written))
            }
            ReportFileRequest::CheckMtime { path } => match std::fs::metadata(path) {
                Ok(m) => {
                    let mtime = m.modified().map_err(|e| {
                        ActivityError::ExecutionFailed(format!("cannot get mtime for {path}: {e}"))
                    })?;
                    let ms = mtime
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    Ok(ActivityResult::ReportFile(ReportFileResult::MtimeMs(ms)))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ActivityError::NotFound(
                    format!("report file not found: {path}"),
                )),
                Err(e) => Err(ActivityError::ExecutionFailed(format!(
                    "cannot inspect report file {path}: {e}"
                ))),
            },
            ReportFileRequest::Exists { path } => {
                let exists = Path::new(path).exists();
                Ok(ActivityResult::ReportFile(ReportFileResult::Exists(exists)))
            }
        }
    }

    fn execute_cost(&self, req: &CostRequest) -> Result<ActivityResult, ActivityError> {
        match req {
            CostRequest::CheckBudget { spend, budget } => Ok(ActivityResult::Cost(
                CostResult::BudgetExceeded(*spend > *budget),
            )),
            CostRequest::QuerySpend { .. } => Err(ActivityError::ExecutionFailed(
                "CostRequest::QuerySpend not wired until W2 (owned by K3)".to_string(),
            )),
        }
    }
}

impl Default for SystemActivities {
    fn default() -> Self {
        Self::new()
    }
}

impl Activity for SystemActivities {
    fn execute(&self, req: &ActivityRequest) -> Result<ActivityResult, ActivityError> {
        match req {
            ActivityRequest::Herdr(r) => self.execute_herdr(r),
            ActivityRequest::HerdrEvents(_) => Err(ActivityError::ExecutionFailed(
                "HerdrEvents activity not wired until W2 (owned by K2)".to_string(),
            )),
            ActivityRequest::Git(r) => self.execute_git(r),
            ActivityRequest::Grid(_) => Err(ActivityError::ExecutionFailed(
                "GridRequest not wired until W2 (owned by P2)".to_string(),
            )),
            ActivityRequest::ReportFile(r) => self.execute_report_file(r),
            ActivityRequest::Cost(r) => self.execute_cost(r),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    static HERDR_TEST_MUTEX: Mutex<()> = Mutex::new(());
    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct EnvRestorer {
        prev: Option<String>,
        temp_dir: PathBuf,
    }

    impl Drop for EnvRestorer {
        fn drop(&mut self) {
            match &self.prev {
                Some(val) => std::env::set_var("HERDR_BIN_PATH", val),
                None => std::env::remove_var("HERDR_BIN_PATH"),
            }
            let _ = fs::remove_dir_all(&self.temp_dir);
        }
    }

    fn with_herdr_stub<F: FnOnce(&SystemActivities, &Path)>(f: F) {
        let pid = std::process::id();
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir().join(format!("quaqua-herdr-stub-{}-{}", pid, count));
        let _ = fs::create_dir_all(&temp_dir);
        let stub_bin = temp_dir.join("herdr_stub.sh");
        let log_file = temp_dir.join("herdr_stub.log");

        let script = format!(
            r#"#!/bin/sh
LOG="{}"
case "$1" in
  agent)
    case "$2" in
      prompt)
        if [ "$3" = "e3-stub-agent" ]; then
          echo "prompted:$4" >> "$LOG"
          exit 0
        else
          echo "unknown agent $3" >&2
          exit 1
        fi
        ;;
      send-keys)
        if [ "$3" = "e3-stub-agent" ]; then
          echo "send-keys:$4" >> "$LOG"
          exit 0
        else
          echo "unknown agent $3" >&2
          exit 1
        fi
        ;;
      read)
        if [ "$3" = "e3-stub-agent" ]; then
          echo "stub output line 1"
          echo "stub output line 2"
          exit 0
        else
          echo "agent_not_found" >&2
          exit 1
        fi
        ;;
      get)
        if [ "$3" = "e3-stub-agent" ]; then
          echo '{{"result": {{"agent": {{"name": "e3-stub-agent", "pane_id": "e3-pane-42"}}}}}}'
          exit 0
        elif [ "$3" = "e3-nopane-agent" ]; then
          echo '{{"result": {{"agent": {{"name": "e3-nopane-agent"}}}}}}'
          exit 0
        else
          echo '{{"error": {{"code": "agent_not_found"}}}}' >&2
          exit 1
        fi
        ;;
      *)
        echo "unknown agent command $2" >&2
        exit 1
        ;;
    esac
    ;;
  pane)
    case "$2" in
      close)
        if [ "$3" = "e3-pane-42" ]; then
          echo "closed-pane:$3" >> "$LOG"
          exit 0
        else
          echo "unknown pane $3" >&2
          exit 1
        fi
        ;;
      *)
        echo "unknown pane command $2" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "unknown command $1" >&2
    exit 1
    ;;
esac
"#,
            log_file.to_string_lossy()
        );

        fs::write(&stub_bin, script).expect("write stub script");
        let mut perms = fs::metadata(&stub_bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stub_bin, perms).expect("chmod +x stub script");

        let stub_herdr = Herdr {
            bin: stub_bin.to_string_lossy().to_string(),
        };
        let activities = SystemActivities::with_adapters(stub_herdr, Git::new(), &temp_dir);

        f(&activities, &log_file);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_herdr_prompt_via_stub() {
        with_herdr_stub(|activities, log_file| {
            let req = ActivityRequest::Herdr(HerdrRequest::Prompt {
                name: "e3-stub-agent".to_string(),
                task: "execute plan".to_string(),
            });
            let res = activities.execute(&req).expect("prompt must succeed");
            assert_eq!(
                res,
                ActivityResult::Herdr(HerdrResult::Prompted {
                    name: "e3-stub-agent".to_string(),
                })
            );

            let log_content = fs::read_to_string(log_file).unwrap_or_default();
            assert!(log_content.contains("prompted:execute plan"));
        });
    }

    #[test]
    fn test_herdr_send_keys_via_stub() {
        with_herdr_stub(|activities, log_file| {
            let req = ActivityRequest::Herdr(HerdrRequest::SendKeys {
                name: "e3-stub-agent".to_string(),
                keys: "C-c".to_string(),
            });
            let res = activities.execute(&req).expect("send-keys must succeed");
            assert_eq!(
                res,
                ActivityResult::Herdr(HerdrResult::KeysSent {
                    name: "e3-stub-agent".to_string(),
                })
            );

            let log_content = fs::read_to_string(log_file).unwrap_or_default();
            assert!(log_content.contains("send-keys:C-c"));
        });
    }

    #[test]
    fn test_herdr_read_agent_via_stub() {
        with_herdr_stub(|activities, _log_file| {
            let req = ActivityRequest::Herdr(HerdrRequest::ReadAgent {
                name: "e3-stub-agent".to_string(),
                lines: 50,
            });
            let res = activities.execute(&req).expect("read agent must succeed");
            match res {
                ActivityResult::Herdr(HerdrResult::AgentOutput { content }) => {
                    assert!(content.contains("stub output line 1"));
                }
                other => panic!("unexpected result: {other:?}"),
            }
        });
    }

    #[test]
    fn test_herdr_stop_agent_via_stub() {
        with_herdr_stub(|activities, log_file| {
            let req = ActivityRequest::Herdr(HerdrRequest::Stop {
                name: "e3-stub-agent".to_string(),
            });
            let res = activities.execute(&req).expect("stop agent must succeed");
            assert_eq!(
                res,
                ActivityResult::Herdr(HerdrResult::Stopped {
                    name: "e3-stub-agent".to_string(),
                })
            );

            let log_content = fs::read_to_string(log_file).unwrap_or_default();
            assert!(log_content.contains("closed-pane:e3-pane-42"));
        });
    }

    #[test]
    fn test_herdr_stop_missing_pane_id_fails() {
        with_herdr_stub(|activities, _log_file| {
            let req = ActivityRequest::Herdr(HerdrRequest::Stop {
                name: "e3-nopane-agent".to_string(),
            });
            let err = activities
                .execute(&req)
                .expect_err("stopping agent without pane_id must fail");
            match err {
                ActivityError::ExecutionFailed(msg) => {
                    assert!(msg.contains("no valid pane_id"));
                }
                other => panic!("expected ExecutionFailed, got: {other:?}"),
            }
        });
    }

    #[test]
    fn test_herdr_unknown_agent_fails() {
        with_herdr_stub(|activities, _log_file| {
            let req = ActivityRequest::Herdr(HerdrRequest::Prompt {
                name: "live-agent-nonexistent".to_string(),
                task: "should fail".to_string(),
            });
            let err = activities
                .execute(&req)
                .expect_err("unknown agent must fail");
            assert!(matches!(err, ActivityError::ExecutionFailed(_)));
        });
    }

    #[test]
    fn test_herdr_stop_nonexistent_returns_not_found() {
        with_herdr_stub(|activities, _log_file| {
            let req = ActivityRequest::Herdr(HerdrRequest::Stop {
                name: "live-agent-nonexistent".to_string(),
            });
            let err = activities
                .execute(&req)
                .expect_err("stopping missing agent must error");
            assert!(matches!(err, ActivityError::NotFound(_)));
        });
    }

    #[test]
    fn test_herdr_bin_path_env_var_resolution() {
        let _lock = HERDR_TEST_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let pid = std::process::id();
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir().join(format!("quaqua-herdr-env-{}-{}", pid, count));
        let _ = fs::create_dir_all(&temp_dir);
        let stub_bin = temp_dir.join("herdr_stub.sh");
        let log_file = temp_dir.join("herdr_stub.log");

        let script = format!(
            r#"#!/bin/sh
LOG="{}"
if [ "$1" = "agent" ] && [ "$2" = "prompt" ] && [ "$3" = "e3-stub-agent" ]; then
  echo "env-prompted:$4" >> "$LOG"
  exit 0
fi
exit 1
"#,
            log_file.to_string_lossy()
        );
        fs::write(&stub_bin, script).expect("write stub script");
        let mut perms = fs::metadata(&stub_bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stub_bin, perms).expect("chmod +x stub script");

        let prev = std::env::var("HERDR_BIN_PATH").ok();
        std::env::set_var("HERDR_BIN_PATH", &stub_bin);
        let restorer = EnvRestorer {
            prev,
            temp_dir: temp_dir.clone(),
        };

        let activities = SystemActivities::new();
        let req = ActivityRequest::Herdr(HerdrRequest::Prompt {
            name: "e3-stub-agent".to_string(),
            task: "verify env path".to_string(),
        });
        let res = activities
            .execute(&req)
            .expect("prompt via HERDR_BIN_PATH must succeed");
        assert_eq!(
            res,
            ActivityResult::Herdr(HerdrResult::Prompted {
                name: "e3-stub-agent".to_string(),
            })
        );
        let log_content = fs::read_to_string(&log_file).unwrap_or_default();
        assert!(log_content.contains("env-prompted:verify env path"));
        drop(restorer);
    }

    #[test]
    fn test_report_file_operations() {
        let pid = std::process::id();
        let temp_dir = std::env::temp_dir().join(format!("quaqua-act-report-{}", pid));
        let _ = fs::create_dir_all(&temp_dir);
        let test_file = temp_dir.join("sub").join("report.md");
        let path_str = test_file.to_string_lossy().to_string();
        let activities = SystemActivities::new();

        // 1. Exists returns false
        let res = activities
            .execute(&ActivityRequest::ReportFile(ReportFileRequest::Exists {
                path: path_str.clone(),
            }))
            .expect("exists query");
        assert_eq!(
            res,
            ActivityResult::ReportFile(ReportFileResult::Exists(false))
        );

        // 2. Read returns NotFound
        let err = activities
            .execute(&ActivityRequest::ReportFile(ReportFileRequest::Read {
                path: path_str.clone(),
            }))
            .expect_err("read missing must fail");
        assert!(matches!(err, ActivityError::NotFound(_)));

        // 3. Write creates file and parent dirs
        let res = activities
            .execute(&ActivityRequest::ReportFile(ReportFileRequest::Write {
                path: path_str.clone(),
                content: "# Report Header\nAll green".to_string(),
            }))
            .expect("write report");
        assert_eq!(res, ActivityResult::ReportFile(ReportFileResult::Written));

        // 4. Exists returns true
        let res = activities
            .execute(&ActivityRequest::ReportFile(ReportFileRequest::Exists {
                path: path_str.clone(),
            }))
            .expect("exists query");
        assert_eq!(
            res,
            ActivityResult::ReportFile(ReportFileResult::Exists(true))
        );

        // 5. Read returns content
        let res = activities
            .execute(&ActivityRequest::ReportFile(ReportFileRequest::Read {
                path: path_str.clone(),
            }))
            .expect("read report");
        assert_eq!(
            res,
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Report Header\nAll green".to_string()
            ))
        );

        // 6. CheckMtime returns timestamp > 0
        let res = activities
            .execute(&ActivityRequest::ReportFile(
                ReportFileRequest::CheckMtime {
                    path: path_str.clone(),
                },
            ))
            .expect("check mtime");
        match res {
            ActivityResult::ReportFile(ReportFileResult::MtimeMs(ms)) => {
                assert!(ms > 0);
            }
            other => panic!("unexpected result: {other:?}"),
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_git_operations() {
        let pid = std::process::id();
        let temp_dir = std::env::temp_dir().join(format!("quaqua-act-git-{}", pid));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("create git test dir");

        // Initialize a clean test repository
        let run_cmd = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&temp_dir)
                .args(args)
                .output()
                .expect("run git command");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
            out
        };

        run_cmd(&["init"]);
        run_cmd(&["config", "user.email", "test@quaqua.local"]);
        run_cmd(&["config", "user.name", "Quaqua Test"]);

        let dummy_file = temp_dir.join("init.txt");
        fs::write(&dummy_file, "init commit").unwrap();
        run_cmd(&["add", "init.txt"]);
        run_cmd(&["commit", "-m", "init"]);

        let activities = SystemActivities::with_repo_dir(&temp_dir);

        // 1. CurrentSha
        let res = activities
            .execute(&ActivityRequest::Git(GitRequest::CurrentSha))
            .expect("current sha");
        let initial_sha = match res {
            ActivityResult::Git(GitResult::Sha(s)) => {
                assert_eq!(s.len(), 40);
                s
            }
            other => panic!("unexpected: {other:?}"),
        };

        // 2. BranchCreate
        let res = activities
            .execute(&ActivityRequest::Git(GitRequest::BranchCreate {
                name: "feature-branch".to_string(),
                base: "HEAD".to_string(),
            }))
            .expect("branch create");
        assert_eq!(
            res,
            ActivityResult::Git(GitResult::BranchCreated {
                name: "feature-branch".to_string(),
            })
        );

        // 3. Commit
        let second_file = temp_dir.join("second.txt");
        fs::write(&second_file, "second commit").unwrap();
        run_cmd(&["add", "second.txt"]);

        let res = activities
            .execute(&ActivityRequest::Git(GitRequest::Commit {
                message: "second".to_string(),
            }))
            .expect("commit");
        let second_sha = match res {
            ActivityResult::Git(GitResult::Committed { sha }) => {
                assert_eq!(sha.len(), 40);
                assert_ne!(sha, initial_sha);
                sha
            }
            other => panic!("unexpected: {other:?}"),
        };

        // Verify CurrentSha now equals second_sha
        let res = activities
            .execute(&ActivityRequest::Git(GitRequest::CurrentSha))
            .expect("current sha");
        assert_eq!(res, ActivityResult::Git(GitResult::Sha(second_sha)));

        // 4. WorktreeAdd & WorktreeRemove
        let wt_path = std::env::temp_dir().join(format!("quaqua-act-wt-{}", pid));
        let _ = fs::remove_dir_all(&wt_path);

        let res = activities
            .execute(&ActivityRequest::Git(GitRequest::WorktreeAdd {
                path: wt_path.to_string_lossy().to_string(),
                branch: "wt-branch".to_string(),
                base: None,
            }))
            .expect("worktree add");
        assert_eq!(
            res,
            ActivityResult::Git(GitResult::WorktreeAdded {
                path: wt_path.to_string_lossy().to_string(),
            })
        );
        assert!(wt_path.exists());

        let res = activities
            .execute(&ActivityRequest::Git(GitRequest::WorktreeRemove {
                path: wt_path.to_string_lossy().to_string(),
            }))
            .expect("worktree remove");
        assert_eq!(
            res,
            ActivityResult::Git(GitResult::WorktreeRemoved {
                path: wt_path.to_string_lossy().to_string(),
            })
        );
        // 5. Merge
        run_cmd(&["checkout", "-b", "feature-merge"]);
        let merge_file = temp_dir.join("merge-item.txt");
        fs::write(&merge_file, "content to merge").unwrap();
        run_cmd(&["add", "merge-item.txt"]);
        run_cmd(&["commit", "-m", "commit on feature-merge"]);

        run_cmd(&["checkout", "feature-branch"]);

        let res = activities
            .execute(&ActivityRequest::Git(GitRequest::Merge {
                branch: "feature-merge".to_string(),
            }))
            .expect("git merge must succeed");
        let merge_sha = match res {
            ActivityResult::Git(GitResult::Merged { sha }) => {
                assert_eq!(sha.len(), 40);
                sha
            }
            other => panic!("unexpected: {other:?}"),
        };
        assert!(merge_file.exists());
        assert_eq!(
            activities
                .execute(&ActivityRequest::Git(GitRequest::CurrentSha))
                .unwrap(),
            ActivityResult::Git(GitResult::Sha(merge_sha))
        );

        // Merge failure (nonexistent branch)
        let err = activities
            .execute(&ActivityRequest::Git(GitRequest::Merge {
                branch: "nonexistent-branch-404".to_string(),
            }))
            .expect_err("merging invalid branch must fail");
        assert!(matches!(err, ActivityError::ExecutionFailed(_)));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_cost_check_budget() {
        let activities = SystemActivities::new();

        let req1 = ActivityRequest::Cost(CostRequest::CheckBudget {
            spend: 5.5,
            budget: 5.0,
        });
        let res1 = activities.execute(&req1).expect("check budget over");
        assert_eq!(res1, ActivityResult::Cost(CostResult::BudgetExceeded(true)));

        let req2 = ActivityRequest::Cost(CostRequest::CheckBudget {
            spend: 3.2,
            budget: 5.0,
        });
        let res2 = activities.execute(&req2).expect("check budget under");
        assert_eq!(
            res2,
            ActivityResult::Cost(CostResult::BudgetExceeded(false))
        );
    }

    #[test]
    fn test_deferred_variants_fail_explicitly() {
        let activities = SystemActivities::new();

        let reqs = [
            ActivityRequest::Herdr(HerdrRequest::Spawn {
                name: "a".to_string(),
                kind: "b".to_string(),
                repo: "c".to_string(),
            }),
            ActivityRequest::Herdr(HerdrRequest::Cleanup {
                name: "a".to_string(),
                repo: "c".to_string(),
            }),
            ActivityRequest::HerdrEvents(HerdrEventRequest::Subscribe {
                agent: "a".to_string(),
                event_types: vec!["b".to_string()],
            }),
            ActivityRequest::HerdrEvents(HerdrEventRequest::Unsubscribe {
                agent: "a".to_string(),
            }),
            ActivityRequest::HerdrEvents(HerdrEventRequest::PollEvents),
            ActivityRequest::Grid(GridRequest::Reconcile {
                tab: None,
                only: None,
            }),
            ActivityRequest::Grid(GridRequest::Plan { tab: None }),
            ActivityRequest::Cost(CostRequest::QuerySpend {
                session_pattern: "x".to_string(),
            }),
        ];

        for req in &reqs {
            let err = activities
                .execute(req)
                .expect_err("deferred variants must fail with ExecutionFailed");
            match err {
                ActivityError::ExecutionFailed(msg) => {
                    assert!(
                        msg.contains("W2") || msg.contains("deferred"),
                        "message must explain deferral to W2: {msg}"
                    );
                }
                other => panic!("expected ExecutionFailed, got: {other:?}"),
            }
        }
    }
}
