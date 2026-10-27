//! Tests for `quaqua lead repair <slug>` and token healing.

use quaqua::herdr::HerdrOps;
use quaqua::identity::{heal, heal_with_runs_dir};
use quaqua::lead::tag::{repair_all_leads, repair_lead};
use std::cell::RefCell;
use std::collections::HashMap;
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

#[derive(Clone, Default)]
struct MockPaneInfo {
    tokens: HashMap<String, String>,
    agent_session: Option<String>,
    label: Option<String>,
}

struct MockMultiPaneHerdr {
    panes: RefCell<HashMap<String, MockPaneInfo>>,
    reported_tokens: RefCell<Vec<(String, Vec<String>)>>,
    renamed_panes: RefCell<Vec<(String, String)>>,
    agent_list_json: RefCell<String>,
}

impl MockMultiPaneHerdr {
    fn new(panes: HashMap<String, MockPaneInfo>, agent_list_json: &str) -> Self {
        Self {
            panes: RefCell::new(panes),
            reported_tokens: RefCell::new(Vec::new()),
            renamed_panes: RefCell::new(Vec::new()),
            agent_list_json: RefCell::new(agent_list_json.to_string()),
        }
    }
}

impl HerdrOps for MockMultiPaneHerdr {
    fn run(&self, args: &[&str]) -> std::io::Result<std::process::Output> {
        let stdout = if args.first() == Some(&"pane") && args.get(1) == Some(&"get") {
            let pane_id = args.get(2).copied().unwrap_or("");
            let panes = self.panes.borrow();
            let info = panes.get(pane_id).cloned().unwrap_or_default();
            let tokens_val = serde_json::to_value(&info.tokens).unwrap_or(serde_json::json!({}));
            let session_val = info
                .agent_session
                .as_ref()
                .map(|s| serde_json::json!({"value": s}));
            let mut pane_obj = serde_json::json!({
                "pane_id": pane_id,
                "tokens": tokens_val,
            });
            if let Some(s) = session_val {
                pane_obj["agent_session"] = s;
            }
            if let Some(lbl) = info.label {
                pane_obj["label"] = serde_json::json!(lbl);
            }
            serde_json::to_string(&serde_json::json!({
                "result": {
                    "pane": pane_obj
                }
            }))
            .unwrap()
        } else if args.first() == Some(&"pane") && args.get(1) == Some(&"report-metadata") {
            let pane_id = args.get(2).copied().unwrap_or("").to_string();
            let mut toks = Vec::new();
            let mut i = 3;
            while i < args.len() {
                if args[i] == "--token" && i + 1 < args.len() {
                    toks.push(args[i + 1].to_string());
                    if let Some((k, v)) = args[i + 1].split_once('=') {
                        self.panes
                            .borrow_mut()
                            .entry(pane_id.clone())
                            .or_default()
                            .tokens
                            .insert(k.to_string(), v.to_string());
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            self.reported_tokens.borrow_mut().push((pane_id, toks));
            r#"{"result":{}}"#.to_string()
        } else if args.first() == Some(&"pane") && args.get(1) == Some(&"rename") {
            let pane_id = args.get(2).copied().unwrap_or("").to_string();
            let label = args.get(3).copied().unwrap_or("").to_string();
            self.renamed_panes
                .borrow_mut()
                .push((pane_id.clone(), label.clone()));
            self.panes.borrow_mut().entry(pane_id).or_default().label = Some(label);
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
        for t in _t {
            args.push("--token");
            args.push(t);
        }
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

#[test]
fn test_identity_heal_lead_with_owner_file_restores_tokens() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT", "QUAQUA_RUNS_DIR"]);
    let ws = hermetic_temp_dir("heal-lead-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());
    let runs_dir = ws.join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();
    guard.set("QUAQUA_RUNS_DIR", runs_dir.to_str().unwrap());

    // mu-lead has runs/mu-lead.owner
    std::fs::write(runs_dir.join("mu-lead.owner"), "cos\n").unwrap();

    let mut panes = HashMap::new();
    let mut mu_tokens = HashMap::new();
    mu_tokens.insert("logo".to_string(), "\u{e1a3}".to_string());
    panes.insert(
        "wB:p1".to_string(),
        MockPaneInfo {
            tokens: mu_tokens,
            agent_session: None,
            label: Some("mu-lead".to_string()),
        },
    );

    let agents_json = r#"{"result":{"agents":[{"name":"mu-lead","pane_id":"wB:p1","tokens":{"logo":"\ue1a3"}}]}}"#;
    let herdr = MockMultiPaneHerdr::new(panes, agents_json);

    let actions = heal(&herdr).expect("heal should succeed");
    assert_eq!(actions.len(), 1);
    assert!(actions[0].contains("healed lead 'mu'"));

    // Verify report-metadata was emitted with grid_role=orchestrator and grid_owner=mu-lead
    let rep = herdr.reported_tokens.borrow();
    assert_eq!(rep.len(), 1);
    assert_eq!(rep[0].0, "wB:p1");
    assert!(rep[0].1.iter().any(|t| t == "grid_role=orchestrator"));
    assert!(rep[0].1.iter().any(|t| t == "grid_owner=mu-lead"));

    // Verify tokens were updated on the mock pane
    let p = herdr.panes.borrow();
    assert_eq!(
        p["wB:p1"].tokens.get("grid_role").map(String::as_str),
        Some("orchestrator")
    );
    assert_eq!(
        p["wB:p1"].tokens.get("grid_owner").map(String::as_str),
        Some("mu-lead")
    );
    assert_eq!(
        p["wB:p1"].tokens.get("logo").map(String::as_str),
        Some("\u{e1a3}")
    );
}

#[test]
fn test_identity_heal_cos_matching_session_stamps_cos_token() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT", "QUAQUA_RUNS_DIR"]);
    let ws = hermetic_temp_dir("heal-cos-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());
    let runs_dir = ws.join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();
    guard.set("QUAQUA_RUNS_DIR", runs_dir.to_str().unwrap());

    // CoS identity recorded in runs/cos.identity.json
    std::fs::write(
        runs_dir.join("cos.identity.json"),
        r#"{"agent_session": "ses-cos-target-42", "pane_id": "w1:p1"}"#,
    )
    .unwrap();

    let mut panes = HashMap::new();
    let mut cos_tokens = HashMap::new();
    cos_tokens.insert("logo".to_string(), "\u{e1a0}".to_string());
    panes.insert(
        "w1:p1".to_string(),
        MockPaneInfo {
            tokens: cos_tokens,
            agent_session: Some("ses-cos-target-42".to_string()),
            label: None,
        },
    );

    // Pane has NO herdr agent name, and tokens has only logo (no grid_role)
    let agents_json = r#"{"result":{"agents":[{"pane_id":"w1:p1","agent":"claude","tokens":{"logo":"\ue1a0"},"agent_session":{"value":"ses-cos-target-42"}}]}}"#;
    let herdr = MockMultiPaneHerdr::new(panes, agents_json);

    let actions = heal(&herdr).expect("heal should succeed");
    assert_eq!(actions.len(), 1);
    assert!(actions[0].contains("healed CoS"));

    // Verify report-metadata was emitted with grid_role=cos
    let rep = herdr.reported_tokens.borrow();
    assert!(rep
        .iter()
        .any(|(p, toks)| p == "w1:p1" && toks.iter().any(|t| t == "grid_role=cos")));

    // Verify tokens were updated on the pane
    let p = herdr.panes.borrow();
    assert_eq!(
        p["w1:p1"].tokens.get("grid_role").map(String::as_str),
        Some("cos")
    );
    assert_eq!(
        p["w1:p1"].tokens.get("grid_owner").map(String::as_str),
        Some("cos")
    );
    assert_eq!(
        p["w1:p1"].tokens.get("logo").map(String::as_str),
        Some("\u{e1a0}")
    );

    // Verify rename was emitted
    let renames = herdr.renamed_panes.borrow();
    assert!(renames
        .iter()
        .any(|(p, title)| p == "w1:p1" && title == "🍑 CoS"));
}

#[test]
fn test_identity_heal_cos_fails_closed_on_named_pane_or_session_mismatch() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT", "QUAQUA_RUNS_DIR"]);
    let ws = hermetic_temp_dir("heal-cos-fail-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());
    let runs_dir = ws.join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();
    guard.set("QUAQUA_RUNS_DIR", runs_dir.to_str().unwrap());

    // CoS identity expects ses-cos-target-42
    std::fs::write(
        runs_dir.join("cos.identity.json"),
        r#"{"agent_session": "ses-cos-target-42", "pane_id": "w1:p1"}"#,
    )
    .unwrap();

    let mut panes = HashMap::new();

    // Pane 1: Session mismatch (ses-different)
    panes.insert(
        "w1:p1".to_string(),
        MockPaneInfo {
            tokens: HashMap::new(),
            agent_session: Some("ses-different".to_string()),
            label: None,
        },
    );

    // Pane 2: Matching session, BUT has an agent name ("worker-alice")
    panes.insert(
        "w2:p1".to_string(),
        MockPaneInfo {
            tokens: HashMap::new(),
            agent_session: Some("ses-cos-target-42".to_string()),
            label: Some("worker-alice".to_string()),
        },
    );

    // Pane 3: Matching session, no name, BUT already carries grid_role=worker
    let mut p3_tokens = HashMap::new();
    p3_tokens.insert("grid_role".to_string(), "worker".to_string());
    panes.insert(
        "w3:p1".to_string(),
        MockPaneInfo {
            tokens: p3_tokens,
            agent_session: Some("ses-cos-target-42".to_string()),
            label: None,
        },
    );

    let agents_json = r#"{"result":{"agents":[
        {"pane_id":"w1:p1","agent":"claude","agent_session":{"value":"ses-different"}},
        {"pane_id":"w2:p1","name":"worker-alice","agent":"claude","agent_session":{"value":"ses-cos-target-42"}},
        {"pane_id":"w3:p1","agent":"claude","tokens":{"grid_role":"worker"},"agent_session":{"value":"ses-cos-target-42"}}
    ]}}"#;

    let herdr = MockMultiPaneHerdr::new(panes, agents_json);

    let actions = heal(&herdr).expect("heal should succeed without errors");
    // None of the panes should have been tagged cos
    assert!(actions.is_empty(), "expected 0 actions, got: {:?}", actions);
    assert!(herdr.reported_tokens.borrow().is_empty());
    assert!(herdr.renamed_panes.borrow().is_empty());
}

#[test]
fn test_identity_heal_worker_restores_tokens() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT", "QUAQUA_RUNS_DIR"]);
    let ws = hermetic_temp_dir("heal-worker-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());
    let runs_dir = ws.join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();
    guard.set("QUAQUA_RUNS_DIR", runs_dir.to_str().unwrap());

    // Worker metadata files
    std::fs::write(runs_dir.join("sude-task.owner"), "mu-lead\n").unwrap();
    std::fs::write(runs_dir.join("sude-task.grid"), "mu-workers\n").unwrap();
    std::fs::write(runs_dir.join("sude-task.repo"), "~/Lab/mu\n").unwrap();

    let mut panes = HashMap::new();
    let mut worker_tokens = HashMap::new();
    worker_tokens.insert("logo".to_string(), "\u{e1a3}".to_string());
    panes.insert(
        "wC:p2".to_string(),
        MockPaneInfo {
            tokens: worker_tokens,
            agent_session: None,
            label: Some("sude-task".to_string()),
        },
    );

    let agents_json = r#"{"result":{"agents":[{"name":"sude-task","pane_id":"wC:p2","tokens":{"logo":"\ue1a3"}}]}}"#;
    let herdr = MockMultiPaneHerdr::new(panes, agents_json);

    let actions = heal(&herdr).expect("heal should succeed");
    assert_eq!(actions.len(), 1);
    assert!(actions[0].contains("healed worker 'sude-task'"));

    let rep = herdr.reported_tokens.borrow();
    assert_eq!(rep.len(), 1);
    assert_eq!(rep[0].0, "wC:p2");
    assert!(rep[0].1.iter().any(|t| t == "grid_role=worker"));
    assert!(rep[0].1.iter().any(|t| t == "grid_owner=mu-lead"));
    assert!(rep[0].1.iter().any(|t| t == "grid_tab=mu-workers"));
    assert!(rep[0].1.iter().any(|t| t == "grid_name=sude-task"));
    assert!(rep[0].1.iter().any(|t| t == "grid_group=mu"));

    let p = herdr.panes.borrow();
    assert_eq!(
        p["wC:p2"].tokens.get("grid_role").map(String::as_str),
        Some("worker")
    );
    assert_eq!(
        p["wC:p2"].tokens.get("grid_owner").map(String::as_str),
        Some("mu-lead")
    );
    assert_eq!(
        p["wC:p2"].tokens.get("grid_tab").map(String::as_str),
        Some("mu-workers")
    );
    assert_eq!(
        p["wC:p2"].tokens.get("grid_name").map(String::as_str),
        Some("sude-task")
    );
    assert_eq!(
        p["wC:p2"].tokens.get("grid_group").map(String::as_str),
        Some("mu")
    );
    assert_eq!(
        p["wC:p2"].tokens.get("logo").map(String::as_str),
        Some("\u{e1a3}")
    );
}

#[test]
fn test_cos_tag_records_cos_identity_json() {
    let guard = EnvGuard::new(&["QUAQUA_ROOT", "QUAQUA_RUNS_DIR"]);
    let ws = hermetic_temp_dir("cos-tag-record-ws");
    guard.set("QUAQUA_ROOT", ws.to_str().unwrap());
    let runs_dir = ws.join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();
    guard.set("QUAQUA_RUNS_DIR", runs_dir.to_str().unwrap());

    let mut panes = HashMap::new();
    panes.insert(
        "w1:p1".to_string(),
        MockPaneInfo {
            tokens: HashMap::new(),
            agent_session: Some("ses-claude-999".to_string()),
            label: None,
        },
    );

    let agents_json = r#"{"result":{"agents":[{"pane_id":"w1:p1","agent":"claude","tokens":{},"agent_session":{"value":"ses-claude-999"}}]}}"#;
    let herdr = MockMultiPaneHerdr::new(panes, agents_json);

    let res = quaqua::cos::tag_cos_pane_in(&herdr, "w1:p1", &runs_dir)
        .expect("tag_cos_pane_in should succeed");
    assert!(res.contains("Tagged pane w1:p1 as grid_role=cos"));

    // Verify runs/cos.identity.json was written with agent_session and pane_id
    let ident_path = runs_dir.join("cos.identity.json");
    assert!(ident_path.is_file(), "cos.identity.json should be created");
    let content = std::fs::read_to_string(ident_path).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(val["agent_session"], "ses-claude-999");
    assert_eq!(val["pane_id"], "w1:p1");

    // Simulate herdr restart: tokens are wiped back to only logo
    herdr
        .panes
        .borrow_mut()
        .get_mut("w1:p1")
        .unwrap()
        .tokens
        .clear();
    herdr
        .panes
        .borrow_mut()
        .get_mut("w1:p1")
        .unwrap()
        .tokens
        .insert("logo".to_string(), "\u{e1a0}".to_string());
    let post_restart_agents = r#"{"result":{"agents":[{"pane_id":"w1:p1","agent":"claude","tokens":{"logo":"\ue1a0"},"agent_session":{"value":"ses-claude-999"}}]}}"#;
    *herdr.agent_list_json.borrow_mut() = post_restart_agents.to_string();

    // Heal restores the CoS pane
    let actions = heal(&herdr).expect("heal should restore CoS");
    assert_eq!(actions.len(), 1);
    assert!(actions[0].contains("healed CoS"));
    assert_eq!(
        herdr.panes.borrow()["w1:p1"]
            .tokens
            .get("grid_role")
            .map(String::as_str),
        Some("cos")
    );
}
