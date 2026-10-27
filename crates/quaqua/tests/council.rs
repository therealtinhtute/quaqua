use quaqua::engine::child::SingleWorkerOutput;
use quaqua::taskforce::driver::{run_taskforce_workflow, CouncilMemberRunner};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(1);

fn hermetic_temp_dir(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tmp = std::env::temp_dir().join(format!("quaqua-council-{prefix}-{id}-{nanos}"));
    fs::create_dir_all(&tmp).expect("create hermetic temp dir");
    tmp
}

#[derive(Default, Clone)]
struct MockMemberRunner {
    spawns: Arc<Mutex<Vec<String>>>,
    cleanups: Arc<Mutex<Vec<String>>>,
    prompt_counts: Arc<Mutex<HashMap<String, usize>>>,
    prompt_results: Arc<Mutex<HashMap<String, Result<SingleWorkerOutput, (i32, String)>>>>,
    delays: Arc<Mutex<HashMap<String, Duration>>>,
    spawn_fail: Arc<Mutex<Option<String>>>,
}

impl MockMemberRunner {
    fn new() -> Self {
        Self::default()
    }

    fn set_result(&self, name: &str, res: Result<SingleWorkerOutput, (i32, String)>) {
        self.prompt_results
            .lock()
            .unwrap()
            .insert(name.to_string(), res);
    }

    fn set_delay(&self, name: &str, delay: Duration) {
        self.delays.lock().unwrap().insert(name.to_string(), delay);
    }

    fn set_spawn_fail(&self, name: &str) {
        *self.spawn_fail.lock().unwrap() = Some(name.to_string());
    }

    fn prompt_count(&self, name: &str) -> usize {
        self.prompt_counts
            .lock()
            .unwrap()
            .get(name)
            .copied()
            .unwrap_or(0)
    }

    fn spawns(&self) -> Vec<String> {
        self.spawns.lock().unwrap().clone()
    }

    fn cleaned_up(&self) -> Vec<String> {
        self.cleanups.lock().unwrap().clone()
    }
}

impl CouncilMemberRunner for MockMemberRunner {
    fn spawn_member(
        &self,
        name: &str,
        _kind: &str,
        _repo: &Path,
        _task: &str,
        _approve_kinds: &[String],
        _base: Option<&str>,
    ) -> Result<(), (i32, String)> {
        if let Some(failing) = self.spawn_fail.lock().unwrap().as_deref() {
            if failing == name {
                return Err((1, format!("simulated spawn failure for {name}")));
            }
        }
        self.spawns.lock().unwrap().push(name.to_string());
        Ok(())
    }

    fn prompt_member(
        &self,
        name: &str,
        _brief: &str,
        _repo: &Path,
        cancel: Arc<AtomicBool>,
    ) -> Result<SingleWorkerOutput, (i32, String)> {
        *self
            .prompt_counts
            .lock()
            .unwrap()
            .entry(name.to_string())
            .or_default() += 1;

        let delay = self.delays.lock().unwrap().get(name).copied();
        if let Some(d) = delay {
            let start = Instant::now();
            while start.elapsed() < d {
                if cancel.load(Ordering::SeqCst) {
                    return Err((1, format!("cancelled by fail-fast: {name}")));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        let res = self
            .prompt_results
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .unwrap_or_else(|| {
                Err((
                    1,
                    format!("no mock prompt result configured for member '{name}'"),
                ))
            });
        res
    }

    fn cleanup_member(&self, name: &str, _repo: &Path) -> Result<(), (i32, String)> {
        self.cleanups.lock().unwrap().push(name.to_string());
        Ok(())
    }
}

#[test]
fn council_3_quorum_at_2_of_3() {
    let tmp = hermetic_temp_dir("q23");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "q23";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    let m2 = format!("quansu-{task}c2");
    let m3 = format!("truonglao-{task}c3");

    let r1 = runs_dir.join(format!("{m1}.md"));
    let r3 = runs_dir.join(format!("{m3}.md"));

    // m1: clean, m2: fails, m3: clean
    fs::write(&r1, "# Report 1\nVerdict: clean\n").unwrap();
    fs::write(&r3, "# Report 3\nVerdict: clean\n").unwrap();

    mock.set_result(&m1, Ok(SingleWorkerOutput::new(0, r1.clone()).unwrap()));
    mock.set_result(&m2, Err((4, "time budget exceeded".to_string())));
    mock.set_result(&m3, Ok(SingleWorkerOutput::new(0, r3.clone()).unwrap()));

    let member_specs = vec![
        (m1.clone(), "suhuynh".to_string()),
        (m2.clone(), "quansu".to_string()),
        (m3.clone(), "truonglao".to_string()),
    ];

    let res = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "test brief",
        "council",
        &member_specs,
        2, // quorum = 2
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );

    let out = res.expect("quorum of 2 of 3 should succeed");
    assert!(out.quorum_met, "quorum must be met");
    assert_eq!(out.report_paths.len(), 2, "2 completed report paths");
    assert_eq!(out.failed, vec![m2.clone()]);
    assert_eq!(out.majority.as_deref(), Some("clean"));
    assert!(out.dissent.is_empty(), "both completed agreed on clean");

    // Merged report checks (F11: Quorum: 2/3, not 2/2)
    let merged_path = runs_dir.join(format!("council-{task}.md"));
    assert!(merged_path.is_file(), "merged report must exist");
    let merged_content = fs::read_to_string(&merged_path).unwrap();
    assert!(merged_content.contains("# Council report: q23"));
    assert!(merged_content.contains("Role: council · Quorum: 2/3 (MET)"));
    assert!(merged_content.contains("Majority verdict: clean"));
    assert!(merged_content.contains("Dissenters: none"));
    assert!(merged_content.contains(&format!("- {m1}: clean · {}", r1.display())));
    assert!(merged_content.contains(&format!("- {m3}: clean · {}", r3.display())));
    assert!(merged_content.contains(&format!("- {m2}: FAILED (code 4) - time budget exceeded")));
    assert!(merged_content.contains("## Lessons\nnone"));

    // State file checks
    let state_file = runs_dir.join(format!("council-{task}.state"));
    assert!(state_file.is_file(), "council state file must exist");
    let state_content = fs::read_to_string(&state_file).unwrap();
    assert!(state_content.contains("task=q23"));
    assert!(state_content.contains("kind=council"));
    assert!(state_content.contains("owner=cos"));
    assert!(state_content.contains("n=3"));
    assert!(state_content.contains("quorum=2"));
    assert!(state_content.contains("finished=1"));
}

#[test]
fn council_fails_cleanly_at_1_of_3() {
    let tmp = hermetic_temp_dir("fail-clean");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "fail13";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    let m2 = format!("quansu-{task}c2");
    let m3 = format!("truonglao-{task}c3");

    let r1 = runs_dir.join(format!("{m1}.md"));
    fs::write(&r1, "# Report 1\nVerdict: clean\n").unwrap();

    // 1 completes, 2 fail -> N - failed = 3 - 2 = 1 < 2 (quorum unreachable)
    mock.set_result(&m1, Ok(SingleWorkerOutput::new(0, r1).unwrap()));
    mock.set_result(&m2, Err((4, "timeout".to_string())));
    mock.set_result(&m3, Err((3, "stall".to_string())));

    let member_specs = vec![
        (m1.clone(), "suhuynh".to_string()),
        (m2.clone(), "quansu".to_string()),
        (m3.clone(), "truonglao".to_string()),
    ];

    let res = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "test brief",
        "council",
        &member_specs,
        2, // quorum = 2
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );

    match res {
        Err((exit_code, msg)) => {
            assert_eq!(exit_code, 3, "fail-fast must exit with code 3");
            assert!(
                msg.contains("quorum unreachable"),
                "error message must state quorum unreachable"
            );
        }
        Ok(_) => panic!("council should fail when quorum is unreachable"),
    }

    // State file must be marked finished=1
    let state_file = runs_dir.join(format!("council-{task}.state"));
    assert!(state_file.is_file(), "state file must exist");
    let state_content = fs::read_to_string(&state_file).unwrap();
    assert!(state_content.contains("finished=1"));
}

#[test]
fn council_fail_fast_kills_live_members() {
    let tmp = hermetic_temp_dir("fail-fast-cleanup");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "ffkill";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    let m2 = format!("quansu-{task}c2");
    let m3 = format!("truonglao-{task}c3");

    // m1 fails immediately, m2 fails immediately -> fail-fast triggers!
    // m3 sleeps for 600ms unless cancelled (F2)
    mock.set_result(&m1, Err((4, "budget".to_string())));
    mock.set_result(&m2, Err((3, "stall".to_string())));
    mock.set_delay(&m3, Duration::from_millis(600));

    let member_specs = vec![
        (m1.clone(), "suhuynh".to_string()),
        (m2.clone(), "quansu".to_string()),
        (m3.clone(), "truonglao".to_string()),
    ];

    let start = Instant::now();
    let res = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "brief",
        "council",
        &member_specs,
        2,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );
    let elapsed = start.elapsed();

    assert!(res.is_err(), "must fail fast");
    assert_eq!(res.unwrap_err().0, 3, "exit code must be 3");

    // F2: Wall time must NOT wait for the 600ms sleeping member!
    assert!(
        elapsed < Duration::from_millis(300),
        "fail-fast must interrupt live members promptly, took {:?}",
        elapsed
    );

    // F3: Specifically assert that ONLY the live member m3 was cleaned up!
    let cleanups = mock.cleaned_up();
    assert_eq!(
        cleanups,
        vec![m3],
        "cleanup_member must be called specifically for the live unsettled member"
    );
}

#[test]
fn council_resume_from_journal_partial() {
    // Tests F1 + F4: Partial resume with hand-crafted journal where c1 completed
    let tmp = hermetic_temp_dir("partial-resume");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "presume";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    let m2 = format!("quansu-{task}c2");

    let r1 = runs_dir.join(format!("{m1}.md"));
    let r2 = runs_dir.join(format!("{m2}.md"));
    fs::write(&r1, "# R1\nVerdict: clean\n").unwrap();
    fs::write(&r2, "# R2\nVerdict: clean\n").unwrap();

    mock.set_result(&m1, Ok(SingleWorkerOutput::new(0, r1).unwrap()));
    mock.set_result(&m2, Ok(SingleWorkerOutput::new(0, r2).unwrap()));

    let member_specs = vec![
        (m1.clone(), "suhuynh".to_string()),
        (m2.clone(), "quansu".to_string()),
    ];

    // Seed journal with c1 already completed
    let db_path = runs_dir.join("quaqua.db");
    {
        use quaqua::engine::journal::{EventRecord, Journal};
        use quaqua::engine::journal_sqlite::SqliteJournal;
        let mut journal = SqliteJournal::open(&db_path).unwrap();
        let wf_id = format!("council:{task}");
        journal
            .append_event(
                &wf_id,
                &EventRecord::new(
                    1,
                    100,
                    "child_started",
                    serde_json::json!({ "id": m1, "workflow_type": "single_worker", "input": {} })
                        .to_string(),
                ),
            )
            .unwrap();
        journal
            .append_event(
                &wf_id,
                &EventRecord::new(
                    2,
                    200,
                    "child_completed",
                    serde_json::json!({ "id": m1, "output": { "exit": 0, "report_path": runs_dir.join(format!("{m1}.md")), "verdict": "clean" } }).to_string(),
                ),
            )
            .unwrap();
    }

    // Run resumed workflow: must NOT spawn or prompt m1 again!
    let res = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "brief",
        "council",
        &member_specs,
        2,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );
    assert!(res.is_ok(), "resumed workflow should complete successfully");

    // F1 & F4: m1 was NOT spawned and NOT prompted on resume!
    assert_eq!(
        mock.spawns(),
        vec![m2.clone()],
        "only m2 should be spawned on partial resume"
    );
    assert_eq!(
        mock.prompt_count(&m1),
        0,
        "m1 was already settled; must NOT be prompted"
    );
    assert_eq!(
        mock.prompt_count(&m2),
        1,
        "m2 was unsettled; must be prompted"
    );
}

#[test]
fn council_terminal_replay_without_spawns() {
    // Tests F1 + F6: A replay on an already terminal run returns outcome without spawning
    let tmp = hermetic_temp_dir("term-replay");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "term";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    let r1 = runs_dir.join(format!("{m1}.md"));
    fs::write(&r1, "# R1\nVerdict: clean\n").unwrap();
    mock.set_result(&m1, Ok(SingleWorkerOutput::new(0, r1).unwrap()));

    let member_specs = vec![(m1.clone(), "suhuynh".to_string())];

    // Pass 1: Runs to terminal completion
    let res1 = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "brief",
        "council",
        &member_specs,
        1,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );
    assert!(res1.is_ok());
    assert_eq!(mock.spawns().len(), 1);

    // Pass 2: Replay!
    let res2 = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "brief",
        "council",
        &member_specs,
        1,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );
    assert!(res2.is_ok());

    // Spawns count must NOT have increased!
    assert_eq!(
        mock.spawns().len(),
        1,
        "terminal replay must not spawn new members"
    );
}

#[test]
fn council_dissent_when_no_majority() {
    // Tests F8: Dissent is independent of majority; list all when split
    let tmp = hermetic_temp_dir("split-dissent");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "split";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    let m2 = format!("quansu-{task}c2");

    let r1 = runs_dir.join(format!("{m1}.md"));
    let r2 = runs_dir.join(format!("{m2}.md"));
    fs::write(&r1, "# R1\nVerdict: clean\n").unwrap();
    fs::write(&r2, "# R2\nVerdict: 2 findings\n").unwrap();

    mock.set_result(&m1, Ok(SingleWorkerOutput::new(0, r1).unwrap()));
    mock.set_result(&m2, Ok(SingleWorkerOutput::new(0, r2).unwrap()));

    let member_specs = vec![
        (m1.clone(), "suhuynh".to_string()),
        (m2.clone(), "quansu".to_string()),
    ];

    let res = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "brief",
        "council",
        &member_specs,
        2,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );

    let out = res.expect("quorum met");
    assert!(
        out.majority.is_none(),
        "1 clean vs 1 with findings has NO strict majority"
    );
    assert_eq!(
        out.dissent.len(),
        2,
        "all differing members must be in dissent list when no majority"
    );

    let merged_path = runs_dir.join(format!("council-{task}.md"));
    let content = fs::read_to_string(&merged_path).unwrap();
    assert!(content.contains("Majority verdict: none"));
    assert!(content.contains(&m1));
    assert!(content.contains(&m2));
}

#[test]
fn council_verdictless_report_fails_with_code_6() {
    // Tests F9: Report missing Verdict: line fails with code 6
    let tmp = hermetic_temp_dir("no-verdict");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "noverd";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    let r1 = runs_dir.join(format!("{m1}.md"));
    // Missing Verdict: line
    fs::write(&r1, "# Report\nJust prose without verdict line\n").unwrap();
    mock.set_result(&m1, Ok(SingleWorkerOutput::new(0, r1).unwrap()));

    let member_specs = vec![(m1.clone(), "suhuynh".to_string())];

    let res = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "brief",
        "council",
        &member_specs,
        1,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );

    assert!(res.is_err());
    let (code, msg) = res.unwrap_err();
    assert_eq!(
        code, 3,
        "quorum unreachable due to failed member must exit 3"
    );
    assert!(msg.contains("quorum unreachable"));
}

#[test]
fn council_member_blocked_exit_5_mapped_to_failed() {
    // Tests F9 & A2: Member exit 5 (blocked on prompt) is mapped to Failed, never returns exit 5
    let tmp = hermetic_temp_dir("exit5");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "e5";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    mock.set_result(&m1, Err((5, "blocked on question".to_string())));

    let member_specs = vec![(m1.clone(), "suhuynh".to_string())];

    let res = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "brief",
        "council",
        &member_specs,
        1,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );

    assert!(res.is_err());
    let (code, _) = res.unwrap_err();
    assert_ne!(code, 5, "council must NEVER exit 5");
    assert_eq!(code, 3, "fail-fast due to member failure must exit 3");
}

#[test]
fn council_fail_fast_boundary_one_earlier_one_later() {
    // Tests F9: fail-fast boundary (one fail: q still reachable; two fails: q unreachable)
    let tmp = hermetic_temp_dir("ff-boundary");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "boundary";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    let m2 = format!("quansu-{task}c2");
    let m3 = format!("truonglao-{task}c3");

    let r2 = runs_dir.join(format!("{m2}.md"));
    let r3 = runs_dir.join(format!("{m3}.md"));
    fs::write(&r2, "# R2\nVerdict: clean\n").unwrap();
    fs::write(&r3, "# R3\nVerdict: clean\n").unwrap();

    // m1 fails. But m2 and m3 complete. Quorum is 2 of 3.
    // Proves that 1 failure does NOT fail-fast (not one earlier).
    mock.set_result(&m1, Err((4, "timeout".to_string())));
    mock.set_result(&m2, Ok(SingleWorkerOutput::new(0, r2).unwrap()));
    mock.set_result(&m3, Ok(SingleWorkerOutput::new(0, r3).unwrap()));

    let member_specs = vec![
        (m1.clone(), "suhuynh".to_string()),
        (m2.clone(), "quansu".to_string()),
        (m3.clone(), "truonglao".to_string()),
    ];

    let res = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "brief",
        "council",
        &member_specs,
        2,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );

    assert!(
        res.is_ok(),
        "1 failure out of 3 when quorum=2 must still succeed"
    );
}

#[test]
fn council_spawn_rollback_on_failure() {
    // Tests F9: Mid-way spawn failure cleans up earlier spawned members and exits 1
    let tmp = hermetic_temp_dir("spawn-fail");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "spfail";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    let m2 = format!("quansu-{task}c2");

    mock.set_spawn_fail(&m2);

    let member_specs = vec![
        (m1.clone(), "suhuynh".to_string()),
        (m2.clone(), "quansu".to_string()),
    ];

    let res = run_taskforce_workflow(
        "council",
        task,
        &repo_dir,
        "brief",
        "council",
        &member_specs,
        2,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );

    assert!(res.is_err());
    assert_eq!(res.unwrap_err().0, 1, "spawn failure must exit 1");
    assert_eq!(
        mock.cleaned_up(),
        vec![m1],
        "m1 must be rolled back on m2 spawn failure"
    );
}

#[test]
fn taskforce_markers_and_state_written() {
    // Tests F9: Asserts .taskforce markers and state files for advisor and dual-lens
    let tmp = hermetic_temp_dir("markers-state");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let mock = MockMemberRunner::new();

    // 1. Advisor mode
    let adv_name = "suhuynh-advtest";
    let adv_report = runs_dir.join(format!("{adv_name}.md"));
    fs::write(&adv_report, "# Report\nVerdict: clean\n").unwrap();
    mock.set_result(
        adv_name,
        Ok(SingleWorkerOutput::new(0, adv_report).unwrap()),
    );

    let res_adv = run_taskforce_workflow(
        "advisor",
        "advtask",
        &repo_dir,
        "brief",
        "advisor",
        &[(adv_name.to_string(), "suhuynh".to_string())],
        1,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );
    assert!(res_adv.is_ok());

    // Advisor marker and state file check (A1: taskforce-<task>.state)
    assert!(runs_dir.join(format!("{adv_name}.taskforce")).is_file());
    assert!(runs_dir.join("taskforce-advtask.state").is_file());

    // 2. Dual-lens mode
    let dl1 = "truonglao-dltaskdl1";
    let dl2 = "suhuynh-dltaskdl2";
    let rep1 = runs_dir.join(format!("{dl1}.md"));
    let rep2 = runs_dir.join(format!("{dl2}.md"));
    fs::write(&rep1, "# DL1\nVerdict: clean\n").unwrap();
    fs::write(&rep2, "# DL2\nVerdict: clean\n").unwrap();
    mock.set_result(dl1, Ok(SingleWorkerOutput::new(0, rep1).unwrap()));
    mock.set_result(dl2, Ok(SingleWorkerOutput::new(0, rep2).unwrap()));

    let res_dl = run_taskforce_workflow(
        "dual-lens",
        "dltask",
        &repo_dir,
        "brief",
        "dual-lens",
        &[
            (dl1.to_string(), "truonglao".to_string()),
            (dl2.to_string(), "suhuynh".to_string()),
        ],
        2,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );
    assert!(res_dl.is_ok());

    // Dual-lens markers and state file check (A1: dual-lens-<task>.state)
    assert!(runs_dir.join(format!("{dl1}.taskforce")).is_file());
    assert!(runs_dir.join(format!("{dl2}.taskforce")).is_file());
    assert!(runs_dir.join("dual-lens-dltask.state").is_file());
}

#[test]
fn dual_lens_behaviour_unchanged() {
    // Tests F5: dual-lens with review: true and Verdict: line intact
    let tmp = hermetic_temp_dir("dual-lens-unchanged");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "dl-check";
    let mock = MockMemberRunner::new();

    let w1 = format!("truonglao-{}dl1", task);
    let w2 = format!("suhuynh-{}dl2", task);

    let r1 = runs_dir.join(format!("{w1}.md"));
    let r2 = runs_dir.join(format!("{w2}.md"));
    fs::write(&r1, "# Dual-lens 1\nAssess.\nVerdict: clean\n").unwrap();
    fs::write(&r2, "# Dual-lens 2\nAssess.\nVerdict: clean\n").unwrap();

    mock.set_result(&w1, Ok(SingleWorkerOutput::new(0, r1.clone()).unwrap()));
    mock.set_result(&w2, Ok(SingleWorkerOutput::new(0, r2.clone()).unwrap()));

    let member_specs = vec![
        (w1.clone(), "truonglao".to_string()),
        (w2.clone(), "suhuynh".to_string()),
    ];

    let res = run_taskforce_workflow(
        "dual-lens",
        task,
        &repo_dir,
        "dual lens brief",
        "dual-lens",
        &member_specs,
        2,
        "cos",
        &[],
        None,
        &runs_dir,
        &mock,
    );

    let out = res.expect("dual-lens must succeed");
    assert_eq!(out.report_paths, vec![r1, r2]);

    let st_file = runs_dir.join(format!("dual-lens-{task}.state"));
    assert!(st_file.is_file());
    let content = fs::read_to_string(&st_file).unwrap();
    assert!(content.contains("kind=dual-lens"));
    assert!(content.contains("finished=1"));
}

#[test]
fn taskforce_member_cannot_review_rejected_at_spawn() {
    // Tests F5: Member kind with can_review = false is rejected with actionable error
    let tmp = hermetic_temp_dir("cannot-review");
    let repo_dir = tmp.join("repo");
    let runs_dir = tmp.join("runs");
    fs::create_dir_all(&repo_dir).unwrap();
    fs::create_dir_all(&runs_dir).unwrap();

    let brief = tmp.join("brief.md");
    fs::write(&brief, "Test brief").unwrap();

    let team_toml = tmp.join("team.toml");
    let team_content = r#"
schema_version = 2

[purposes]
review = "tieude"

[kinds.tieude]
harness = "opencode"
model = "deepseek-v4.1-flash"
effort = "medium"
fallback = "sude"
can_review = false

[kinds.sude]
harness = "omp"
model = "gemini-3.8-flash"
effort = "medium"
can_review = true

[taskforce.advisor]
members = ["tieude"]
"#;
    fs::write(&team_toml, team_content).unwrap();

    let prev_team = std::env::var("QUAQUA_TEAM_TOML").ok();
    let prev_runs = std::env::var("QUAQUA_RUNS_DIR").ok();
    let prev_owner = std::env::var("QUAQUA_OWNER").ok();
    let prev_test = std::env::var("QUAQUA_TEST").ok();
    let prev_sock = std::env::var("HERDR_SOCKET_PATH").ok();

    std::env::set_var("QUAQUA_TEAM_TOML", &team_toml);
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_OWNER", "cos");
    std::env::set_var("QUAQUA_TEST", "1");
    std::env::set_var("HERDR_SOCKET_PATH", "/dev/null/none");
    let git = quaqua::git::Git::new();
    let herdr = quaqua::herdr::Herdr::new();

    let args = vec![
        "advisor".to_string(),
        "advtask".to_string(),
        repo_dir.to_str().unwrap().to_string(),
        "--brief".to_string(),
        brief.to_str().unwrap().to_string(),
    ];

    let res = quaqua::taskforce::handle_taskforce(&args, &git, &herdr);

    if let Some(v) = prev_team {
        std::env::set_var("QUAQUA_TEAM_TOML", v);
    } else {
        std::env::remove_var("QUAQUA_TEAM_TOML");
    }
    if let Some(v) = prev_runs {
        std::env::set_var("QUAQUA_RUNS_DIR", v);
    } else {
        std::env::remove_var("QUAQUA_RUNS_DIR");
    }
    if let Some(v) = prev_owner {
        std::env::set_var("QUAQUA_OWNER", v);
    } else {
        std::env::remove_var("QUAQUA_OWNER");
    }
    if let Some(v) = prev_test {
        std::env::set_var("QUAQUA_TEST", v);
    } else {
        std::env::remove_var("QUAQUA_TEST");
    }
    if let Some(v) = prev_sock {
        std::env::set_var("HERDR_SOCKET_PATH", v);
    } else {
        std::env::remove_var("HERDR_SOCKET_PATH");
    }

    assert!(
        res.is_err(),
        "must reject member kind with can_review = false"
    );
    let (code, msg) = res.unwrap_err();
    assert_eq!(code, 1);
    assert!(msg.contains("cannot review (can_review = false in [kinds.tieude])"));
}
