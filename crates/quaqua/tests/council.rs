use quaqua::engine::child::SingleWorkerOutput;
use quaqua::taskforce::driver::{run_taskforce_workflow, CouncilMemberRunner};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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

    fn prompt_count(&self, name: &str) -> usize {
        self.prompt_counts
            .lock()
            .unwrap()
            .get(name)
            .copied()
            .unwrap_or(0)
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
        self.spawns.lock().unwrap().push(name.to_string());
        Ok(())
    }

    fn prompt_member(
        &self,
        name: &str,
        _brief: &str,
        _repo: &Path,
    ) -> Result<SingleWorkerOutput, (i32, String)> {
        *self
            .prompt_counts
            .lock()
            .unwrap()
            .entry(name.to_string())
            .or_default() += 1;

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
    let r2 = runs_dir.join(format!("{m2}.md"));
    let r3 = runs_dir.join(format!("{m3}.md"));

    // m1: clean, m2: fails, m3: clean
    fs::write(&r1, "# Report 1\nVerdict: clean\n## Lessons\nnone\n").unwrap();
    fs::write(&r3, "# Report 3\nVerdict: clean\n## Lessons\nnone\n").unwrap();

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

    // Merged report checks
    let merged_path = runs_dir.join(format!("council-{task}.md"));
    assert!(merged_path.is_file(), "merged report must exist");
    let merged_content = fs::read_to_string(&merged_path).unwrap();
    assert!(merged_content.contains("# Council report: q23"));
    assert!(merged_content.contains("Role: council · Quorum: 2/2 (MET)"));
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
    fs::write(&r1, "# Report 1\nVerdict: clean\n## Lessons\nnone\n").unwrap();

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

    // m1 fails, m2 fails -> fail-fast triggers immediately!
    // m3 is still running / live when fail-fast triggers.
    mock.set_result(&m1, Err((4, "budget".to_string())));
    mock.set_result(&m2, Err((3, "stall".to_string())));
    // m3 sleeps longer or remains pending
    mock.set_result(
        &m3,
        Err((
            1,
            "should have been cleaned up before completing".to_string(),
        )),
    );

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

    assert!(res.is_err(), "must fail fast");
    assert_eq!(res.unwrap_err().0, 3, "exit code must be 3");

    // Verify cleanup was invoked for still-live members (A2)
    let cleanups = mock.cleaned_up();
    assert!(
        !cleanups.is_empty(),
        "cleanup_member must be called for live members on fail-fast"
    );
}

#[test]
fn council_resume_from_journal() {
    let tmp = hermetic_temp_dir("journal-resume");
    let runs_dir = tmp.join("runs");
    let repo_dir = tmp.join("repo");
    fs::create_dir_all(&runs_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();

    let task = "resume-test";
    let mock = MockMemberRunner::new();

    let m1 = format!("suhuynh-{task}c1");
    let m2 = format!("quansu-{task}c2");

    let r1 = runs_dir.join(format!("{m1}.md"));
    let r2 = runs_dir.join(format!("{m2}.md"));
    fs::write(&r1, "# R1\nVerdict: clean\n## Lessons\nnone\n").unwrap();
    fs::write(&r2, "# R2\nVerdict: clean\n## Lessons\nnone\n").unwrap();

    mock.set_result(&m1, Ok(SingleWorkerOutput::new(0, r1).unwrap()));
    mock.set_result(&m2, Ok(SingleWorkerOutput::new(0, r2).unwrap()));

    let member_specs = vec![
        (m1.clone(), "suhuynh".to_string()),
        (m2.clone(), "quansu".to_string()),
    ];

    // Pass 1: Run member 1 and member 2 to completion
    let res1 = run_taskforce_workflow(
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
    assert!(res1.is_ok());
    assert_eq!(mock.prompt_count(&m1), 1);
    assert_eq!(mock.prompt_count(&m2), 1);

    // Pass 2: Resume! The journal already contains completed events for council:resume-test.
    // Proves a settled member is NOT re-prompted by counting activity calls (A3).
    let res2 = run_taskforce_workflow(
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
    assert!(res2.is_ok());

    // Prompt counts must NOT have increased on resume!
    assert_eq!(
        mock.prompt_count(&m1),
        1,
        "settled member m1 must NOT be re-prompted on resume"
    );
    assert_eq!(
        mock.prompt_count(&m2),
        1,
        "settled member m2 must NOT be re-prompted on resume"
    );
}

#[test]
fn dual_lens_behaviour_unchanged() {
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
    fs::write(&r1, "# Dual-lens 1\nAssess.\n## Lessons\nnone\n").unwrap();
    fs::write(&r2, "# Dual-lens 2\nAssess.\n## Lessons\nnone\n").unwrap();

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

    // Check dual-lens state file
    let st_file = runs_dir.join(format!("dual-lens-{task}.state"));
    assert!(st_file.is_file());
    let content = fs::read_to_string(&st_file).unwrap();
    assert!(content.contains("kind=dual-lens"));
    assert!(content.contains("finished=1"));
}
