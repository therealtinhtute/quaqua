//! Integration tests for layout call sites (P8 U4).
//!
//! Verifies:
//! - `lead_spawn_denied_leaves_no_snapshot`
//! - `user_layout_chair_lead_spawn_allowed`
//! - `prompt_gate_uses_effective_layout`
//! - `spawn_refused_leaves_no_layout_role`

use std::cell::RefCell;
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Output};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use quaqua::config::Config;
use quaqua::git::GitOps;
use quaqua::herdr::HerdrOps;
use quaqua::layout::gate::{gate, SpawnOrigin};
use quaqua::layout::model::Verb;
use quaqua::layout::read_layout_role;
use quaqua::lead::spawn::spawn_lead;

static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn new(keys: &[&'static str]) -> Self {
        let saved = keys.iter().map(|&k| (k, std::env::var(k).ok())).collect();
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }
}

fn hermetic_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("quaqua-u4-{prefix}-{nanos}"));
    fs::create_dir_all(&dir).expect("create hermetic temp dir");
    dir
}

struct DummyGit;

impl GitOps for DummyGit {
    fn run_in(&self, _dir: &Path, _args: &[&str]) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn run(&self, _args: &[&str]) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn rev_parse_commit(&self, _repo: &Path, _base: &str) -> bool {
        true
    }
    fn worktree_list_porcelain(&self, _repo: &Path) -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }
    fn worktree_add(
        &self,
        _repo: &Path,
        _branch: &str,
        _wt: &Path,
        _base: Option<&str>,
    ) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn branch_exists(&self, _repo: &Path, _branch: &str) -> bool {
        true
    }
    fn worktree_add_existing(
        &self,
        _repo: &Path,
        _branch: &str,
        _wt: &Path,
    ) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn worktree_remove(&self, _repo: &Path, _wt: &Path) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn remote_list(&self, _repo: &Path) -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }
    fn is_core_bare(&self, _repo: &Path) -> bool {
        false
    }
    fn has_core_worktree(&self, _repo: &Path) -> bool {
        false
    }
    fn enable_worktree_config(&self, _repo: &Path) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn set_worktree_pushurl_blocked(&self, _wt: &Path, _remote: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn is_git_dir(&self, _cwd: &Path) -> bool {
        true
    }
    fn diff_and_status(&self, _cwd: &Path) -> (String, String) {
        (String::new(), String::new())
    }
}

struct DummyHerdr {
    calls: RefCell<Vec<String>>,
    started: std::cell::Cell<bool>,
    fail_workspace: bool,
}

impl DummyHerdr {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            started: std::cell::Cell::new(false),
            fail_workspace: false,
        }
    }
}

impl HerdrOps for DummyHerdr {
    fn run(&self, args: &[&str]) -> std::io::Result<Output> {
        self.calls.borrow_mut().push(args.join(" "));
        let stdout = if args.first() == Some(&"workspace") && args.get(1) == Some(&"create") {
            if self.fail_workspace {
                return Ok(Output {
                    status: std::process::ExitStatus::from_raw(1),
                    stdout: Vec::new(),
                    stderr: b"error: mock workspace create failed\n".to_vec(),
                });
            }
            r#"{"result":{"workspace":{"workspace_id":"w1"},"root_pane":{"pane_id":"w1:p1"}}}"#
        } else if args.first() == Some(&"agent") && args.get(1) == Some(&"start") {
            self.started.set(true);
            r#"{"result":{}}"#
        } else {
            r#"{"result":{}}"#
        };
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        })
    }
    fn agent_list(&self) -> std::io::Result<String> {
        Ok(r#"{"result":{"agents":[]}}"#.to_string())
    }
    fn agent_get(&self, _name: &str) -> std::io::Result<(bool, String)> {
        if self.started.get() {
            Ok((
                true,
                r#"{"result":{"agent":{"agent_status":"working","pane_id":"w1:p1"}}}"#.to_string(),
            ))
        } else {
            Ok((false, r#"{"error":{"code":"agent_not_found"}}"#.to_string()))
        }
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
    ) -> std::io::Result<Output> {
        self.started.set(true);
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn pane_split(&self, _c: &str, _o: &str, _e: &[(&str, &str)]) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: br#"{"result":{"pane":{"pane_id":"w1:p2"}}}"#.to_vec(),
            stderr: Vec::new(),
        })
    }
    fn pane_close(&self, _p: &str) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn pane_report_metadata(&self, _p: &str, _t: &[&str]) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn pane_wait_output(&self, _p: &str, _m: &str, _t: u32) -> bool {
        true
    }
    fn pane_read(&self, _p: &str, _s: &str, _l: u32) -> std::io::Result<String> {
        Ok(String::new())
    }
    fn pane_send_keys(&self, _p: &str, _k: &str) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn agent_send_keys(&self, _n: &str, _key: &str) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn agent_wait_idle(&self, _n: &str, _t: u32) -> std::io::Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn agent_prompt_nowait(&self, name: &str, prompt: &str) -> std::io::Result<Output> {
        self.run(&["agent", "prompt", name, prompt])
    }
    fn agent_prompt_wait_background(
        &self,
        _name: &str,
        _prompt: &str,
        _timeout_ms: u32,
        _log_file: std::fs::File,
    ) -> std::io::Result<Child> {
        unimplemented!()
    }
}

/// Proving Test 1: A lead spawn denied by the layout gate must leave no frozen snapshot file.
#[test]
fn lead_spawn_denied_leaves_no_snapshot() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp_root = hermetic_temp_dir("lead-denied");
    let runs_dir = temp_root.join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let _env = EnvGuard::new(&[
        "QUAQUA_TEST",
        "QUAQUA_RUNS_DIR",
        "QUAQUA_ROOT",
        "QUAQUA_HOME",
        "HOME",
        "QUAQUA_OWNER",
        "QUAQUA_LAYOUT_SHADOW",
        "QUAQUA_TEAM_TOML",
    ]);
    std::env::set_var("QUAQUA_TEST", "1");
    std::env::set_var("QUAQUA_ROOT", &temp_root);
    std::env::set_var("QUAQUA_HOME", &temp_root);
    std::env::set_var("HOME", &temp_root);
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_OWNER", "parent-lead");
    std::env::remove_var("QUAQUA_LAYOUT_SHADOW");

    // Standard layout where cos -> lead -> peer.
    // parent-lead cannot spawn lead (lead.can_spawn = ["peer", "taskforce"]).
    let team_toml_path = temp_root.join("team.toml");
    let team_toml_content = r#"
schema_version = 3
[strategy]
default_kind = "sude"
lead_kind = "sude"

[kinds.sude]
harness = "omp"
model = "claude-3-7-sonnet"
effort = "high"
"#;
    fs::write(&team_toml_path, team_toml_content).unwrap();
    std::env::set_var("QUAQUA_TEAM_TOML", &team_toml_path);

    // Create projects/INDEX.md, projects/testslug.md, rules/_all
    fs::create_dir_all(temp_root.join("rules/_all")).unwrap();
    fs::create_dir_all(temp_root.join("projects")).unwrap();
    fs::write(
        temp_root.join("projects/INDEX.md"),
        "| slug | repo | owner | status | one-line goal |\n|------|------|-------|--------|---------------|\n| testslug | ~/Lab/testslug | cos | active | goal |\n",
    )
    .unwrap();
    fs::write(temp_root.join("projects/testslug.md"), "# testslug card\n").unwrap();

    let git = DummyGit;
    let herdr = DummyHerdr::new();

    let res = spawn_lead(&["testslug".to_string()], &git, &herdr);
    println!("spawn_lead returned: {res:?}");
    assert!(res.is_err(), "lead spawn must be denied by layout gate");

    let snapshot_file = runs_dir.join("testslug.layout.toml");
    assert!(
        !snapshot_file.exists(),
        "denied lead spawn must leave no layout snapshot in runs/<slug>.layout.toml"
    );
}

/// Proving Test 2: In a user layout where lead-class role is named "chair",
/// lead spawn passes target class "lead" which maps to "chair" and is allowed.
#[test]
fn user_layout_chair_lead_spawn_allowed() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp_root = hermetic_temp_dir("chair-lead");
    let runs_dir = temp_root.join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let _env = EnvGuard::new(&[
        "QUAQUA_TEST",
        "QUAQUA_RUNS_DIR",
        "QUAQUA_ROOT",
        "QUAQUA_HOME",
        "HOME",
        "QUAQUA_OWNER",
        "QUAQUA_LAYOUT_SHADOW",
        "QUAQUA_TEAM_TOML",
    ]);
    std::env::set_var("QUAQUA_TEST", "1");
    std::env::set_var("QUAQUA_ROOT", &temp_root);
    std::env::set_var("QUAQUA_HOME", &temp_root);
    std::env::set_var("HOME", &temp_root);
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_OWNER", "cos");
    std::env::remove_var("QUAQUA_LAYOUT_SHADOW");

    // Layout with cos -> chair (class = "lead") -> peer
    let team_toml_path = temp_root.join("team.toml");
    let team_toml_content = r#"
schema_version = 3
[strategy]
default_kind = "sude"
lead_kind = "sude"

[kinds.sude]
harness = "omp"
model = "claude-3-7-sonnet"
effort = "high"

[layout]
use = "chair-layout"

[layouts.chair-layout]
name = "chair-layout"
depth = 2

[layouts.chair-layout.roles.cos]
class = "cos"
can_spawn = ["chair"]
can_prompt = ["chair"]
max_children = 2

[layouts.chair-layout.roles.chair]
class = "lead"
can_spawn = ["peer"]
can_prompt = ["peer"]
reports_to = "cos"
naming = "slug-lead"

[layouts.chair-layout.roles.peer]
class = "peer"
can_spawn = []
can_prompt = []
reports_to = "owner"
"#;
    fs::write(&team_toml_path, team_toml_content).unwrap();
    std::env::set_var("QUAQUA_TEAM_TOML", &team_toml_path);

    fs::create_dir_all(temp_root.join("rules/_all")).unwrap();
    fs::create_dir_all(temp_root.join("projects")).unwrap();
    fs::write(
        temp_root.join("projects/INDEX.md"),
        "| slug | repo | owner | status | one-line goal |\n|------|------|-------|--------|---------------|\n| council | ~/Lab/council | cos | active | goal |\n",
    )
    .unwrap();
    fs::write(temp_root.join("projects/council.md"), "# council card\n").unwrap();

    let git = DummyGit;
    let herdr = DummyHerdr::new();

    let res = spawn_lead(&["council".to_string()], &git, &herdr);
    assert!(
        res.is_ok(),
        "lead spawn must succeed when lead class maps to chair: {:?}",
        res.err()
    );

    let snapshot_file = runs_dir.join("council.layout.toml");
    assert!(
        snapshot_file.exists(),
        "successful lead spawn must write layout snapshot"
    );
    assert_eq!(
        read_layout_role(&runs_dir, "council-lead"),
        Some("chair".to_string()),
        "recorded layout role for council-lead must be 'chair'"
    );
}

/// Proving Test 3: Prompt gate must use the effective layout (frozen snapshot of worker's Lead chain).
#[test]
fn prompt_gate_uses_effective_layout() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp_root = hermetic_temp_dir("prompt-effective");
    let runs_dir = temp_root.join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let _env = EnvGuard::new(&[
        "QUAQUA_TEST",
        "QUAQUA_RUNS_DIR",
        "QUAQUA_ROOT",
        "QUAQUA_HOME",
        "HOME",
        "QUAQUA_OWNER",
        "QUAQUA_LAYOUT_SHADOW",
        "QUAQUA_TEAM_TOML",
    ]);
    std::env::set_var("QUAQUA_TEST", "1");
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_OWNER", "cos");
    std::env::remove_var("QUAQUA_LAYOUT_SHADOW");

    // Worker owned by myproj-lead
    let worker_name = "sude-task1";
    fs::write(
        runs_dir.join(format!("{worker_name}.owner")),
        "myproj-lead\n",
    )
    .unwrap();
    fs::write(runs_dir.join("myproj-lead.owner"), "cos\n").unwrap();
    fs::write(
        runs_dir.join(format!("{worker_name}.layout_role")),
        "peer\n",
    )
    .unwrap();

    // Freeze snapshot for myproj with layout A where cos CAN prompt peer
    let snapshot_content = r#"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
source = "team.toml@0"

[layout]
name = "snapshot-layout"
depth = 2

[layout.roles.cos]
class = "cos"
can_spawn = ["lead", "peer"]
can_prompt = ["lead", "peer"]
max_children = 4

[layout.roles.lead]
class = "lead"
can_spawn = ["peer"]
can_prompt = ["peer"]
reports_to = "cos"

[layout.roles.peer]
class = "peer"
can_spawn = []
can_prompt = []
reports_to = "owner"
"#;
    fs::write(runs_dir.join("myproj.layout.toml"), snapshot_content).unwrap();

    // Live team.toml has layout B where cos CANNOT prompt peer (can_prompt = [])
    let live_toml = r#"
schema_version = 3
[strategy]
default_kind = "sude"

[layout]
use = "live-layout"

[layouts.live-layout]
name = "live-layout"
depth = 2

[layouts.live-layout.roles.cos]
class = "cos"
can_spawn = ["lead", "peer"]
can_prompt = []
max_children = 4

[layouts.live-layout.roles.lead]
class = "lead"
can_spawn = ["peer"]
can_prompt = []
reports_to = "cos"

[layouts.live-layout.roles.peer]
class = "peer"
can_spawn = []
can_prompt = []
reports_to = "owner"
"#;
    let live_cfg: Config = toml::from_str(live_toml).unwrap();

    let herdr = DummyHerdr::new();
    let dec = gate(
        None,
        Some(&herdr),
        Verb::Prompt,
        "peer",
        Some(worker_name),
        SpawnOrigin::Cli,
        Some("myproj-lead"),
        None,
        &runs_dir,
        Some(&live_cfg),
    );

    assert!(
        dec.is_ok(),
        "prompt gate must use frozen snapshot layout where prompt is allowed: {:?}",
        dec.err()
    );
}

/// Proving Test 4: A spawn refused or failing after the layout gate must leave no .layout_role marker.
#[test]
fn spawn_refused_leaves_no_layout_role() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp_root = hermetic_temp_dir("refused-no-role");
    let runs_dir = temp_root.join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let _env = EnvGuard::new(&[
        "QUAQUA_TEST",
        "QUAQUA_RUNS_DIR",
        "QUAQUA_ROOT",
        "QUAQUA_HOME",
        "HOME",
        "QUAQUA_OWNER",
        "QUAQUA_LAYOUT_SHADOW",
        "QUAQUA_TEAM_TOML",
    ]);
    std::env::set_var("QUAQUA_TEST", "1");
    std::env::set_var("QUAQUA_ROOT", &temp_root);
    std::env::set_var("QUAQUA_HOME", &temp_root);
    std::env::set_var("HOME", &temp_root);
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_OWNER", "cos");
    std::env::remove_var("QUAQUA_LAYOUT_SHADOW");

    // Standard team config
    let team_toml_path = temp_root.join("team.toml");
    let team_toml_content = r#"
schema_version = 3
[strategy]
default_kind = "sude"
lead_kind = "sude"

[kinds.sude]
harness = "omp"
model = "claude-3-7-sonnet"
effort = "high"
"#;
    fs::write(&team_toml_path, team_toml_content).unwrap();
    std::env::set_var("QUAQUA_TEAM_TOML", &team_toml_path);

    fs::create_dir_all(temp_root.join("rules/_all")).unwrap();
    fs::create_dir_all(temp_root.join("projects")).unwrap();
    fs::write(
        temp_root.join("projects/INDEX.md"),
        "| slug | repo | owner | status | one-line goal |\n|------|------|-------|--------|---------------|\n| mylead | ~/Lab/mylead | cos | active | goal |\n",
    )
    .unwrap();
    fs::write(temp_root.join("projects/mylead.md"), "# mylead card\n").unwrap();

    // Create a herdr dummy that fails at workspace create (after layout gate succeeded)
    let git = DummyGit;
    let mut herdr = DummyHerdr::new();
    herdr.fail_workspace = true;

    let res = spawn_lead(&["mylead".to_string()], &git, &herdr);
    assert!(
        res.is_err(),
        "lead spawn must fail when workspace create fails"
    );

    let role_file = runs_dir.join("mylead-lead.layout_role");
    assert!(
        !role_file.exists(),
        "failed spawn must leave no runs/<name>.layout_role marker on disk"
    );
}

/// Verifies that a successful worker spawn writes .layout_role and worker cleanup deletes it.
#[test]
fn worker_spawn_and_cleanup_lifecycle() {
    use quaqua::worker::cleanup::cleanup_worker_in;
    use quaqua::worker::spawn::{spawn_worker, SpawnOptions};

    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp_root = hermetic_temp_dir("worker-lifecycle");
    let runs_dir = temp_root.join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let repo_dir = temp_root.join("testrepo");
    fs::create_dir_all(&repo_dir).unwrap();

    let _env = EnvGuard::new(&[
        "QUAQUA_TEST",
        "QUAQUA_RUNS_DIR",
        "QUAQUA_ROOT",
        "QUAQUA_HOME",
        "HOME",
        "QUAQUA_OWNER",
        "QUAQUA_LAYOUT_SHADOW",
        "QUAQUA_TEAM_TOML",
        "GRID_SKIP",
    ]);
    std::env::set_var("QUAQUA_TEST", "1");
    std::env::set_var("GRID_SKIP", "1");
    std::env::set_var("QUAQUA_ROOT", &temp_root);
    std::env::set_var("QUAQUA_HOME", &temp_root);
    std::env::set_var("HOME", &temp_root);
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_OWNER", "cos");
    std::env::remove_var("QUAQUA_LAYOUT_SHADOW");

    let team_toml_path = temp_root.join("team.toml");
    let team_toml_content = r#"
schema_version = 3
[strategy]
default_kind = "sude"
lead_kind = "sude"

[kinds.sude]
harness = "omp"
model = "claude-3-7-sonnet"
effort = "high"
"#;
    fs::write(&team_toml_path, team_toml_content).unwrap();
    std::env::set_var("QUAQUA_TEAM_TOML", &team_toml_path);
    let cfg: Config = toml::from_str(team_toml_content).unwrap();

    fs::create_dir_all(temp_root.join("rules/_all")).unwrap();
    fs::create_dir_all(temp_root.join("projects")).unwrap();
    fs::write(
        temp_root.join("projects/INDEX.md"),
        "| slug | repo | owner | status | one-line goal |\n|------|------|-------|--------|---------------|\n| testrepo | ~/testrepo | cos | active | goal |\n",
    )
    .unwrap();

    let git = DummyGit;
    let herdr = DummyHerdr::new();

    let opts = SpawnOptions {
        name: "sude-task1".to_string(),
        kind: Some("sude".to_string()),
        repo_path: repo_dir.clone(),
        base: None,
        review: false,
        purpose: None,
        approve_kinds: Vec::new(),
        solo: None,
    };

    let res = spawn_worker(opts, &cfg, &git, &herdr, SpawnOrigin::Cli);
    assert!(res.is_ok(), "worker spawn must succeed: {:?}", res.err());

    let role_file = runs_dir.join("sude-task1.layout_role");
    assert!(
        role_file.is_file(),
        "runs/<name>.layout_role must exist after successful worker spawn"
    );
    assert_eq!(
        read_layout_role(&runs_dir, "sude-task1"),
        Some("peer".to_string())
    );

    // Now cleanup worker
    let clean_res = cleanup_worker_in("sude-task1", &repo_dir, &git, &herdr, Some(&runs_dir));
    assert!(
        clean_res.is_ok(),
        "worker cleanup must succeed: {:?}",
        clean_res.err()
    );

    assert!(
        !role_file.exists(),
        "runs/<name>.layout_role must be removed by worker cleanup"
    );
}

/// Verifies naming enforcement: KindTask role rejects names not conforming to {kind}-{task}.
#[test]
fn naming_wire_enforcement() {
    use quaqua::worker::spawn::{spawn_worker, SpawnOptions};

    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp_root = hermetic_temp_dir("naming-wire");
    let runs_dir = temp_root.join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    let repo_dir = temp_root.join("testrepo");
    fs::create_dir_all(&repo_dir).unwrap();

    let _env = EnvGuard::new(&[
        "QUAQUA_TEST",
        "QUAQUA_RUNS_DIR",
        "QUAQUA_ROOT",
        "QUAQUA_HOME",
        "HOME",
        "QUAQUA_OWNER",
        "QUAQUA_LAYOUT_SHADOW",
        "QUAQUA_TEAM_TOML",
        "GRID_SKIP",
    ]);
    std::env::set_var("QUAQUA_TEST", "1");
    std::env::set_var("GRID_SKIP", "1");
    std::env::set_var("QUAQUA_ROOT", &temp_root);
    std::env::set_var("QUAQUA_HOME", &temp_root);
    std::env::set_var("HOME", &temp_root);
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_OWNER", "cos");
    std::env::remove_var("QUAQUA_LAYOUT_SHADOW");

    let team_toml_path = temp_root.join("team.toml");
    let team_toml_content = r#"
schema_version = 3
[strategy]
default_kind = "sude"
lead_kind = "sude"

[kinds.sude]
harness = "omp"
model = "claude-3-7-sonnet"
effort = "high"
"#;
    fs::write(&team_toml_path, team_toml_content).unwrap();
    std::env::set_var("QUAQUA_TEAM_TOML", &team_toml_path);
    let cfg: Config = toml::from_str(team_toml_content).unwrap();

    let git = DummyGit;
    let herdr = DummyHerdr::new();

    // Name ending with "-lead" is rejected for KindTask role
    let opts_bad_lead = SpawnOptions {
        name: "sude-worker-lead".to_string(),
        kind: Some("sude".to_string()),
        repo_path: repo_dir.clone(),
        base: None,
        review: false,
        purpose: None,
        approve_kinds: Vec::new(),
        solo: None,
    };
    let res = spawn_worker(opts_bad_lead, &cfg, &git, &herdr, SpawnOrigin::Cli);
    assert!(
        res.is_err(),
        "KindTask role must reject name ending with -lead"
    );

    // Name not starting with {kind}- is rejected
    let opts_bad_prefix = SpawnOptions {
        name: "wrongprefix-task".to_string(),
        kind: Some("sude".to_string()),
        repo_path: repo_dir,
        base: None,
        review: false,
        purpose: None,
        approve_kinds: Vec::new(),
        solo: None,
    };
    let res = spawn_worker(opts_bad_prefix, &cfg, &git, &herdr, SpawnOrigin::Cli);
    assert!(
        res.is_err(),
        "KindTask role must reject name with wrong prefix"
    );
}
