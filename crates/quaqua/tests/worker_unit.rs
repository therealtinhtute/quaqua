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

    // suhuynh/truonglao are claude with can_review=true; every kind is opencode/omp/claude
    assert_eq!(cfg.kinds["suhuynh"].harness, "claude");
    assert!(cfg.kinds["suhuynh"].can_review);
    assert_eq!(cfg.kinds["truonglao"].harness, "claude");
    assert!(cfg.kinds["truonglao"].can_review);
    assert_eq!(cfg.kinds.len(), 6, "only the six surviving kinds");
    assert!(cfg.kinds.values().all(|k| ["opencode", "omp", "claude"].contains(&k.harness.as_str())));
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
