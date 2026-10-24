//! Tests for `quaqua cos tag` and CoS pane protection in worker cleanup,
//! grid reconcile, and gate.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use quaqua::cos::{is_cos_pane_token, is_cos_tokens};
use quaqua::gate::policy::{check_with_lookups, OfflinePrLookup, PaneLookup, Verdict};
use quaqua::grid::apply::apply_ops;
use quaqua::grid::types::{Op, Pane, Snapshot, Target, Tokens};
use quaqua::herdr::{HerdrDriver, HerdrOps};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("quaqua-cos-test-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_herdr_stub(dir: &Path, log_file: &Path) -> PathBuf {
    let stub = dir.join("herdr");
    let script = format!(
        r#"#!/bin/sh
echo "$*" >> "{}"
case "$1 $2" in
  "pane current")
    echo '{{"result":{{"pane":{{"pane_id":"w1:pCurrent"}}}}}}'
    ;;
  "pane get")
    if [ "$3" = "w1:pCos" ]; then
      echo '{{"result":{{"pane":{{"pane_id":"w1:pCos","tokens":{{"grid_role":"cos"}}}}}}}}'
    else
      echo '{{"result":{{"pane":{{"pane_id":"$3","tokens":{{"grid_role":"worker"}}}}}}}}'
    fi
    ;;
  *)
    echo '{{"result":{{"ok":true}}}}'
    ;;
esac
exit 0
"#,
        log_file.display()
    );
    fs::write(&stub, script).expect("write stub");
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).expect("chmod stub");
    stub
}

#[test]
fn test_is_cos_pane_token_helper() {
    assert!(is_cos_pane_token("cos"));
    assert!(is_cos_pane_token("grid_role=cos"));
    assert!(!is_cos_pane_token("worker"));
    assert!(!is_cos_pane_token("orchestrator"));
    assert!(!is_cos_pane_token("lead"));
    assert!(!is_cos_pane_token(""));

    let cos_json = serde_json::json!({
        "grid_role": "cos",
        "grid_owner": "cos"
    });
    assert!(is_cos_tokens(&cos_json));

    let worker_json = serde_json::json!({
        "grid_role": "worker",
        "grid_owner": "cos"
    });
    assert!(!is_cos_tokens(&worker_json));

    let empty_json = serde_json::json!({});
    assert!(!is_cos_tokens(&empty_json));
}

#[test]
fn test_cos_tag_cli_with_explicit_pane() {
    let bin = env!("CARGO_BIN_EXE_quaqua");
    let dir = temp_dir("tag-explicit");
    let log = dir.join("herdr.log");
    let stub = write_herdr_stub(&dir, &log);

    let out = Command::new(bin)
        .args(["cos", "tag", "--pane", "w1:p1"])
        .env("HERDR_BIN_PATH", &stub)
        .output()
        .expect("run quaqua cos tag");

    assert!(out.status.success(), "cos tag failed: {:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Tagged pane w1:p1 as grid_role=cos (🍑 CoS Quả Quả)"), "stdout: {stdout}");
    assert!(stdout.contains("ui.sidebar.agents"), "stdout lacks sidebar hint: {stdout}");
    assert!(stdout.contains("$grid_role"), "stdout lacks $grid_role in hint: {stdout}");
    assert!(stdout.contains("herdr server reload-config"), "stdout lacks reload hint: {stdout}");

    let calls = fs::read_to_string(&log).expect("read log");
    let lines: Vec<&str> = calls.lines().collect();
    assert_eq!(
        lines,
        vec![
            "pane report-metadata w1:p1 --source herdr-quaqua --token grid_role=cos --display-agent 🍑 CoS Quả Quả",
            "pane rename w1:p1 🍑 CoS",
        ]
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_cos_tag_cli_idempotent() {
    let bin = env!("CARGO_BIN_EXE_quaqua");
    let dir = temp_dir("tag-idempotent");
    let log = dir.join("herdr.log");
    let stub = write_herdr_stub(&dir, &log);

    for run_idx in 1..=2 {
        let out = Command::new(bin)
            .args(["cos", "tag", "--pane", "w1:p42"])
            .env("HERDR_BIN_PATH", &stub)
            .output()
            .expect("run quaqua cos tag");

        assert!(out.status.success(), "run {run_idx} failed: {:?}", out);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("Tagged pane w1:p42 as grid_role=cos (🍑 CoS Quả Quả)"));
    }

    let calls = fs::read_to_string(&log).expect("read log");
    let lines: Vec<&str> = calls.lines().collect();
    assert_eq!(
        lines,
        vec![
            "pane report-metadata w1:p42 --source herdr-quaqua --token grid_role=cos --display-agent 🍑 CoS Quả Quả",
            "pane rename w1:p42 🍑 CoS",
            "pane report-metadata w1:p42 --source herdr-quaqua --token grid_role=cos --display-agent 🍑 CoS Quả Quả",
            "pane rename w1:p42 🍑 CoS",
        ]
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_cos_tag_cli_resolves_env_pane() {
    let bin = env!("CARGO_BIN_EXE_quaqua");
    let dir = temp_dir("tag-env");
    let log = dir.join("herdr.log");
    let stub = write_herdr_stub(&dir, &log);

    let out = Command::new(bin)
        .args(["cos", "tag"])
        .env("HERDR_BIN_PATH", &stub)
        .env("HERDR_PANE_ID", "w1:pEnv")
        .output()
        .expect("run quaqua cos tag");

    assert!(out.status.success(), "cos tag failed: {:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Tagged pane w1:pEnv as grid_role=cos (🍑 CoS Quả Quả)"));

    let calls = fs::read_to_string(&log).expect("read log");
    assert!(calls.contains("pane report-metadata w1:pEnv --source herdr-quaqua --token grid_role=cos --display-agent 🍑 CoS Quả Quả"));
    assert!(calls.contains("pane rename w1:pEnv 🍑 CoS"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_cos_tag_cli_resolves_current_pane() {
    let bin = env!("CARGO_BIN_EXE_quaqua");
    let dir = temp_dir("tag-current");
    let log = dir.join("herdr.log");
    let stub = write_herdr_stub(&dir, &log);

    let out = Command::new(bin)
        .args(["cos", "tag"])
        .env("HERDR_BIN_PATH", &stub)
        .env_remove("HERDR_PANE_ID")
        .output()
        .expect("run quaqua cos tag");

    assert!(out.status.success(), "cos tag failed: {:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Tagged pane w1:pCurrent as grid_role=cos (🍑 CoS Quả Quả)"));

    let calls = fs::read_to_string(&log).expect("read log");
    assert!(calls.contains("pane current"));
    assert!(calls.contains("pane report-metadata w1:pCurrent --source herdr-quaqua --token grid_role=cos --display-agent 🍑 CoS Quả Quả"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_cos_tag_cli_unresolvable_pane_fails() {
    let bin = env!("CARGO_BIN_EXE_quaqua");
    let dir = temp_dir("tag-fail");
    let stub = dir.join("herdr");
    // Stub that fails pane current
    fs::write(&stub, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();

    let out = Command::new(bin)
        .args(["cos", "tag"])
        .env("HERDR_BIN_PATH", &stub)
        .env_remove("HERDR_PANE_ID")
        .output()
        .expect("run quaqua cos tag");

    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot determine pane"), "stderr: {stderr}");

    let _ = fs::remove_dir_all(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Refusal tests: worker cleanup, grid reconcile, and gate
// ─────────────────────────────────────────────────────────────────────────────

struct StubHerdrDriver {
    snapshot: Snapshot,
    moved: Vec<(String, Target)>,
}

impl HerdrDriver for StubHerdrDriver {
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        Ok(self.snapshot.clone())
    }
    fn move_pane(&mut self, pane: &str, target: &Target) -> Result<quaqua::grid::apply::MoveResult, String> {
        self.moved.push((pane.to_string(), target.clone()));
        Ok(quaqua::grid::apply::MoveResult {
            pane_id: pane.to_string(),
            tab_id: "tab:test".to_string(),
            previous_pane_id: None,
        })
    }
    fn rename_pane(&mut self, _pane: &str, _title: &str) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn test_grid_apply_refuses_to_move_cos_pane() {
    let mut driver = StubHerdrDriver {
        snapshot: Snapshot {
            panes: vec![Pane {
                pane_id: "w1:pCos".to_string(),
                tab_id: Some("w1:t1".to_string()),
                workspace_id: None,
                agent: Some("claude".to_string()),
                agent_status: Some("idle".to_string()),
                tokens: Some(Tokens {
                    grid_role: Some("cos".to_string()),
                    grid_owner: Some("cos".to_string()),
                    grid_group: None,
                    grid_tab: None,
                    grid_ready: Some(serde_json::Value::String("1".to_string())),
                    grid_name: None,
                }),
                label: None,
            }],
            ..Default::default()
        },
        moved: vec![],
    };

    let op = Op {
        op: "move".to_string(),
        pane: "w1:pCos".to_string(),
        target: Target::NewTab {
            label: "quaqua-workers".to_string(),
        },
        deferred: false,
    };

    let res = apply_ops(&mut driver, &[op], None);
    assert!(res.is_err(), "apply_ops must fail when targeting CoS pane");
    let err = res.unwrap_err();
    assert!(
        err.contains("refusing to move CoS pane") && err.contains("grid_role=cos"),
        "error message must clearly state refusal: {err}"
    );
    assert!(driver.moved.is_empty(), "move_pane must NEVER be called for CoS pane");
}

struct StubGatePaneLookup {
    cos_pane: String,
}

impl PaneLookup for StubGatePaneLookup {
    fn is_cos_pane(&self, pane: &str) -> bool {
        pane == self.cos_pane
    }
}

#[test]
fn test_gate_refuses_close_cos_pane() {
    let lookup = StubGatePaneLookup {
        cos_pane: "w1:pCos".to_string(),
    };

    // Closing CoS pane must be denied
    let verdict = check_with_lookups("herdr pane close w1:pCos", None, &OfflinePrLookup, &lookup);
    match verdict {
        Verdict::Deny(reason) => {
            assert!(
                reason.contains("refusing to close CoS pane 'w1:pCos'") && reason.contains("grid_role=cos"),
                "reason: {reason}"
            );
        }
        other => panic!("expected Verdict::Deny, got {other:?}"),
    }

    // Closing non-CoS pane must be allowed
    let verdict_other = check_with_lookups("herdr pane close w1:pWorker", None, &OfflinePrLookup, &lookup);
    assert_eq!(verdict_other, Verdict::Allow);
}

#[test]
fn test_gate_refuses_move_cos_pane() {
    let lookup = StubGatePaneLookup {
        cos_pane: "w1:pCos".to_string(),
    };

    // Moving CoS pane must be denied
    let verdict = check_with_lookups("herdr pane move w1:pCos --new-tab", None, &OfflinePrLookup, &lookup);
    match verdict {
        Verdict::Deny(reason) => {
            assert!(
                reason.contains("refusing to move CoS pane 'w1:pCos'") && reason.contains("grid_role=cos"),
                "reason: {reason}"
            );
        }
        other => panic!("expected Verdict::Deny, got {other:?}"),
    }

    // Moving non-CoS pane must be Block (standard grid policy, not Deny)
    let verdict_other = check_with_lookups("herdr pane move w1:pWorker --new-tab", None, &OfflinePrLookup, &lookup);
    assert_eq!(verdict_other, Verdict::Block);
}

struct CleanupTestHerdr {
    pane_id: String,
    role: String,
    close_called: std::cell::Cell<bool>,
}

impl HerdrOps for CleanupTestHerdr {
    fn run(&self, args: &[&str]) -> std::io::Result<std::process::Output> {
        let stdout = if args.len() >= 3 && args[0] == "pane" && args[1] == "get" {
            format!(
                r#"{{"result":{{"pane":{{"pane_id":"{}","tokens":{{"grid_role":"{}"}}}}}}}}"#,
                self.pane_id, self.role
            )
        } else {
            r#"{"result":{"ok":true}}"#.to_string()
        };
        Ok(std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: stdout.into_bytes(),
            stderr: vec![],
        })
    }
    fn agent_list(&self) -> std::io::Result<String> {
        Ok(format!(r#"{{"result":{{"agents":[{{"name":"test-worker","pane_id":"{}"}}]}}}}"#, self.pane_id))
    }
    fn agent_get(&self, _name: &str) -> std::io::Result<(bool, String)> {
        Ok((true, format!(r#"{{"result":{{"agent":{{"name":"test-worker","pane_id":"{}"}}}}}}"#, self.pane_id)))
    }
    fn agent_read(&self, _name: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        unimplemented!()
    }
    fn agent_start(&self, _: &str, _: &str, _: &str, _: u32, _: &[&str]) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn pane_split(&self, _: &str, _: &str, _: &[(&str, &str)]) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn pane_close(&self, _pane: &str) -> std::io::Result<std::process::Output> {
        self.close_called.set(true);
        Ok(std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: vec![],
            stderr: vec![],
        })
    }
    fn pane_report_metadata(&self, _: &str, _: &[&str]) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn pane_wait_output(&self, _: &str, _: &str, _: u32) -> bool {
        unimplemented!()
    }
    fn pane_read(&self, _: &str, _: &str, _: u32) -> std::io::Result<String> {
        unimplemented!()
    }
    fn pane_send_keys(&self, _: &str, _: &str) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn agent_send_keys(&self, _: &str, _: &str) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn agent_wait_idle(&self, _: &str, _: u32) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn agent_prompt_nowait(&self, _: &str, _: &str) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn agent_prompt_wait_background(&self, _: &str, _: &str, _: u32, _: std::fs::File) -> std::io::Result<std::process::Child> {
        unimplemented!()
    }
}

struct DummyGit;
impl quaqua::git::GitOps for DummyGit {
    fn worktree_remove(&self, _repo: &Path, _wt: &Path) -> std::io::Result<std::process::Output> {
        Ok(std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: vec![],
            stderr: vec![],
        })
    }
    fn run_in(&self, _dir: &Path, _args: &[&str]) -> std::io::Result<std::process::Output> { unimplemented!() }
    fn run(&self, _args: &[&str]) -> std::io::Result<std::process::Output> { unimplemented!() }
    fn rev_parse_commit(&self, _repo: &Path, _base: &str) -> bool { true }
    fn worktree_list_porcelain(&self, _repo: &Path) -> std::io::Result<Vec<String>> { Ok(vec![]) }
    fn worktree_add(&self, _repo: &Path, _branch: &str, _wt: &Path, _base: Option<&str>) -> std::io::Result<std::process::Output> { unimplemented!() }
    fn branch_exists(&self, _repo: &Path, _branch: &str) -> bool { false }
    fn worktree_add_existing(&self, _repo: &Path, _branch: &str, _wt: &Path) -> std::io::Result<std::process::Output> { unimplemented!() }
    fn remote_list(&self, _repo: &Path) -> std::io::Result<Vec<String>> { Ok(vec![]) }
    fn is_core_bare(&self, _repo: &Path) -> bool { false }
    fn has_core_worktree(&self, _repo: &Path) -> bool { false }
    fn enable_worktree_config(&self, _repo: &Path) -> std::io::Result<std::process::Output> { unimplemented!() }
    fn set_worktree_pushurl_blocked(&self, _wt: &Path, _remote: &str) -> std::io::Result<()> { Ok(()) }
    fn is_git_dir(&self, _cwd: &Path) -> bool { true }
    fn diff_and_status(&self, _repo: &Path) -> (String, String) { (String::new(), String::new()) }
}
#[test]
fn test_worker_cleanup_refuses_to_close_cos_pane() {
    let dir = temp_dir("cleanup-refuse");
    let repo = dir.join("repo");
    let runs = dir.join("runs");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&runs).unwrap();

    let herdr = CleanupTestHerdr {
        pane_id: "w1:pCos".to_string(),
        role: "cos".to_string(),
        close_called: std::cell::Cell::new(false),
    };
    let git = DummyGit;

    let res = quaqua::worker::cleanup::cleanup_worker_in("test-worker", &repo, &git, &herdr, Some(&runs));
    assert!(res.is_err(), "cleanup must fail when pane has grid_role=cos");
    let err = res.unwrap_err();
    assert_eq!(err.0, 1);
    assert!(
        err.1.contains("refusing to close CoS pane 'w1:pCos'") && err.1.contains("grid_role=cos"),
        "error message must clearly state refusal: {}",
        err.1
    );
    assert!(!herdr.close_called.get(), "pane_close must NEVER be called for CoS pane");

    let _ = fs::remove_dir_all(&dir);
}
