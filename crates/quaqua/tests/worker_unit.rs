use quaqua::cap::{count_owner_slots, is_task_worker_name, matches_pipeline_reviewer};
use quaqua::config;
use quaqua::git::GitOps;
use quaqua::herdr::HerdrOps;
use quaqua::worker::cleanup::{resolve_default_runs_dir, resolve_home_quaqua_runs};
use quaqua::worker::prompt::{brief_budget_sec, brief_headers_ok, resolve_budget_sec, validate_env_budget_val};
use quaqua::worker::spawn::parse_spawn_args;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn test_task_worker_name_validation() {
    assert!(is_task_worker_name("tieude-task1"));
    assert!(is_task_worker_name("sude-hardimpl"));
    assert!(is_task_worker_name("quansu-review2"));
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
    // N2: quansu-prep-pr1 has '-prep' before '-pr1'; must match on second occurrence
    assert!(matches_pipeline_reviewer("quansu-prep-pr1", "test"));
    assert!(matches_pipeline_reviewer("quansu-pr1", "test"));
    assert!(matches_pipeline_reviewer("suhuynh-pr24", "test"));
    assert!(matches_pipeline_reviewer("quansu-testr1", "test"));
    assert!(matches_pipeline_reviewer("suhuynh-testr3", "test"));

    // N2: empty task matches *-r[0-9]*
    assert!(matches_pipeline_reviewer("suhuynh-r3r2", ""));
    assert!(matches_pipeline_reviewer("quansu-r1", ""));

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
    fn pane_send_keys(&self, _pane: &str, _key: &str) -> std::io::Result<Output> {
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
    fn pane_send_keys(&self, _pane: &str, _key: &str) -> std::io::Result<Output> {
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
fn test_runs_dir_resolution_n5() {
    // N5: no process-global set_var mutations
    let test_home = Path::new("target/test_home");
    assert_eq!(resolve_home_quaqua_runs(Some(test_home)), test_home.join("quaqua/runs"));
    assert_eq!(resolve_default_runs_dir(None, Some(test_home)), test_home.join("quaqua/runs"));

    let override_dir = Path::new("target/override_runs");
    assert_eq!(resolve_default_runs_dir(Some(override_dir), Some(test_home)), override_dir.to_path_buf());
    assert_eq!(resolve_home_quaqua_runs(Some(test_home)), test_home.join("quaqua/runs"));
}

#[test]
fn test_team_toml_harness_routing() {
    let team_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../team.toml");
    let cfg = config::load(&team_path).expect("valid team.toml");

    // tieude/detu are opencode
    assert_eq!(cfg.kinds["tieude"].harness, "opencode");
    assert_eq!(cfg.kinds["tieude"].model, "opencode-go/deepseek-v4.1-flash");
    assert!(!cfg.kinds["tieude"].can_review);

    // sude is omp with antigravity gemini high
    assert_eq!(cfg.kinds["sude"].harness, "omp");
    assert_eq!(cfg.kinds["sude"].model, "google-antigravity/gemini-3.8-flash");
    assert_eq!(cfg.kinds["sude"].effort.as_deref(), Some("high"));
    assert!(!cfg.kinds["sude"].can_review);

    // ni/suhuynh/truonglao are claude with can_review=true
    assert_eq!(cfg.kinds["ni"].harness, "claude");
    assert_eq!(cfg.kinds["ni"].model, "opus-5.5");
    assert!(cfg.kinds["ni"].can_review);

    // bacdau/quansu are codex with can_review=true
    assert_eq!(cfg.kinds["bacdau"].harness, "codex");
    assert!(cfg.kinds["bacdau"].can_review);
    assert_eq!(cfg.kinds["quansu"].harness, "codex");
    assert!(cfg.kinds["quansu"].can_review);
}

#[test]
fn test_spawn_args_parsing() {
    let args = vec![
        "--base".to_string(),
        "main".to_string(),
        "--review".to_string(),
        "quansu-rev1".to_string(),
        "quansu".to_string(),
        "/repo".to_string(),
    ];
    let opts = parse_spawn_args(&args).expect("valid args");
    assert_eq!(opts.base.as_deref(), Some("main"));
    assert!(opts.review);
    assert_eq!(opts.name, "quansu-rev1");
    assert_eq!(opts.kind, "quansu");

    // Trailing base without value
    let bad = vec![
        "quansu-rev1".to_string(),
        "quansu".to_string(),
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
        fn pane_send_keys(&self, _: &str, _: &str) -> std::io::Result<Output> {
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
    assert!(!has_real_blockers("## Blockers\n\n- none\n\n## Lessons\n- none\n"));
    assert!(!has_real_blockers("Just plan text\n\n## Lessons\n- none\n"));

    // Real blockers must return true
    assert!(has_real_blockers("## Blockers\n- cannot plan without the spec\n"));
    assert!(has_real_blockers("## Blockers\n- implementation incomplete\n"));
    assert!(has_real_blockers("## Blockers\n- none\n- also a real blocker\n"));
}

#[test]
fn test_strategy_default_reviewer_selection() {
    use quaqua::pipeline::select_default_reviewer;

    let toml_str = r#"
schema_version = 2
[strategy]
default_kind = "tieude"
default_reviewer = ["quansu", "suhuynh"]
builder_reviewer_differ = "harness"
[kinds.sude]
harness = "omp"
model = "google-antigravity/gemini-3.8-flash"
can_review = false
[kinds.quansu]
harness = "codex"
model = "gpt-6.1-sol"
can_review = true
[kinds.suhuynh]
harness = "claude"
model = "opus-5.5"
can_review = true
"#;
    let cfg: quaqua::config::Config = toml::from_str(toml_str).unwrap();

    // 1. With builder sude (omp), select_default_reviewer selects quansu
    let sel1 = select_default_reviewer("sude", &cfg).unwrap();
    assert_eq!(sel1, "quansu");

    // 2. When quansu is unavailable (via QUAQUA_UNAVAILABLE_REVIEWERS probe), falls back to suhuynh
    std::env::set_var("QUAQUA_UNAVAILABLE_REVIEWERS", "quansu");
    let sel_fallback = select_default_reviewer("sude", &cfg).unwrap();
    std::env::remove_var("QUAQUA_UNAVAILABLE_REVIEWERS");
    assert_eq!(sel_fallback, "suhuynh");

    // 3. With builder quansu (codex), skips quansu (same harness) and selects suhuynh
    let sel2 = select_default_reviewer("quansu", &cfg).unwrap();
    assert_eq!(sel2, "suhuynh");
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
