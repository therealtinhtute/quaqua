use quaqua::cap::{count_owner_slots, is_task_worker_name, matches_pipeline_reviewer};
use quaqua::config;
use quaqua::git::GitOps;
use quaqua::herdr::HerdrOps;
use quaqua::worker::cleanup::{resolve_default_runs_dir, resolve_home_quaqua_runs};
use quaqua::worker::prompt::{
    brief_budget_sec, brief_headers_ok, fallback_header_line, prepend_header,
    validate_env_budget_val,
};
use quaqua::worker::spawn::{
    build_launch_cmd, fallback_kind, parse_spawn_args, resolve_launch, start_failure_reason,
    Launch,
};
use quaqua::workspace::resolve_workspace_root;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn test_task_worker_name_validation() {
    assert!(is_task_worker_name("tieude-task1"));
    assert!(is_task_worker_name("sude-hardimpl"));
    assert!(is_task_worker_name("suhuynh-review2"));
    assert!(is_task_worker_name("ni-quickfix"));

    // Leads do not count as task workers
    assert!(!is_task_worker_name("coucou-lead"));
    assert!(!is_task_worker_name("sude-lead"));
    assert!(!is_task_worker_name("quaqua-rs-lead"));

    // Unknown kinds
    assert!(!is_task_worker_name("random-worker"));
    assert!(!is_task_worker_name("unknown-task"));
}

#[test]
fn test_brief_headers() {
    let valid = "Goal: accomplish task\nHard constraints: none\nCurrent design (open to challenge): plan";
    assert!(brief_headers_ok(valid));

    let valid_alt = "Goal: accomplish task\nHard constraints: none\nCurrent design: direct";
    assert!(brief_headers_ok(valid_alt));

    let invalid_missing_goal = "Hard constraints: none\nCurrent design: direct";
    assert!(!brief_headers_ok(invalid_missing_goal));

    let invalid_missing_design = "Goal: accomplish task\nHard constraints: none";
    assert!(!brief_headers_ok(invalid_missing_design));
}

#[test]
fn test_brief_budget_regex_matching() {
    // Happy paths
    assert_eq!(brief_budget_sec("Time budget: 25 min"), Some(1500));
    assert_eq!(brief_budget_sec("Time budget: 10m"), Some(600));
    assert_eq!(brief_budget_sec("Time budget: 45 minutes"), Some(2700));
    assert_eq!(brief_budget_sec("Time budget: 15 mins"), Some(900));
    assert_eq!(brief_budget_sec("time budget: 20 min."), Some(1200));

    // Finding 3 edge cases: must NOT match if unit is hours or seconds, even if line contains 'm'
    assert_eq!(brief_budget_sec("Time budget: 2 hours max"), None);
    assert_eq!(brief_budget_sec("Time budget: 1800s (team max)"), None);
    assert_eq!(brief_budget_sec("Time budget: 3 days"), None);
}

#[test]
fn test_brief_budget_non_ascii_n1() {
    // N1: multi-byte UTF-8 after 'Time budget:' must not panic and must match bash
    assert_eq!(brief_budget_sec("Time budget: ≈ 20 min"), Some(1200));
    assert_eq!(brief_budget_sec("Time budget: —"), None);
    assert_eq!(brief_budget_sec("Time budget: — 45 mins"), Some(2700));
    assert_eq!(brief_budget_sec("Time budget: ≈ 10m"), Some(600));
}

#[test]
fn test_env_budget_validation_n3_n5() {
    // N3: empty string = unset (valid, not err)
    // N5: no process-global set_var mutations
    assert!(validate_env_budget_val(Some("")).is_ok());
    assert!(validate_env_budget_val(None).is_ok());
    assert!(validate_env_budget_val(Some("120")).is_ok());

    // Invalid values
    assert!(validate_env_budget_val(Some("abc")).is_err());
    assert!(validate_env_budget_val(Some("12.5")).is_err());
    assert!(validate_env_budget_val(Some("-5")).is_err());
}

#[test]
fn test_pipeline_reviewer_prefix_matching_n2() {
    // N2: suhuynh-prep-pr1 has '-prep' before '-pr1'; must match on second occurrence
    assert!(matches_pipeline_reviewer("suhuynh-prep-pr1", "test"));
    assert!(matches_pipeline_reviewer("suhuynh-pr1", "test"));
    assert!(matches_pipeline_reviewer("suhuynh-pr24", "test"));
    assert!(matches_pipeline_reviewer("suhuynh-testr1", "test"));
    assert!(matches_pipeline_reviewer("suhuynh-testr3", "test"));

    // N2: empty task matches *-r[0-9]*
    assert!(matches_pipeline_reviewer("suhuynh-r3r2", ""));
    assert!(matches_pipeline_reviewer("ni-r1", ""));

    // Non-reviewer workers
    assert!(!matches_pipeline_reviewer("tieude-prompt", "test"));
    assert!(!matches_pipeline_reviewer("tieude-process", "test"));
    assert!(!matches_pipeline_reviewer("sude-testrun", "test"));
    assert!(!matches_pipeline_reviewer("sude-testreview", "test"));
}

struct MockHerdr {
    agents_json: String,
}

impl HerdrOps for MockHerdr {
    fn run(&self, _args: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_list(&self) -> std::io::Result<String> {
        Ok(self.agents_json.clone())
    }
    fn agent_get(&self, _name: &str) -> std::io::Result<(bool, String)> {
        unimplemented!()
    }
    fn agent_read(&self, _name: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        unimplemented!()
    }
    fn agent_start(&self, _name: &str, _kind: &str, _pane: &str, _timeout_ms: u32, _extra_args: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_split(&self, _cwd: &str, _owner: &str, _envs: &[(&str, &str)]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_close(&self, _pane: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_report_metadata(&self, _pane: &str, _tokens: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_wait_output(&self, _pane: &str, _pattern: &str, _timeout_ms: u32) -> bool {
        unimplemented!()
    }
    fn pane_read(&self, _pane: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        unimplemented!()
    }
    fn pane_send_keys(&self, _pane: &str, _key: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_send_keys(&self, _name: &str, _key: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_wait_idle(&self, _name: &str, _timeout_ms: u32) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_prompt_nowait(&self, _name: &str, _prompt: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_prompt_wait_background(&self, _name: &str, _prompt: &str, _timeout_ms: u32, _log_file: std::fs::File) -> std::io::Result<Child> {
        unimplemented!()
    }
}

struct SpyHerdr {
    split_called: AtomicBool,
}

impl HerdrOps for SpyHerdr {
    fn run(&self, _args: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_list(&self) -> std::io::Result<String> {
        unimplemented!()
    }
    fn agent_get(&self, _name: &str) -> std::io::Result<(bool, String)> {
        unimplemented!()
    }
    fn agent_read(&self, _name: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        unimplemented!()
    }
    fn agent_start(&self, _name: &str, _kind: &str, _pane: &str, _timeout_ms: u32, _extra_args: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_split(&self, _cwd: &str, _owner: &str, _envs: &[(&str, &str)]) -> std::io::Result<Output> {
        self.split_called.store(true, Ordering::SeqCst);
        unimplemented!()
    }
    fn pane_close(&self, _pane: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_report_metadata(&self, _pane: &str, _tokens: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_wait_output(&self, _pane: &str, _pattern: &str, _timeout_ms: u32) -> bool {
        unimplemented!()
    }
    fn pane_read(&self, _pane: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        unimplemented!()
    }
    fn pane_send_keys(&self, _pane: &str, _key: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_send_keys(&self, _name: &str, _key: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_wait_idle(&self, _name: &str, _timeout_ms: u32) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_prompt_nowait(&self, _name: &str, _prompt: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_prompt_wait_background(&self, _name: &str, _prompt: &str, _timeout_ms: u32, _log_file: std::fs::File) -> std::io::Result<Child> {
        unimplemented!()
    }
}

struct DummyGit;

impl GitOps for DummyGit {
    fn run_in(&self, _dir: &Path, _args: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn run(&self, _args: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn rev_parse_commit(&self, _repo: &Path, _base: &str) -> bool {
        true
    }
    fn worktree_list_porcelain(&self, _repo: &Path) -> std::io::Result<Vec<String>> {
        Ok(vec![])
    }
    fn worktree_add(&self, _repo: &Path, _branch: &str, _wt: &Path, _base: Option<&str>) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn branch_exists(&self, _repo: &Path, _branch: &str) -> bool {
        false
    }
    fn worktree_add_existing(&self, _repo: &Path, _branch: &str, _wt: &Path) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn worktree_remove(&self, _repo: &Path, _wt: &Path) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn remote_list(&self, _repo: &Path) -> std::io::Result<Vec<String>> {
        Ok(vec![])
    }
    fn is_core_bare(&self, _repo: &Path) -> bool {
        false
    }
    fn has_core_worktree(&self, _repo: &Path) -> bool {
        false
    }
    fn enable_worktree_config(&self, _repo: &Path) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn set_worktree_pushurl_blocked(&self, _wt: &Path, _remote: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn is_git_dir(&self, _cwd: &Path) -> bool {
        true
    }
    fn diff_and_status(&self, _cwd: &Path) -> (String, String) {
        ("dh".to_string(), "clean".to_string())
    }
}

#[test]
fn test_unknown_harness_rejected_before_split_n4() {
    let spy = SpyHerdr {
        split_called: AtomicBool::new(false),
    };
    let dummy_git = DummyGit;

    let mut cfg = config::Config::default();
    cfg.kinds.insert("badkind".to_string(), config::Kind {
        harness: "unsupported_harness".to_string(),
        model: "m".to_string(),
        effort: None,
        fallback: None,
        can_review: false,
    });

    let opts = parse_spawn_args(&[
        "badkind-task1".to_string(),
        "badkind".to_string(),
        "/repo".to_string(),
    ]).unwrap();

    let res = quaqua::worker::spawn::spawn_worker(opts, &cfg, &dummy_git, &spy);
    assert!(res.is_err());
    // N4: pane_split must NEVER have been called!
    assert!(!spy.split_called.load(Ordering::SeqCst));
}

#[test]
fn test_missing_effort_rejected_before_split() {
    let spy = SpyHerdr {
        split_called: AtomicBool::new(false),
    };
    let dummy_git = DummyGit;

    let mut cfg = config::Config::default();
    cfg.kinds.insert("tieude".to_string(), config::Kind {
        harness: "opencode".to_string(),
        model: "m".to_string(),
        effort: None,
        fallback: None,
        can_review: false,
    });

    let opts = parse_spawn_args(&[
        "tieude-task1".to_string(),
        "tieude".to_string(),
        "/repo".to_string(),
    ]).unwrap();

    let res = quaqua::worker::spawn::spawn_worker(opts, &cfg, &dummy_git, &spy);
    assert!(res.is_err());
    assert!(!spy.split_called.load(Ordering::SeqCst));
}

#[test]
fn test_unsupported_effort_rejected_before_split() {
    let spy = SpyHerdr {
        split_called: AtomicBool::new(false),
    };
    let dummy_git = DummyGit;

    let mut cfg = config::Config::default();
    cfg.kinds.insert("tieude".to_string(), config::Kind {
        harness: "opencode".to_string(),
        model: "m".to_string(),
        effort: Some("banana".to_string()),
        fallback: None,
        can_review: false,
    });

    let opts = parse_spawn_args(&[
        "tieude-task1".to_string(),
        "tieude".to_string(),
        "/repo".to_string(),
    ]).unwrap();

    let res = quaqua::worker::spawn::spawn_worker(opts, &cfg, &dummy_git, &spy);
    assert!(res.is_err());
    assert!(!spy.split_called.load(Ordering::SeqCst));
}

#[test]
fn test_cap_pipeline_slot_reservation_mock() {
    let temp_dir = std::env::temp_dir().join(format!("quaqua-cap-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);

    // Write owner files: 1 live worker tieude-prompt (not a reviewer!)
    std::fs::write(temp_dir.join("tieude-prompt.owner"), "cos\n").unwrap();

    // Write pipeline state: finished=0, builder dead, but builder name in live_workers
    std::fs::write(
        temp_dir.join("pipeline-feature.state"),
        "task=feature\nbuilder=sude\nlead=cos\npipeline_pid=99999999\nfinished=0\n",
    )
    .unwrap();

    // Agents list has sude-feature (alive builder) + tieude-prompt
    let mock = MockHerdr {
        agents_json: r#"{"result":{"agents":[{"name":"sude-feature","agent_status":"working"},{"name":"tieude-prompt","agent_status":"working"}]}}"#.to_string(),
    };
    std::fs::write(temp_dir.join("sude-feature.owner"), "cos\n").unwrap();
    std::fs::write(temp_dir.join("sude-feature.pipeline"), "feature\n").unwrap();

    // Total slots: 2 live workers (sude-feature + tieude-prompt)
    // Pipeline 'feature' has 1 active worker (sude-feature), so it reserves 2 - 1 = 1 slot.
    // Total should be 2 live + 1 reserved = 3 slots!
    // Under the old bug, tieude-prompt was counted as a pipeline reviewer for 'feature', reserving 0 slots (giving 2).
    let count = count_owner_slots(&temp_dir, "cos", None, &mock);
    assert_eq!(count, 3);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cap_pipeline_rust_state_without_task_line() {
    // A state written by PipelineState::save has no `task=` line for Rust pipelines.
    // The same 2-slot pipeline must not also count its live builder as a standalone worker.
    let temp_dir = std::env::temp_dir().join(format!("quaqua-cap-notask-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let owner = "quaqua-intake-lead";

    std::fs::write(temp_dir.join("sude-blockedfix.owner"), format!("{owner}\n")).unwrap();
    std::fs::write(temp_dir.join("sude-blockedfix.pipeline"), "blockedfix\n").unwrap();
    std::fs::write(
        temp_dir.join("pipeline-blockedfix.state"),
        format!("repo=/repo\nbrief=/b.md\nbuilder=sude\nreviewer=suhuynh\nreview=auto\nrounds=2\nbase=main\nwt=/repo/.wt/sude-blockedfix\nphase=build\nround=0\nfinished=0\nlead=quaqua-intake-lead\nlead_pane=w1:p1\npipeline_pid={}\n", std::process::id()),
    )
    .unwrap();

    let mock = MockHerdr {
        agents_json: r#"{"result":{"agents":[{"name":"sude-blockedfix","agent_status":"working"}]}}"#.to_string(),
    };

    // 1 live builder + (2 - 1 already counted) reserved = 2, not 1 + 2 = 3.
    let count = count_owner_slots(&temp_dir, owner, None, &mock);
    assert_eq!(count, 2);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cap_pipeline_rust_state_review_round() {
    // Builder + fresh reviewer of the same pipeline are its 2 slots, not 2 live + 2 reserved.
    let temp_dir = std::env::temp_dir().join(format!("quaqua-cap-review-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let owner = "quaqua-intake-lead";

    std::fs::write(temp_dir.join("sude-blockedfix.owner"), format!("{owner}\n")).unwrap();
    std::fs::write(temp_dir.join("sude-blockedfix.pipeline"), "blockedfix\n").unwrap();
    std::fs::write(temp_dir.join("suhuynh-blockedfixr1.owner"), format!("{owner}\n")).unwrap();
    std::fs::write(temp_dir.join("suhuynh-blockedfixr1.pipeline"), "blockedfix\n").unwrap();
    std::fs::write(
        temp_dir.join("pipeline-blockedfix.state"),
        format!("repo=/repo\nbuilder=sude\nreviewer=suhuynh\nphase=review\nround=1\nfinished=0\nlead=quaqua-intake-lead\npipeline_pid={}\n", std::process::id()),
    )
    .unwrap();

    let mock = MockHerdr {
        agents_json: r#"{"result":{"agents":[{"name":"sude-blockedfix","agent_status":"working"},{"name":"suhuynh-blockedfixr1","agent_status":"working"}]}}"#.to_string(),
    };

    let count = count_owner_slots(&temp_dir, owner, None, &mock);
    assert_eq!(count, 2);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cap_pipeline_state_task_line_wins_over_filename() {
    // Legacy/shell states carry `task=`; an explicit line is the task, not the file name.
    let temp_dir = std::env::temp_dir().join(format!("quaqua-cap-taskline-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let owner = "cos";

    std::fs::write(temp_dir.join("sude-other.owner"), "cos\n").unwrap();
    std::fs::write(temp_dir.join("sude-other.pipeline"), "other\n").unwrap();
    std::fs::write(
        temp_dir.join("pipeline-blockedfix.state"),
        format!("task=other\nbuilder=sude\nlead=cos\nfinished=0\npipeline_pid={}\n", std::process::id()),
    )
    .unwrap();

    let mock = MockHerdr {
        agents_json: r#"{"result":{"agents":[{"name":"sude-other","agent_status":"working"}]}}"#.to_string(),
    };
    assert_eq!(count_owner_slots(&temp_dir, owner, None, &mock), 2);

    std::fs::write(temp_dir.join("sude-blockedfix.owner"), "cos\n").unwrap();
    std::fs::write(temp_dir.join("sude-blockedfix.pipeline"), "blockedfix\n").unwrap();
    std::fs::write(
        temp_dir.join("pipeline-blockedfix.state"),
        format!("task=other\nbuilder=sude\nlead=cos\nfinished=0\npipeline_pid={}\n", std::process::id()),
    )
    .unwrap();
    let mock = MockHerdr {
        agents_json: r#"{"result":{"agents":[{"name":"sude-blockedfix","agent_status":"working"}]}}"#.to_string(),
    };
    // Not part of pipeline 'other': 1 live + full 2 reservation.
    assert_eq!(count_owner_slots(&temp_dir, owner, None, &mock), 3);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
#[test]
fn test_cap_pipeline_double_count_repro() {
    // Repro for Cause (c): pipeline gridown2 has a live builder tieude-gridown2
    // (e.g. after a fallback or non-default builder) without .pipeline sidecar.
    // Must count as 2 slots (the pipeline once), not 3 (builder 1 + reserve 2).
    let temp_dir = std::env::temp_dir().join(format!("quaqua-cap-doublecount-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let owner = "quaqua-rs-lead";

    std::fs::write(temp_dir.join("tieude-gridown2.owner"), format!("{owner}\n")).unwrap();
    // No tieude-gridown2.pipeline file written
    std::fs::write(
        temp_dir.join("pipeline-gridown2.state"),
        format!("repo=/repo\nbuilder=sude\nphase=build\nround=1\nfinished=0\nlead=quaqua-rs-lead\npipeline_pid={}\n", std::process::id()),
    )
    .unwrap();

    let mock = MockHerdr {
        agents_json: r#"{"result":{"agents":[{"name":"tieude-gridown2","agent_status":"working"}]}}"#.to_string(),
    };

    let count = count_owner_slots(&temp_dir, owner, None, &mock);
    assert_eq!(count, 2, "a pipeline must count once (2 slots), never double-count to 3");

    let _ = std::fs::remove_dir_all(&temp_dir);
}
#[test]
fn test_cost_budget_parser_and_reader() {
    use quaqua::worker::prompt::{brief_cost_budget, read_worker_cost};

    let b1 = "Goal: test\nHard constraints: none\nCurrent design: none\nCost budget: $0.001\nTime budget: 5 min";
    assert_eq!(brief_cost_budget(b1), Some(0.001));

    let b2 = "Goal: test\nCost budget: $2.50\n";
    assert_eq!(brief_cost_budget(b2), Some(2.5));

    let b3 = "Goal: test\nCost budget: 0.75\n";
    assert_eq!(brief_cost_budget(b3), Some(0.75));

    let b4 = "Goal: test\nNo cost line";
    assert_eq!(brief_cost_budget(b4), None);

    // Cost reader via PROMPT_WORKER_COST_FILE fixture
    let tmp = std::env::temp_dir().join(format!("cost-fixture-{}", std::process::id()));
    std::fs::write(&tmp, "0.4242\n").unwrap();
    std::env::set_var("PROMPT_WORKER_COST_FILE", &tmp);
    let cost = read_worker_cost("tieude-test", "ses_123", "/tmp", "opencode");
    assert_eq!(cost, Some(0.4242));
    std::env::remove_var("PROMPT_WORKER_COST_FILE");
    let _ = std::fs::remove_file(&tmp);

    // Non-opencode without fixture returns None
    assert_eq!(read_worker_cost("sude-test", "ses_123", "/tmp", "omp"), None);
}

#[test]
fn test_runs_dir_resolution_n5() {
    // N5: no process-global set_var mutations
    let test_home = Path::new("target/test_home");
    assert_eq!(resolve_home_quaqua_runs(Some(test_home)), test_home.join("quaqua/runs"));
    assert_eq!(resolve_default_runs_dir(None, Some(test_home)), test_home.join("quaqua/runs"));

    let override_dir = Path::new("target/override_runs");
    assert_eq!(resolve_default_runs_dir(Some(override_dir), Some(test_home)), override_dir.to_path_buf());
    assert_eq!(resolve_home_quaqua_runs(Some(test_home)), test_home.join("quaqua/runs"));

    // workspace_root: QUAQUA_HOME wins, else $HOME/quaqua
    let quaqua_home = Path::new("target/quaqua_home");
    assert_eq!(resolve_workspace_root(Some(quaqua_home), Some(test_home)), quaqua_home.to_path_buf());
    assert_eq!(resolve_workspace_root(None, Some(test_home)), test_home.join("quaqua"));
}

#[test]
fn test_team_toml_harness_routing() {
    let team_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../team.toml");
    let cfg = config::load(&team_path).expect("valid team.toml");

    // tieude is opencode at effort max
    assert_eq!(cfg.kinds["tieude"].harness, "opencode");
    assert_eq!(cfg.kinds["tieude"].model, "opencode-go/deepseek-v4.1-flash");
    assert_eq!(cfg.kinds["tieude"].effort.as_deref(), Some("max"));
    assert!(!cfg.kinds["tieude"].can_review);
    // detu is omp at effort high (anh iu 2026-10-16, 03ed0a7)
    assert_eq!(cfg.kinds["detu"].harness, "omp");
    assert_eq!(cfg.kinds["detu"].model, "google-antigravity/gemini-3.8-flash");
    assert_eq!(cfg.kinds["detu"].effort.as_deref(), Some("high"));
    assert!(!cfg.kinds["detu"].can_review);
    // fallback seeds: tieude->detu, detu->tieude, sude->tieude; claude kinds none
    assert_eq!(cfg.kinds["tieude"].fallback.as_deref(), Some("detu"));
    assert_eq!(cfg.kinds["detu"].fallback.as_deref(), Some("tieude"));
    assert_eq!(cfg.kinds["sude"].fallback.as_deref(), Some("tieude"));
    for k in ["ni", "suhuynh", "truonglao"] {
        assert_eq!(cfg.kinds[k].fallback, None, "{k} must have no fallback");
    }

    // sude is omp with antigravity gemini high
    assert_eq!(cfg.kinds["sude"].harness, "omp");
    assert_eq!(cfg.kinds["sude"].model, "google-antigravity/gemini-3.8-flash");
    assert_eq!(cfg.kinds["sude"].effort.as_deref(), Some("high"));
    assert!(!cfg.kinds["sude"].can_review);

    // ni is claude with can_review=true
    assert_eq!(cfg.kinds["ni"].harness, "claude");
    assert_eq!(cfg.kinds["ni"].model, "opus-5.5");
    assert!(cfg.kinds["ni"].can_review);

    // suhuynh is opencode with can_review=true (Claude out, anh iu 2026-10-17)
    assert_eq!(cfg.kinds["suhuynh"].harness, "opencode");
    assert_eq!(cfg.kinds["suhuynh"].model, "opencode-go/deepseek-v4.1-flash");
    assert_eq!(cfg.kinds["suhuynh"].effort.as_deref(), Some("max"));
    assert!(cfg.kinds["suhuynh"].can_review);

    // truonglao is claude with can_review=true; every kind is opencode/omp/claude
    assert_eq!(cfg.kinds["truonglao"].harness, "claude");
    assert!(cfg.kinds["truonglao"].can_review);
    assert_eq!(cfg.kinds.len(), 6, "only the six surviving kinds");
    assert!(cfg.kinds.values().all(|k| ["opencode", "omp", "claude"].contains(&k.harness.as_str())));

    // strategy.lead_kind defaults to tieude
    assert_eq!(cfg.strategy.lead_kind.as_deref(), Some("tieude"));
}

#[test]
fn test_lead_launch_uses_strategy_lead_kind_and_override() {
    let team_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../team.toml");
    let cfg = config::load(&team_path).expect("valid team.toml");

    // 1. Default lead launch uses strategy.lead_kind ("tieude" on opencode)
    let (kind, launch) = quaqua::lead::spawn::resolve_lead_launch(&cfg, None).expect("lead launch");
    assert_eq!(kind, "tieude");
    assert_eq!(launch.harness, "opencode");
    assert_eq!(launch.model, "opencode-go/deepseek-v4.1-flash");
    assert_eq!(launch.effort, "max");

    let cmd = build_launch_cmd(&launch, "/runs/test.rules.md", "/runs", "/repo", false).unwrap();
    assert_eq!(cmd.split_envs.len(), 1);
    assert_eq!(cmd.split_envs[0].0, "OPENCODE_CONFIG_CONTENT");
    assert!(cmd.split_envs[0].1.contains("opencode-go/deepseek-v4.1-flash#max"));
    assert_eq!(cmd.agent_args, vec!["--", "--standalone", "--auto"]);

    // 2. LEAD_KIND env override switches lead to detu (omp)
    let (kind_detu, launch_detu) = quaqua::lead::spawn::resolve_lead_launch(&cfg, Some("detu")).expect("detu lead");
    assert_eq!(kind_detu, "detu");
    assert_eq!(launch_detu.harness, "omp");
    assert_eq!(launch_detu.model, "google-antigravity/gemini-3.8-flash");
    assert_eq!(launch_detu.effort, "high");

    let cmd_detu = build_launch_cmd(&launch_detu, "/runs/test.rules.md", "/runs", "/repo", false).unwrap();
    assert!(cmd_detu.split_envs.is_empty());
    assert_eq!(cmd_detu.agent_args, vec![
        "--",
        "--model",
        "google-antigravity/gemini-3.8-flash",
        "--thinking",
        "high",
        "--auto-approve",
        "--append-system-prompt",
        "/runs/test.rules.md",
    ]);

    // 3. LEAD_KIND env override switches lead to ni (claude)
    let (kind_ni, launch_ni) = quaqua::lead::spawn::resolve_lead_launch(&cfg, Some("ni")).expect("ni lead");
    assert_eq!(kind_ni, "ni");
    assert_eq!(launch_ni.harness, "claude");
    assert_eq!(launch_ni.model, "opus-5.5");
    assert_eq!(launch_ni.effort, "low");

    let cmd_ni = build_launch_cmd(&launch_ni, "/runs/test.rules.md", "/runs", "/repo", false).unwrap();
    assert!(cmd_ni.split_envs.is_empty());
    assert_eq!(cmd_ni.agent_args, vec![
        "--",
        "--model",
        "claude-opus-5-5",
        "--effort",
        "low",
        "--append-system-prompt-file",
        "/runs/test.rules.md",
    ]);

    // 4. Unknown LEAD_KIND falls back to tieude
    let (kind_fallback, launch_fallback) = quaqua::lead::spawn::resolve_lead_launch(&cfg, Some("nonexistent_kind")).expect("fallback");
    assert_eq!(kind_fallback, "tieude");
    assert_eq!(launch_fallback.harness, "opencode");

    // 5. Missing lead_kind in Strategy defaults to tieude
    let mut cfg_no_lead = cfg.clone();
    cfg_no_lead.strategy.lead_kind = None;
    let (kind_default, launch_default) = quaqua::lead::spawn::resolve_lead_launch(&cfg_no_lead, None).expect("default");
    assert_eq!(kind_default, "tieude");
    assert_eq!(launch_default.harness, "opencode");
}

#[test]
fn test_spawn_args_parsing() {
    let args = vec![
        "--base".to_string(),
        "main".to_string(),
        "--review".to_string(),
        "suhuynh-rev1".to_string(),
        "suhuynh".to_string(),
        "/repo".to_string(),
    ];
    let opts = parse_spawn_args(&args).expect("valid args");
    assert_eq!(opts.base.as_deref(), Some("main"));
    assert!(opts.review);
    assert_eq!(opts.name, "suhuynh-rev1");
    assert_eq!(opts.kind, "suhuynh");

    // Trailing base without value
    let bad = vec![
        "suhuynh-rev1".to_string(),
        "suhuynh".to_string(),
        "/repo".to_string(),
        "--base".to_string(),
    ];
    assert!(parse_spawn_args(&bad).is_err());
}

#[test]
fn test_cli_cap_smoke() {
    let bin = env!("CARGO_BIN_EXE_quaqua");
    let out = Command::new(bin)
        .arg("cap")
        .output()
        .expect("runs quaqua cap");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.trim().parse::<i64>().is_ok());
}
#[test]
fn test_taskforce_dual_lens_cap_reservation() {
    struct MockHerdr;
    impl HerdrOps for MockHerdr {
        fn run(&self, _: &[&str]) -> std::io::Result<Output> {
            Ok(Output {
                status: std::os::unix::process::ExitStatusExt::from_raw(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
        fn agent_list(&self) -> std::io::Result<String> {
            Ok(r#"{"result":{"agents":[]}}"#.to_string())
        }
        fn agent_get(&self, _: &str) -> std::io::Result<(bool, String)> {
            Ok((false, String::new()))
        }
        fn agent_read(&self, _: &str, _: &str, _: u32) -> std::io::Result<String> {
            Ok(String::new())
        }
        fn agent_start(&self, _: &str, _: &str, _: &str, _: u32, _: &[&str]) -> std::io::Result<Output> {
            self.run(&[])
        }
        fn pane_split(&self, _: &str, _: &str, _: &[(&str, &str)]) -> std::io::Result<Output> {
            self.run(&[])
        }
        fn pane_close(&self, _: &str) -> std::io::Result<Output> {
            self.run(&[])
        }
        fn pane_report_metadata(&self, _: &str, _: &[&str]) -> std::io::Result<Output> {
            self.run(&[])
        }
        fn pane_wait_output(&self, _: &str, _: &str, _: u32) -> bool {
            true
        }
        fn pane_read(&self, _: &str, _: &str, _: u32) -> std::io::Result<String> {
            Ok(String::new())
        }
        fn pane_send_keys(&self, _: &str, _: &str) -> std::io::Result<Output> {
            self.run(&[])
        }
        fn agent_send_keys(&self, _: &str, _: &str) -> std::io::Result<Output> {
            self.run(&[])
        }
        fn agent_wait_idle(&self, _: &str, _: u32) -> std::io::Result<Output> {
            self.run(&[])
        }
        fn agent_prompt_nowait(&self, _: &str, _: &str) -> std::io::Result<Output> {
            self.run(&[])
        }
        fn agent_prompt_wait_background(&self, _: &str, _: &str, _: u32, _: std::fs::File) -> std::io::Result<Child> {
            Err(std::io::Error::new(std::io::ErrorKind::Other, "mock"))
        }
    }

    let tmp = std::env::temp_dir().join(format!("quaqua-cap-tf-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let mock = MockHerdr;

    // Zero workers, no state -> 0
    assert_eq!(count_owner_slots(&tmp, "cos", None, &mock), 0);

    // Active dual-lens state with current PID (alive) -> reserves 2 slots
    let state_file = tmp.join("taskforce-mytest.state");
    std::fs::write(
        &state_file,
        format!("task=mytest\nkind=dual-lens\nowner=cos\nfinished=0\ntaskforce_pid={}\n", std::process::id()),
    ).unwrap();

    assert_eq!(count_owner_slots(&tmp, "cos", None, &mock), 2);

    // Finished dual-lens -> 0 slots reserved
    std::fs::write(
        &state_file,
        format!("task=mytest\nkind=dual-lens\nowner=cos\nfinished=1\ntaskforce_pid={}\n", std::process::id()),
    ).unwrap();
    assert_eq!(count_owner_slots(&tmp, "cos", None, &mock), 0);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_pipeline_blockers_none_exemption() {
    use quaqua::pipeline::has_real_blockers;

    assert!(!has_real_blockers("## Blockers\n- none\n"));
    assert!(!has_real_blockers("## Blockers\nnone\n"));
    assert!(!has_real_blockers("## Blockers\n* none\n"));
    assert!(!has_real_blockers("## Blockers\n- None\n"));
    assert!(!has_real_blockers("## Blockers\n- None…\n"));
    assert!(!has_real_blockers("## Blockers\n- None.\n"));
    assert!(!has_real_blockers("## Blockers\n- None...\n"));
    assert!(!has_real_blockers("## Blockers\nNone…\n"));
    assert!(!has_real_blockers("## Blockers\nNone.\n"));
    assert!(!has_real_blockers("## Blockers\n* none…\n"));
    assert!(!has_real_blockers("## Blockers\n\n- none\n\n## Lessons\n- none\n"));
    assert!(!has_real_blockers("Just plan text\n\n## Lessons\n- none\n"));

    // Real blockers must return true
    assert!(has_real_blockers("## Blockers\n- cannot plan without the spec\n"));
    assert!(has_real_blockers("## Blockers\n- implementation incomplete\n"));
    assert!(has_real_blockers("## Blockers\n- none\n- also a real blocker\n"));
    assert!(has_real_blockers("## Blockers\n- None of the tests pass\n"));
}

#[test]
fn test_pipeline_blocked_keeps_finished_zero() {
    use quaqua::pipeline::pipeline_finished;

    // A blocked stop stays resumable: finished must not be set.
    assert!(!pipeline_finished("blocked", 0));
    assert!(!pipeline_finished("blocked", 3));
    // Real outcomes stay final.
    assert!(pipeline_finished("clean", 0));
    assert!(pipeline_finished("disputed", 0));
    // Non-zero exits never finish the state, whatever the status says.
    assert!(!pipeline_finished("plan-ready", 5));
    assert!(!pipeline_finished("clean", 4));
}

#[test]
fn test_strategy_default_reviewer_selection() {
    use quaqua::pipeline::select_default_reviewer;

    let toml_str = r#"
schema_version = 2
[strategy]
default_kind = "tieude"
default_reviewer = ["suhuynh", "ni"]
builder_reviewer_differ = "harness"
[kinds.sude]
harness = "omp"
model = "google-antigravity/gemini-3.8-flash"
can_review = false
[kinds.suhuynh]
harness = "claude"
model = "opus-5.5"
can_review = true
[kinds.ni]
harness = "claude"
model = "opus-5.5"
can_review = true
"#;
    let cfg: quaqua::config::Config = toml::from_str(toml_str).unwrap();

    // 1. With builder sude (omp), select_default_reviewer selects suhuynh
    let sel1 = select_default_reviewer("sude", &cfg).unwrap();
    assert_eq!(sel1, "suhuynh");

    // 2. When suhuynh is unavailable (via QUAQUA_UNAVAILABLE_REVIEWERS probe), falls back to ni
    std::env::set_var("QUAQUA_UNAVAILABLE_REVIEWERS", "suhuynh");
    let sel_fallback = select_default_reviewer("sude", &cfg).unwrap();
    std::env::remove_var("QUAQUA_UNAVAILABLE_REVIEWERS");
    assert_eq!(sel_fallback, "ni");
}

#[test]
fn test_pipeline_project_override_cap() {
    let toml_str = r#"
schema_version = 2
[limits]
max_workers_per_orchestrator = 4
weights = { pipeline = 2 }
[projects.zimage]
max_workers_per_orchestrator = 3
"#;
    let base_cfg: quaqua::config::Config = toml::from_str(toml_str).unwrap();
    let resolved = quaqua::config::resolve(base_cfg.clone(), Some("zimage")).unwrap();
    assert_eq!(resolved.limits.max_workers_per_orchestrator, 3);

    let pipe_weight = resolved.limits.weights.get("pipeline").copied().unwrap_or(2);
    let slots_used = 2; // owner has 2 slots in use
    // For standard project (cap 4): 2 + 2 <= 4 -> ok
    assert!(slots_used + pipe_weight <= base_cfg.limits.max_workers_per_orchestrator);
    // For zimage (cap 3): 2 + 2 > 3 -> refused
    assert!(slots_used + pipe_weight > resolved.limits.max_workers_per_orchestrator);
}

#[test]
fn test_sude_harness_mapped_to_omp() {
    use quaqua::pipeline::harness_of;
    assert_eq!(harness_of("sude"), Some("omp"));
}

#[test]
fn test_claude_review_mode_verification() {
    use quaqua::worker::spawn::parse_spawn_args;

    let args = vec![
        "--review".to_string(),
        "suhuynh-testrev".to_string(),
        "suhuynh".to_string(),
        ".".to_string(),
    ];
    let opts = parse_spawn_args(&args).unwrap();
    assert!(opts.review);
    assert_eq!(opts.kind, "suhuynh");
}

fn kind(harness: &str, model: &str, effort: Option<&str>) -> config::Kind {
    config::Kind {
        harness: harness.to_string(),
        model: model.to_string(),
        effort: effort.map(str::to_string),
        fallback: None,
        can_review: false,
    }
}

#[test]
fn test_launch_args_carry_exact_model_and_effort() {
    // opencode: model+effort travel as `provider/model#variant` in the pane env —
    // the interactive TUI lists no --model flag (`opencode --help`), only
    // --standalone/--server/--auto/--continue/--session/--prompt.
    let oc = resolve_launch(
        &kind("opencode", "opencode-go/deepseek-v4.1-flash", Some("max")),
        "tieude",
    )
    .unwrap();
    assert_eq!(
        oc,
        Launch {
            harness: "opencode".into(),
            model: "opencode-go/deepseek-v4.1-flash".into(),
            effort: "max".into(),
        }
    );
    let cmd = build_launch_cmd(&oc, "tieude-test", "/r/x.rules.md", "~/quaqua/runs", "/repo/.wt/x", false)
        .unwrap();
    assert_eq!(cmd.split_envs.len(), 2);
    assert_eq!(cmd.split_envs[0].0, "OPENCODE_CONFIG_CONTENT");
    assert_eq!(
        cmd.split_envs[0].1,
        r#"{"model":{"providerID":"opencode-go","model":"deepseek-v4.1-flash","variant":"max"},"instructions":["/r/x.rules.md"]}"#
    );
    assert_eq!(cmd.split_envs[1].0, "XDG_STATE_HOME");
    assert_eq!(cmd.split_envs[1].1, "~/quaqua/runs/.opencode-state/tieude-test");

    // opencode carries "low" variant as object
    let oc_low = resolve_launch(
        &kind("opencode", "opencode-go/deepseek-v4.1-flash", Some("low")),
        "tieude",
    )
    .unwrap();
    let cmd_low = build_launch_cmd(&oc_low, "tieude-low", "/r/x.rules.md", "~/quaqua/runs", "/repo/.wt/x", false)
        .unwrap();
    assert_eq!(
        cmd_low.split_envs[0].1,
        r#"{"model":{"providerID":"opencode-go","model":"deepseek-v4.1-flash","variant":"low"},"instructions":["/r/x.rules.md"]}"#
    );
    assert_eq!(cmd.agent_args, vec!["--", "--standalone", "--auto"]);
    assert_eq!(cmd_low.split_envs[1].0, "XDG_STATE_HOME");
    assert_eq!(cmd_low.split_envs[1].1, "~/quaqua/runs/.opencode-state/tieude-low");

    // omp: `--model` + `--thinking` (`omp --help`).
    let omp = resolve_launch(
        &kind("omp", "google-antigravity/gemini-3.8-flash", Some("high")),
        "sude",
    )
    .unwrap();
    let cmd = build_launch_cmd(&omp, "sude-test", "/r/x.rules.md", "~/quaqua/runs", "/repo/.wt/x", false)
        .unwrap();
    assert!(cmd.split_envs.is_empty(), "omp takes no pane env");
    let joined = cmd.agent_args.join(" ");
    assert!(
        joined.contains("--model google-antigravity/gemini-3.8-flash"),
        "{joined}"
    );
    assert!(joined.contains("--thinking high"), "{joined}");

    // claude: `--model` (mapped to claude-*) + `--effort` (`claude --help`).
    let cl = resolve_launch(&kind("claude", "opus-5.5", Some("high")), "truonglao").unwrap();
    let cmd = build_launch_cmd(&cl, "truonglao-test", "/r/x.rules.md", "~/quaqua/runs", "/repo/.wt/x", true)
        .unwrap();
    let joined = cmd.agent_args.join(" ");
    assert!(joined.contains("--model claude-opus-5-5"), "{joined}");
    assert!(joined.contains("--effort high"), "{joined}");
    // review keeps its read-only boundary
    assert!(joined.contains("--permission-mode dontAsk"), "{joined}");
}

#[test]
fn test_launch_fails_closed_without_model_or_effort() {
    let e = resolve_launch(&kind("opencode", "m", None), "tieude")
        .unwrap_err()
        .1;
    assert!(e.contains("no effort in team.toml"), "{e}");

    let e = resolve_launch(&kind("opencode", "m", Some("banana")), "tieude")
        .unwrap_err()
        .1;
    assert!(e.contains("does not take effort 'banana'"), "{e}");

    // opencode model variant vocabulary is enforced per-model
    let e = resolve_launch(
        &kind("opencode", "opencode-go/deepseek-v4.1-flash", Some("medium")),
        "tieude",
    )
    .unwrap_err()
    .1;
    assert!(e.contains("does not take effort 'medium'"), "{e}");

    // openrouter model variant vocabulary is matched on last segment
    let e = resolve_launch(
        &kind("opencode", "openrouter/deepseek/deepseek-v4.1-flash", Some("medium")),
        "tieude",
    )
    .unwrap_err()
    .1;
    assert!(e.contains("does not take effort 'medium'"), "{e}");

    // unknown opencode model rejected with allowlist error
    let e = resolve_launch(
        &kind("opencode", "opencode-go/unknown-model", Some("max")),
        "tieude",
    )
    .unwrap_err()
    .1;
    assert!(e.contains("is not in allowlist"), "{e}");

    // claude's vocabulary is narrower than opencode's
    let e = resolve_launch(&kind("claude", "m", Some("minimal")), "ni")
        .unwrap_err()
        .1;
    assert!(e.contains("does not take effort 'minimal'"), "{e}");

    let e = resolve_launch(&kind("opencode", "  ", Some("max")), "tieude")
        .unwrap_err()
        .1;
    assert!(e.contains("has no model"), "{e}");

    let e = resolve_launch(&kind("weird", "m", Some("max")), "x")
        .unwrap_err()
        .1;
    assert!(e.contains("unsupported harness"), "{e}");
}

#[test]
fn test_fallback_header_line_and_stamping() {
    let srt = r#"{"name":"x","kind":"sude","fallback":{"from":"sude","to":"detu","reason":"agent_not_ready"}}"#;
    assert_eq!(
        fallback_header_line(srt).as_deref(),
        Some("fallback: sude→detu (agent_not_ready)")
    );
    // no fallback in the metadata -> no header
    assert_eq!(fallback_header_line(r#"{"name":"x","kind":"tieude"}"#), None);
    // malformed / incomplete -> no header (never a panic)
    assert_eq!(fallback_header_line("not json"), None);
    assert_eq!(fallback_header_line(r#"{"fallback":{"from":"sude"}}"#), None);

    let report = "# sude-x — report\n\n## Lessons\n- none\n";
    let stamped = prepend_header(report, "fallback: sude→detu (agent_not_ready)").unwrap();
    assert!(stamped.starts_with("fallback: sude→detu (agent_not_ready)\n# sude-x"));
    // idempotent: already present -> None (no rewrite)
    assert_eq!(prepend_header(&stamped, "fallback: sude→detu (agent_not_ready)"), None);
}

#[test]
fn test_start_failure_reason_reads_herdr_error_code() {
    fn out(stdout: &str) -> Output {
        Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(1 << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }
    assert_eq!(
        start_failure_reason(&out(
            r#"{"error":{"code":"agent_not_ready","message":"blocked during startup"}}"#
        )),
        "agent_not_ready"
    );
    assert_eq!(
        start_failure_reason(&out(r#"{"error":{"code":"quota_exceeded"}}"#)),
        "quota_exceeded"
    );
    assert_eq!(start_failure_reason(&out("plain text, not json")), "start-failed");
}

#[test]
fn test_fallback_kind_resolution() {
    let mut cfg = config::Config::default();
    cfg.kinds.insert(
        "tieude".to_string(),
        config::Kind {
            harness: "opencode".to_string(),
            model: "m".to_string(),
            effort: Some("max".to_string()),
            fallback: Some("detu".to_string()),
            can_review: false,
        },
    );
    cfg.kinds.insert(
        "detu".to_string(),
        config::Kind {
            harness: "opencode".to_string(),
            model: "m".to_string(),
            effort: Some("max".to_string()),
            fallback: None,
            can_review: false,
        },
    );

    // Valid fallback resolves to detu
    let (fb_name, fb_kind) = fallback_kind(&cfg, &cfg.kinds["tieude"], "tieude")
        .unwrap()
        .unwrap();
    assert_eq!(fb_name, "detu");
    assert_eq!(fb_kind.harness, "opencode");

    // No fallback returns Ok(None)
    assert!(fallback_kind(&cfg, &cfg.kinds["detu"], "detu").unwrap().is_none());

    // Fallback naming a missing kind returns Err
    let mut bad_kind = cfg.kinds["tieude"].clone();
    bad_kind.fallback = Some("ghost".to_string());
    let err = fallback_kind(&cfg, &bad_kind, "tieude").unwrap_err().1;
    assert!(err.contains("fallback 'ghost' is not a kind"), "{err}");
}

struct SpawnTestGit;
impl GitOps for SpawnTestGit {
    fn run_in(&self, _dir: &Path, _args: &[&str]) -> std::io::Result<Output> {
        Ok(Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn run(&self, _args: &[&str]) -> std::io::Result<Output> {
        self.run_in(Path::new("."), _args)
    }
    fn rev_parse_commit(&self, _repo: &Path, _base: &str) -> bool {
        true
    }
    fn worktree_list_porcelain(&self, _repo: &Path) -> std::io::Result<Vec<String>> {
        Ok(vec![])
    }
    fn worktree_add(
        &self,
        _repo: &Path,
        _branch: &str,
        wt: &Path,
        _base: Option<&str>,
    ) -> std::io::Result<Output> {
        let _ = std::fs::create_dir_all(wt);
        self.run_in(_repo, &[])
    }
    fn branch_exists(&self, _repo: &Path, _branch: &str) -> bool {
        false
    }
    fn worktree_add_existing(&self, _repo: &Path, _branch: &str, _wt: &Path) -> std::io::Result<Output> {
        self.run_in(_repo, &[])
    }
    fn worktree_remove(&self, _repo: &Path, wt: &Path) -> std::io::Result<Output> {
        let _ = std::fs::remove_dir_all(wt);
        self.run_in(_repo, &[])
    }
    fn remote_list(&self, _repo: &Path) -> std::io::Result<Vec<String>> {
        Ok(vec![])
    }
    fn is_core_bare(&self, _repo: &Path) -> bool {
        false
    }
    fn has_core_worktree(&self, _repo: &Path) -> bool {
        false
    }
    fn enable_worktree_config(&self, _repo: &Path) -> std::io::Result<Output> {
        self.run_in(_repo, &[])
    }
    fn set_worktree_pushurl_blocked(&self, _wt: &Path, _remote: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn is_git_dir(&self, _cwd: &Path) -> bool {
        true
    }
    fn diff_and_status(&self, _cwd: &Path) -> (String, String) {
        ("dh".to_string(), "clean".to_string())
    }
}

struct FallbackSpyHerdr {
    agent_starts: std::cell::RefCell<Vec<(String, String)>>,
    fail_harness: Option<String>,
}

impl HerdrOps for FallbackSpyHerdr {
    fn run(&self, _args: &[&str]) -> std::io::Result<Output> {
        Ok(Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: b"{\"snapshot\":{\"workspaces\":[],\"tabs\":[],\"panes\":[],\"layouts\":[],\"agents\":[]}}".to_vec(),
            stderr: Vec::new(),
        })
    }
    fn agent_list(&self) -> std::io::Result<String> {
        Ok(r#"{"result":{"agents":[]}}"#.to_string())
    }
    fn agent_get(&self, _name: &str) -> std::io::Result<(bool, String)> {
        Ok((false, String::new()))
    }
    fn agent_read(&self, _name: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        Ok(String::new())
    }
    fn agent_start(
        &self,
        name: &str,
        kind: &str,
        _pane: &str,
        _timeout_ms: u32,
        _extra_args: &[&str],
    ) -> std::io::Result<Output> {
        self.agent_starts
            .borrow_mut()
            .push((name.to_string(), kind.to_string()));
        let should_fail = match &self.fail_harness {
            Some(fh) => fh == "*" || fh == kind,
            None => false,
        };
        if should_fail {
            Ok(Output {
                status: std::os::unix::process::ExitStatusExt::from_raw(1 << 8),
                stdout: b"{\"error\":{\"code\":\"agent_not_ready\",\"message\":\"blocked during startup\"}}".to_vec(),
                stderr: Vec::new(),
            })
        } else {
            Ok(Output {
                status: std::os::unix::process::ExitStatusExt::from_raw(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }
    fn pane_split(
        &self,
        _cwd: &str,
        _owner: &str,
        _envs: &[(&str, &str)],
    ) -> std::io::Result<Output> {
        Ok(Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: b"{\"result\":{\"pane\":{\"pane_id\":\"%1\"}}}".to_vec(),
            stderr: Vec::new(),
        })
    }
    fn pane_close(&self, _pane: &str) -> std::io::Result<Output> {
        Ok(Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn pane_report_metadata(&self, _pane: &str, _tokens: &[&str]) -> std::io::Result<Output> {
        Ok(Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn pane_wait_output(&self, _pane: &str, _pattern: &str, _timeout_ms: u32) -> bool {
        false
    }
    fn pane_read(&self, _pane: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        Ok(String::new())
    }
    fn pane_send_keys(&self, _pane: &str, _key: &str) -> std::io::Result<Output> {
        Ok(Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn agent_send_keys(&self, _name: &str, _key: &str) -> std::io::Result<Output> {
        Ok(Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn agent_wait_idle(&self, _name: &str, _timeout_ms: u32) -> std::io::Result<Output> {
        let code = if self.fail_harness.is_some() { 1 } else { 0 };
        Ok(Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(code << 8),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
    fn agent_prompt_nowait(&self, _name: &str, _prompt: &str) -> std::io::Result<Output> {
        Ok(Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
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

fn test_cfg_with_fallback() -> config::Config {
    let mut cfg = config::Config::default();
    cfg.kinds.insert(
        "sude".to_string(),
        config::Kind {
            harness: "omp".to_string(),
            model: "gemini".to_string(),
            effort: Some("high".to_string()),
            fallback: Some("detu".to_string()),
            can_review: false,
        },
    );
    cfg.kinds.insert(
        "detu".to_string(),
        config::Kind {
            harness: "opencode".to_string(),
            model: "deepseek-v4.1-flash".to_string(),
            effort: Some("max".to_string()),
            fallback: Some("tieude".to_string()),
            can_review: false,
        },
    );
    cfg.kinds.insert(
        "ni".to_string(),
        config::Kind {
            harness: "claude".to_string(),
            model: "opus".to_string(),
            effort: Some("low".to_string()),
            fallback: None,
            can_review: true,
        },
    );
    cfg
}

struct EnvLockGuard;
static ENV_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn lock_env() -> EnvLockGuard {
    while ENV_BUSY.compare_exchange_weak(false, true, std::sync::atomic::Ordering::Acquire, std::sync::atomic::Ordering::Relaxed).is_err() {
        std::thread::yield_now();
    }
    EnvLockGuard
}
impl Drop for EnvLockGuard {
    fn drop(&mut self) {
        std::env::remove_var("QUAQUA_RUNS_DIR");
        std::env::remove_var("QUAQUA_OWNER");
        std::env::remove_var("GRID_TAB");
        std::env::remove_var("GRID_CHECK_TIMEOUT_SEC");
        std::env::remove_var("HERDR_STUB_SNAPSHOT");
        std::env::remove_var("GRID_SKIP");
        std::env::remove_var("QUAQUA_TEST");
        ENV_BUSY.store(false, std::sync::atomic::Ordering::Release);
    }
}
#[test]
fn test_spawn_worker_fallback_scenarios() {
    let _env_lock = lock_env();
    let temp = std::env::temp_dir().join(format!("quaqua-spawn-fb-{}", std::process::id()));
    let repo_dir = temp.join("repo");
    let runs_dir = temp.join("quaqua/runs");
    let _ = std::fs::create_dir_all(&repo_dir);
    let _ = std::fs::create_dir_all(&runs_dir);
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_TEST", "1");
    std::env::set_var("GRID_SKIP", "1");

    // Scenario 1: Primary start fails (omp), falls back to detu (opencode), srt.json recorded
    {
        let cfg = test_cfg_with_fallback();
        let git = SpawnTestGit;
        let herdr = FallbackSpyHerdr {
            agent_starts: std::cell::RefCell::new(Vec::new()),
            fail_harness: Some("omp".to_string()),
        };

        let opts = parse_spawn_args(&[
            "sude-fbtest".to_string(),
            "sude".to_string(),
            repo_dir.to_str().unwrap().to_string(),
        ])
        .unwrap();

        let res = quaqua::worker::spawn::spawn_worker(opts, &cfg, &git, &herdr);
        assert!(
            res.is_ok(),
            "spawn should succeed with fallback: {:?}",
            res.err()
        );

        let starts = herdr.agent_starts.borrow().clone();
        assert_eq!(starts.len(), 2, "expected exactly 2 starts (primary, fallback)");
        assert_eq!(starts[0], ("sude-fbtest".to_string(), "omp".to_string()));
        assert_eq!(
            starts[1],
            ("sude-fbtest".to_string(), "opencode".to_string())
        );

        let srt_path = runs_dir.join("sude-fbtest.srt.json");
        assert!(srt_path.is_file(), "srt.json must be written");
        let srt_content = std::fs::read_to_string(&srt_path).unwrap();
        assert!(
            srt_content.contains(r#""fallback":{"from":"sude","to":"detu","reason":"agent_not_ready"}"#),
            "{srt_content}"
        );
        assert!(
            srt_content.contains(r#""running_kind":"detu""#),
            "srt.json must record running_kind after fallback: {srt_content}"
        );
        assert!(
            srt_content.contains(r#""harness":"opencode""#),
            "srt.json must record running harness after fallback: {srt_content}"
        );
        assert!(
            srt_content.contains(r#""kind":"sude""#),
            "srt.json must preserve original kind field: {srt_content}"
        );
    }

    // Scenario 2: One hop limit — both primary and fallback fail -> error after 2 starts
    {
        let cfg = test_cfg_with_fallback();
        let git = SpawnTestGit;
        let herdr = FallbackSpyHerdr {
            agent_starts: std::cell::RefCell::new(Vec::new()),
            fail_harness: Some("*".to_string()),
        };

        let opts = parse_spawn_args(&[
            "sude-onehop".to_string(),
            "sude".to_string(),
            repo_dir.to_str().unwrap().to_string(),
        ])
        .unwrap();

        let res = quaqua::worker::spawn::spawn_worker(opts, &cfg, &git, &herdr);
        assert!(res.is_err(), "must error when fallback fails");

        let starts = herdr.agent_starts.borrow().clone();
        assert_eq!(starts.len(), 2, "expected exactly 2 starts (one hop max)");
    }

    // Scenario 3: Fallback naming missing kind -> error
    {
        let mut cfg = test_cfg_with_fallback();
        cfg.kinds.get_mut("sude").unwrap().fallback = Some("ghost".to_string());
        let git = SpawnTestGit;
        let herdr = FallbackSpyHerdr {
            agent_starts: std::cell::RefCell::new(Vec::new()),
            fail_harness: Some("omp".to_string()),
        };

        let opts = parse_spawn_args(&[
            "sude-fbmiss".to_string(),
            "sude".to_string(),
            repo_dir.to_str().unwrap().to_string(),
        ])
        .unwrap();

        let res = quaqua::worker::spawn::spawn_worker(opts, &cfg, &git, &herdr);
        assert!(res.is_err(), "must error when fallback kind is missing");

        let starts = herdr.agent_starts.borrow().clone();
        assert_eq!(
            starts.len(),
            1,
            "only primary attempted before bad fallback error"
        );
    }

    // Scenario 4: A review spawn (--review) refuses fallback to a non-review kind
    {
        let mut cfg = test_cfg_with_fallback();
        cfg.kinds.get_mut("ni").unwrap().fallback = Some("detu".to_string());
        let git = SpawnTestGit;
        let herdr = FallbackSpyHerdr {
            agent_starts: std::cell::RefCell::new(Vec::new()),
            fail_harness: Some("claude".to_string()),
        };

        let opts = parse_spawn_args(&[
            "--review".to_string(),
            "ni-revtest".to_string(),
            "ni".to_string(),
            repo_dir.to_str().unwrap().to_string(),
        ])
        .unwrap();

        let res = quaqua::worker::spawn::spawn_worker(opts, &cfg, &git, &herdr);
        assert!(
            res.is_err(),
            "must refuse fallback to non-review kind when in review mode"
        );
        let starts = herdr.agent_starts.borrow().clone();
        assert_eq!(
            starts.len(),
            1,
            "must not attempt start of non-review fallback kind"
        );
    }

    std::env::remove_var("QUAQUA_RUNS_DIR");
    std::env::remove_var("QUAQUA_TEST");
    std::env::remove_var("GRID_SKIP");
    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn test_spawn_worker_grid_timeout_fails() {
    let temp = std::env::temp_dir().join(format!("quaqua-spawn-gridfail-{}", std::process::id()));
    let repo_dir = temp.join("repo");
    let _env_lock = lock_env();
    let runs_dir = temp.join("quaqua/runs");
    let _ = std::fs::create_dir_all(&repo_dir);
    let _ = std::fs::create_dir_all(&runs_dir);
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_OWNER", "cos");
    std::env::set_var("GRID_TAB", "quaqua-workers");
    std::env::set_var("GRID_CHECK_TIMEOUT_SEC", "0");
    std::env::remove_var("GRID_SKIP");

    let fix = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/herdr-grid/2-workers.json");
    std::env::set_var("HERDR_STUB_SNAPSHOT", fix.to_str().unwrap());

    let cfg = test_cfg_with_fallback();
    let git = SpawnTestGit;
    let herdr = FallbackSpyHerdr {
        agent_starts: std::cell::RefCell::new(Vec::new()),
        fail_harness: None,
    };

    let opts = parse_spawn_args(&[
        "sude-gridfail".to_string(),
        "sude".to_string(),
        repo_dir.to_str().unwrap().to_string(),
    ])
    .unwrap();

    let res = quaqua::worker::spawn::spawn_worker(opts, &cfg, &git, &herdr);

    std::env::remove_var("QUAQUA_RUNS_DIR");
    std::env::remove_var("QUAQUA_OWNER");
    std::env::remove_var("GRID_TAB");
    std::env::remove_var("GRID_CHECK_TIMEOUT_SEC");
    std::env::remove_var("HERDR_STUB_SNAPSHOT");
    let _ = std::fs::remove_dir_all(&temp);

    let err = res.expect_err("spawn_worker must fail when grid check times out");
    assert_eq!(err.0, 1);
    assert!(err.1.contains("grid"), "error must mention grid: {}", err.1);
    assert!(err.1.contains("sude-gridfail"), "error must name the live worker: {}", err.1);
}

#[test]
fn test_cleanup_worker_grid_timeout_fails() {
    let temp = std::env::temp_dir().join(format!("quaqua-clean-gridfail-{}", std::process::id()));
    let repo_dir = temp.join("repo");
    let runs_dir = temp.join("quaqua/runs");
    let _env_lock = lock_env();
    let _ = std::fs::create_dir_all(&repo_dir);
    let _ = std::fs::create_dir_all(&runs_dir);
    let wt = temp.join(".wt/sude-gridfail");
    let _ = std::fs::create_dir_all(&wt);
    let owner_file = runs_dir.join("sude-gridfail.owner");
    let _ = std::fs::write(&owner_file, "cos\n");
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_OWNER", "cos");
    std::env::set_var("GRID_TAB", "quaqua-workers");
    std::env::set_var("GRID_CHECK_TIMEOUT_SEC", "0");
    std::env::remove_var("GRID_SKIP");

    let fix = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/herdr-grid/2-workers.json");
    std::env::set_var("HERDR_STUB_SNAPSHOT", fix.to_str().unwrap());

    let git = SpawnTestGit;
    struct CleanupTestHerdr;
    impl HerdrOps for CleanupTestHerdr {
        fn run(&self, _: &[&str]) -> std::io::Result<Output> { unimplemented!() }
        fn agent_list(&self) -> std::io::Result<String> { unimplemented!() }
        fn agent_get(&self, _: &str) -> std::io::Result<(bool, String)> {
            Ok((true, r#"{"result":{"agent":{"pane_id":"w1:p2"}}}"#.to_string()))
        }
        fn agent_read(&self, _: &str, _: &str, _: u32) -> std::io::Result<String> { unimplemented!() }
        fn agent_start(&self, _: &str, _: &str, _: &str, _: u32, _: &[&str]) -> std::io::Result<Output> { unimplemented!() }
        fn pane_split(&self, _: &str, _: &str, _: &[(&str, &str)]) -> std::io::Result<Output> { unimplemented!() }
        fn pane_close(&self, _: &str) -> std::io::Result<Output> {
            Ok(Output { status: std::process::ExitStatus::default(), stdout: vec![], stderr: vec![] })
        }
        fn pane_report_metadata(&self, _: &str, _: &[&str]) -> std::io::Result<Output> { unimplemented!() }
        fn pane_wait_output(&self, _: &str, _: &str, _: u32) -> bool { unimplemented!() }
        fn pane_read(&self, _: &str, _: &str, _: u32) -> std::io::Result<String> { unimplemented!() }
        fn pane_send_keys(&self, _: &str, _: &str) -> std::io::Result<Output> { unimplemented!() }
        fn agent_send_keys(&self, _: &str, _: &str) -> std::io::Result<Output> { unimplemented!() }
        fn agent_wait_idle(&self, _: &str, _: u32) -> std::io::Result<Output> { unimplemented!() }
        fn agent_prompt_nowait(&self, _: &str, _: &str) -> std::io::Result<Output> { unimplemented!() }
        fn agent_prompt_wait_background(&self, _: &str, _: &str, _: u32, _: std::fs::File) -> std::io::Result<Child> { unimplemented!() }
    }
    let herdr = CleanupTestHerdr;

    let res = quaqua::worker::cleanup::cleanup_worker("sude-gridfail", &repo_dir, &git, &herdr);

    let wt_exists = wt.exists();
    let owner_file_exists = owner_file.exists();
    std::env::remove_var("QUAQUA_RUNS_DIR");
    std::env::remove_var("QUAQUA_OWNER");
    std::env::remove_var("GRID_TAB");
    std::env::remove_var("GRID_CHECK_TIMEOUT_SEC");
    std::env::remove_var("HERDR_STUB_SNAPSHOT");
    let _ = std::fs::remove_dir_all(&temp);

    let err = res.expect_err("cleanup_worker must fail when grid check times out");
    assert_eq!(err.0, 1);
    assert!(err.1.contains("grid"), "error must mention grid: {}", err.1);
    assert!(!wt_exists, "worktree must be removed even when grid check fails");
    assert!(!owner_file_exists, "metadata must be removed even when grid check fails");
}

// --- quaqua worker stop ---------------------------------------------------

use quaqua::worker::stop::{harness_from_info, parse_stop_args, stop_keys, stop_worker};
use std::sync::Mutex;

fn agent_get_json(harness: &str) -> String {
    format!(
        "{{\"result\":{{\"agent\":{{\"agent\":\"{harness}\",\"name\":\"tieude-x\",\"pane_id\":\"w1:p2\"}}}}}}"
    )
}

/// Records the keys stop sends; everything else is unused by stop.
/// `agent_get` is either an Ok body or an io failure (herdr unreachable).
struct StopSpy {
    agent_get: Result<String, String>,
    sent: Mutex<Vec<(String, String)>>,
}

impl StopSpy {
    fn new(agent_json: &str) -> Self {
        StopSpy {
            agent_get: Ok(agent_json.to_string()),
            sent: Mutex::new(Vec::new()),
        }
    }

    /// herdr itself cannot be reached (binary missing, socket down).
    fn new_unreachable(io_message: &str) -> Self {
        StopSpy {
            agent_get: Err(io_message.to_string()),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.sent.lock().unwrap().clone()
    }

    fn ok() -> std::io::Result<Output> {
        Ok(Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
}

impl HerdrOps for StopSpy {
    fn run(&self, _args: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_list(&self) -> std::io::Result<String> {
        unimplemented!()
    }
    fn agent_get(&self, _name: &str) -> std::io::Result<(bool, String)> {
        match &self.agent_get {
            Ok(body) => Ok((true, body.clone())),
            Err(message) => Err(std::io::Error::new(std::io::ErrorKind::Other, message.clone())),
        }
    }
    fn agent_read(&self, _name: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        unimplemented!()
    }
    fn agent_start(&self, _name: &str, _kind: &str, _pane: &str, _timeout_ms: u32, _extra_args: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_split(&self, _cwd: &str, _owner: &str, _envs: &[(&str, &str)]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_close(&self, _pane: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_report_metadata(&self, _pane: &str, _tokens: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_wait_output(&self, _pane: &str, _pattern: &str, _timeout_ms: u32) -> bool {
        unimplemented!()
    }
    fn pane_read(&self, _pane: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        unimplemented!()
    }
    fn pane_send_keys(&self, _pane: &str, _key: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_send_keys(&self, name: &str, key: &str) -> std::io::Result<Output> {
        self.sent
            .lock()
            .unwrap()
            .push((name.to_string(), key.to_string()));
        StopSpy::ok()
    }
    fn agent_wait_idle(&self, _name: &str, _timeout_ms: u32) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_prompt_nowait(&self, _name: &str, _prompt: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_prompt_wait_background(&self, _name: &str, _prompt: &str, _timeout_ms: u32, _log_file: std::fs::File) -> std::io::Result<Child> {
        unimplemented!()
    }
}

#[test]
fn test_stop_keys_by_harness() {
    assert_eq!(stop_keys("opencode"), Some(&["esc", "esc"][..]));
    assert_eq!(stop_keys("omp"), Some(&["esc"][..]));
    // No verified sequence for claude yet (Lead decision B, 2026-10-17):
    // esc esc would open Claude Code's message selector, so stop refuses.
    assert_eq!(stop_keys("claude"), None);
    assert_eq!(stop_keys("aider"), None);
    assert_eq!(stop_keys(""), None);
}

#[test]
fn test_stop_opencode_sends_esc_twice_in_order() {
    let spy = StopSpy::new(&agent_get_json("opencode"));
    stop_worker("tieude-x", &spy).unwrap();
    assert_eq!(
        spy.calls(),
        vec![
            ("tieude-x".to_string(), "esc".to_string()),
            ("tieude-x".to_string(), "esc".to_string()),
        ]
    );
}

#[test]
fn test_stop_omp_sends_esc_once() {
    let spy = StopSpy::new(&agent_get_json("omp"));
    stop_worker("sude-x", &spy).unwrap();
    assert_eq!(spy.calls(), vec![("sude-x".to_string(), "esc".to_string())]);
}

#[test]
fn test_stop_unmapped_harness_fails_closed_and_sends_nothing() {
    for harness in ["claude", "aider"] {
        let spy = StopSpy::new(&agent_get_json(harness));
        let err = stop_worker("tieude-x", &spy).unwrap_err();
        assert_eq!(err.0, 1, "exit code for {harness}");
        assert!(err.1.contains(harness), "error names harness {harness}: {}", err.1);
        assert!(err.1.contains("tieude-x"), "error names the worker: {}", err.1);
        assert!(spy.calls().is_empty(), "no keys for {harness}");
    }
}

#[test]
fn test_stop_missing_agent_fails_closed_and_sends_nothing() {
    let spy = StopSpy::new(r#"{"error":{"code":"agent_not_found","message":"no such agent"}}"#);
    let err = stop_worker("tieude-gone", &spy).unwrap_err();
    assert_eq!(err.0, 1);
    assert!(err.1.contains("tieude-gone"), "{}", err.1);
    assert!(spy.calls().is_empty());
}

#[test]
fn test_stop_harness_from_info_parses_only_a_real_agent() {
    assert_eq!(harness_from_info(&agent_get_json("opencode")).as_deref(), Some("opencode"));
    assert_eq!(harness_from_info(&agent_get_json("omp")).as_deref(), Some("omp"));
    // Agent gone, harness missing/empty, or garbage: no harness, nothing to send.
    assert_eq!(harness_from_info(r#"{"error":{"code":"agent_not_found"}}"#), None);
    assert_eq!(harness_from_info(r#"{"result":{"agent":{"agent":""}}}"#), None);
    assert_eq!(harness_from_info(r#"{"result":{}}"#), None);
    assert_eq!(harness_from_info("not json"), None);
}

#[test]
fn test_stop_unreachable_herdr_is_reported_as_io_not_missing_agent() {
    let spy = StopSpy::new_unreachable("No such file or directory (os error 2)");
    let err = stop_worker("tieude-x", &spy).unwrap_err();
    assert_eq!(err.0, 1);
    assert!(err.1.contains("cannot reach herdr"), "{}", err.1);
    assert!(err.1.contains("No such file or directory"), "{}", err.1);
    assert!(!err.1.contains("not running"), "herdr I/O is not a missing agent: {}", err.1);
    assert!(spy.calls().is_empty());
}

#[test]
fn test_stop_requires_exactly_one_valid_name() {
    assert!(parse_stop_args(&[]).is_err());
    assert!(parse_stop_args(&["".to_string()]).is_err());
    assert!(parse_stop_args(&["a".to_string(), "b".to_string()]).is_err());
    assert!(parse_stop_args(&["Tieude-X".to_string()]).is_err());
    assert!(parse_stop_args(&["--force".to_string()]).is_err());
    assert_eq!(parse_stop_args(&["tieude-x".to_string()]).unwrap(), "tieude-x");
}

#[test]
fn test_cleanup_worker_removes_per_worker_opencode_state() {
    let _env_lock = lock_env();
    let temp = std::env::temp_dir().join(format!("quaqua-clean-state-{}", std::process::id()));
    let repo_dir = temp.join("repo");
    let runs_dir = temp.join("quaqua/runs");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::create_dir_all(&runs_dir).unwrap();

    let worker_name = "tieude-cleanup";
    let wt = temp.join(".wt").join(worker_name);
    std::fs::create_dir_all(&wt).unwrap();

    let state_dir = runs_dir.join(".opencode-state").join(worker_name);
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("model.json"), "{}").unwrap();
    assert!(state_dir.exists());

    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_TEST", "1");
    std::env::set_var("GRID_SKIP", "1");
    struct CleanHerdr;
    impl HerdrOps for CleanHerdr {
        fn run(&self, _: &[&str]) -> std::io::Result<Output> { unimplemented!() }
        fn agent_list(&self) -> std::io::Result<String> { unimplemented!() }
        fn agent_get(&self, _: &str) -> std::io::Result<(bool, String)> {
            Ok((true, r#"{"result":{"agent":{"pane_id":"w1:p2"}}}"#.to_string()))
        }
        fn agent_read(&self, _: &str, _: &str, _: u32) -> std::io::Result<String> { unimplemented!() }
        fn agent_start(&self, _: &str, _: &str, _: &str, _: u32, _: &[&str]) -> std::io::Result<Output> { unimplemented!() }
        fn pane_split(&self, _: &str, _: &str, _: &[(&str, &str)]) -> std::io::Result<Output> { unimplemented!() }
        fn pane_close(&self, _: &str) -> std::io::Result<Output> {
            Ok(Output { status: std::process::ExitStatus::default(), stdout: vec![], stderr: vec![] })
        }
        fn pane_report_metadata(&self, _: &str, _: &[&str]) -> std::io::Result<Output> { unimplemented!() }
        fn pane_wait_output(&self, _: &str, _: &str, _: u32) -> bool { unimplemented!() }
        fn pane_read(&self, _: &str, _: &str, _: u32) -> std::io::Result<String> { unimplemented!() }
        fn pane_send_keys(&self, _: &str, _: &str) -> std::io::Result<Output> { unimplemented!() }
        fn agent_send_keys(&self, _: &str, _: &str) -> std::io::Result<Output> { unimplemented!() }
        fn agent_wait_idle(&self, _: &str, _: u32) -> std::io::Result<Output> { unimplemented!() }
        fn agent_prompt_nowait(&self, _: &str, _: &str) -> std::io::Result<Output> { unimplemented!() }
        fn agent_prompt_wait_background(&self, _: &str, _: &str, _: u32, _: std::fs::File) -> std::io::Result<std::process::Child> { unimplemented!() }
    }
    let git = SpawnTestGit;
    let herdr = CleanHerdr;
    let res = quaqua::worker::cleanup::cleanup_worker(worker_name, &repo_dir, &git, &herdr);
    assert!(res.is_ok(), "cleanup_worker failed: {:?}", res);
    assert!(!state_dir.exists(), "per-worker state dir must be removed on cleanup");

    let _ = std::fs::remove_dir_all(&temp);
}
