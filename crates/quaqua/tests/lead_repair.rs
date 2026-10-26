//! Tests for `quaqua lead repair <slug>` and token healing.

use quaqua::herdr::HerdrOps;
use quaqua::lead::tag::{repair_all_leads, repair_lead};
use std::cell::RefCell;
use std::path::PathBuf;

fn hermetic_temp_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "quaqua-lead-repair-test-{prefix}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create hermetic temp dir");
    dir
}

static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct EnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    vars: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn new(keys: &[&'static str]) -> Self {
        let lock = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let vars = keys.iter().map(|&k| (k, std::env::var(k).ok())).collect();
        Self { _lock: lock, vars }
    }
    fn set(&self, key: &'static str, val: &str) {
        std::env::set_var(key, val);
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.vars {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }
}

struct MockRepairHerdr {
    tokens_json: RefCell<String>,
    reported_tokens: RefCell<Vec<String>>,
    agent_list_json: RefCell<String>,
}

impl MockRepairHerdr {
    fn new(tokens_json: &str, agents_json: &str) -> Self {
        Self {
            tokens_json: RefCell::new(tokens_json.to_string()),
            reported_tokens: RefCell::new(Vec::new()),
            agent_list_json: RefCell::new(agents_json.to_string()),
        }
    }
}

impl HerdrOps for MockRepairHerdr {
    fn run(&self, args: &[&str]) -> std::io::Result<std::process::Output> {
        let stdout = if args.first() == Some(&"pane") && args.get(1) == Some(&"get") {
            format!(
                r#"{{"result":{{"pane":{{"pane_id":"wB:p1","tokens":{}}}}}}}"#,
                self.tokens_json.borrow()
            )
        } else if args.first() == Some(&"pane") && args.get(1) == Some(&"report-metadata") {
            let mut role = None;
            let mut owner = None;
            for arg in args {
                self.reported_tokens.borrow_mut().push(arg.to_string());
                if let Some(r) = arg.strip_prefix("grid_role=") {
                    role = Some(r);
                } else if let Some(o) = arg.strip_prefix("grid_owner=") {
                    owner = Some(o);
                }
            }
            if let (Some(r), Some(o)) = (role, owner) {
                *self.tokens_json.borrow_mut() =
                    format!(r#"{{"grid_role":"{r}","grid_owner":"{o}","logo":"\ue1a3"}}"#);
            }
            r#"{"result":{}}"#.to_string()
        } else if args.first() == Some(&"agent") && args.get(1) == Some(&"list") {
            self.agent_list_json.borrow().clone()
        } else {
            r#"{"result":{}}"#.to_string()
        };
        Ok(std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: stdout.into_bytes(),
            stderr: Vec::new(),
        })
    }

    fn agent_list(&self) -> std::io::Result<String> {
        Ok(self.agent_list_json.borrow().clone())
    }

    fn agent_get(&self, _n: &str) -> std::io::Result<(bool, String)> {
        unimplemented!()
    }
    fn agent_read(&self, _n: &str, _s: &str, _l: u32) -> std::io::Result<String> {
        Ok(String::new())
    }
    fn agent_start(
        &self,
        _n: &str,
        _k: &str,
        _p: &str,
        _t: u32,
        _a: &[&str],
    ) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn pane_split(
        &self,
        _c: &str,
        _o: &str,
        _e: &[(&str, &str)],
    ) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn pane_close(&self, _p: &str) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn pane_report_metadata(&self, _p: &str, _t: &[&str]) -> std::io::Result<std::process::Output> {
        let mut args = vec!["pane", "report-metadata", _p];
        args.extend_from_slice(_t);
        self.run(&args)
    }
    fn pane_wait_output(&self, _p: &str, _m: &str, _t: u32) -> bool {
        true
    }
    fn pane_read(&self, _p: &str, _s: &str, _l: u32) -> std::io::Result<String> {
        Ok(String::new())
    }
    fn pane_send_keys(&self, _p: &str, _k: &str) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn agent_send_keys(&self, _n: &str, _key: &str) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn agent_wait_idle(&self, _n: &str, _t: u32) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn agent_prompt_nowait(&self, _n: &str, _p: &str) -> std::io::Result<std::process::Output> {
        unimplemented!()
    }
    fn agent_prompt_wait_background(
        &self,
        _n: &str,
        _p: &str,
        _t: u32,
        _f: std::fs::File,
    ) -> std::io::Result<std::process::Child> {
        unimplemented!()
    }
}

#[test]
fn test_repair_lead_success_stamps_tokens() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT"]);
    let ws = hermetic_temp_dir("repair-lead-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());

    let projects_dir = ws.join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();
    std::fs::write(
        projects_dir.join("INDEX.md"),
        "| slug | repo | owner | status | goal |\n|---|---|---|---|---|\n| mu | ~/Lab/mu | mu-lead | active | Mole parity |\n",
    ).unwrap();

    let herdr = MockRepairHerdr::new(
        r#"{"logo":"\ue1a3"}"#,
        r#"{"result":{"agents":[{"name":"mu-lead","pane_id":"wB:p1","tokens":{"logo":"\ue1a3"}}]}}"#,
    );

    let res = repair_lead(&herdr, "mu", None).expect("repair_lead should succeed");
    assert!(res.contains("repaired lead 'mu'"));
    assert!(res.contains("grid_role=orchestrator"));
    assert!(res.contains("grid_owner=mu-lead"));

    // Verify token was stamped
    assert_eq!(
        *herdr.tokens_json.borrow(),
        r#"{"grid_role":"orchestrator","grid_owner":"mu-lead","logo":"\ue1a3"}"#
    );

    // Running repair a second time reports already tagged
    let res2 = repair_lead(&herdr, "mu", None).expect("second repair should report already tagged");
    assert!(res2.contains("already tagged"));
}

#[test]
fn test_repair_lead_rejects_unregistered_slug() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT"]);
    let ws = hermetic_temp_dir("repair-unreg-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());

    let projects_dir = ws.join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();
    std::fs::write(
        projects_dir.join("INDEX.md"),
        "| slug | repo | owner | status | goal |\n|---|---|---|---|---|\n| mu | ~/Lab/mu | mu-lead | active | Mole parity |\n",
    ).unwrap();

    let herdr = MockRepairHerdr::new("{}", r#"{"result":{"agents":[]}}"#);
    let err = repair_lead(&herdr, "unknown-proj", None).expect_err("unknown slug must fail");
    assert!(
        err.contains("unknown project slug 'unknown-proj'"),
        "got: {err}"
    );
}

#[test]
fn test_repair_lead_rejects_cos_owned_project() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT"]);
    let ws = hermetic_temp_dir("repair-cos-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());

    let projects_dir = ws.join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();
    std::fs::write(
        projects_dir.join("INDEX.md"),
        "| slug | repo | owner | status | goal |\n|---|---|---|---|---|\n| arch | ~/Lab/quaqua | cos | active | arch goal |\n",
    ).unwrap();

    let herdr = MockRepairHerdr::new("{}", r#"{"result":{"agents":[]}}"#);
    let err = repair_lead(&herdr, "arch", None).expect_err("cos owned project must fail");
    assert!(
        err.contains("owner in projects/INDEX.md is 'cos', not 'arch-lead'"),
        "got: {err}"
    );
}

#[test]
fn test_repair_lead_fails_when_agent_not_running() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT"]);
    let ws = hermetic_temp_dir("repair-noagent-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());

    let projects_dir = ws.join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();
    std::fs::write(
        projects_dir.join("INDEX.md"),
        "| slug | repo | owner | status | goal |\n|---|---|---|---|---|\n| mu | ~/Lab/mu | mu-lead | active | Mole parity |\n",
    ).unwrap();

    let herdr = MockRepairHerdr::new("{}", r#"{"result":{"agents":[]}}"#);
    let err = repair_lead(&herdr, "mu", None).expect_err("agent not running must fail");
    assert!(
        err.contains("agent 'mu-lead' is not registered or running in herdr"),
        "got: {err}"
    );
}

#[test]
fn test_repair_all_leads_repairs_multiple() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT"]);
    let ws = hermetic_temp_dir("repair-all-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());

    let projects_dir = ws.join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();
    std::fs::write(
        projects_dir.join("INDEX.md"),
        "| slug | repo | owner | status | goal |\n|---|---|---|---|---|\n| mu | ~/Lab/mu | mu-lead | active | Mole |\n| llmhub | ~/Lab/llmhub | llmhub-lead | active | CLI |\n",
    ).unwrap();

    let herdr = MockRepairHerdr::new(
        r#"{"logo":"\ue1a3"}"#,
        r#"{"result":{"agents":[{"name":"mu-lead","pane_id":"wB:p1","tokens":{"logo":"\ue1a3"}},{"name":"llmhub-lead","pane_id":"wA:p1","tokens":{"logo":"\ue1a3"}}]}}"#,
    );

    let res = repair_all_leads(&herdr).expect("repair_all_leads should succeed");
    assert_eq!(res.len(), 2);
    assert!(res[0].contains("repaired lead 'mu'"));
    assert!(res[1].contains("repaired lead 'llmhub'"));
}

#[test]
fn test_repair_lead_fails_closed_when_workspace_root_unresolvable() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT", "QUAQUA_HOME", "HOME"]);
    let dir = hermetic_temp_dir("no-index-ws");
    guard.set("QUAQUA_ROOT", dir.to_str().unwrap());

    let herdr = MockRepairHerdr::new("{}", r#"{"result":{"agents":[]}}"#);
    let err = repair_lead(&herdr, "totally-made-up", Some("wC:p7"))
        .expect_err("repair_lead must fail closed when workspace INDEX is unresolvable");
    assert!(
        err.contains("projects/INDEX.md is missing")
            || err.contains("no quaqua workspace")
            || err.contains("cannot resolve workspace root"),
        "got: {err}"
    );
}

#[test]
fn test_repair_lead_rejects_pane_mismatch() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT"]);
    let ws = hermetic_temp_dir("repair-mismatch-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());

    let projects_dir = ws.join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();
    std::fs::write(
        projects_dir.join("INDEX.md"),
        "| slug | repo | owner | status | goal |\n|---|---|---|---|---|\n| mu | ~/Lab/mu | mu-lead | active | Mole parity |\n",
    ).unwrap();

    let herdr = MockRepairHerdr::new(
        r#"{"logo":"\ue1a3"}"#,
        r#"{"result":{"agents":[{"name":"mu-lead","pane_id":"wB:p1","tokens":{"logo":"\ue1a3"}}]}}"#,
    );

    let err = repair_lead(&herdr, "mu", Some("wC:p7"))
        .expect_err("repair_lead must refuse when explicit pane does not match agent pane");
    assert!(err.contains("wC:p7") && err.contains("wB:p1"), "got: {err}");
}

#[test]
fn test_repair_all_leads_fails_if_any_lead_fails() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT"]);
    let ws = hermetic_temp_dir("repair-all-fail-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());

    let projects_dir = ws.join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();
    std::fs::write(
        projects_dir.join("INDEX.md"),
        "| slug | repo | owner | status | goal |\n|---|---|---|---|---|\n| mu | ~/Lab/mu | mu-lead | active | Mole |\n| llmhub | ~/Lab/llmhub | llmhub-lead | active | CLI |\n",
    ).unwrap();

    // Only mu-lead is running; llmhub-lead is offline
    let herdr = MockRepairHerdr::new(
        r#"{"logo":"\ue1a3"}"#,
        r#"{"result":{"agents":[{"name":"mu-lead","pane_id":"wB:p1","tokens":{"logo":"\ue1a3"}}]}}"#,
    );

    let err = repair_all_leads(&herdr)
        .expect_err("repair_all_leads must return error when any lead fails to repair");
    assert!(err.contains("llmhub"), "got: {err}");
}

#[test]
fn test_repair_all_leads_ignores_non_matching_lead_owner() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT"]);
    let ws = hermetic_temp_dir("repair-all-filter-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());

    let projects_dir = ws.join("projects");
    std::fs::create_dir_all(&projects_dir).unwrap();
    std::fs::write(
        projects_dir.join("INDEX.md"),
        "| slug | repo | owner | status | goal |\n|---|---|---|---|---|\n| mu | ~/Lab/mu | other-lead | active | Mole |\n| llmhub | ~/Lab/llmhub | llmhub-lead | active | CLI |\n",
    ).unwrap();

    // Only llmhub-lead is running
    let herdr = MockRepairHerdr::new(
        r#"{"logo":"\ue1a3"}"#,
        r#"{"result":{"agents":[{"name":"llmhub-lead","pane_id":"wA:p1","tokens":{"logo":"\ue1a3"}}]}}"#,
    );

    let res = repair_all_leads(&herdr)
        .expect("repair_all_leads should ignore row where owner is not <slug>-lead and succeed for matching rows");
    assert_eq!(res.len(), 1);
    assert!(res[0].contains("repaired lead 'llmhub'"));
}
