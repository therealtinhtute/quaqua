//! Concurrent child driver and lifecycle execution for taskforce workflows.

use super::report::{parse_member_verdict, write_council_report};
use super::workflow::{
    build_taskforce_output, MemberStatus, TaskforceInput, TaskforceOutput, TaskforceState,
    TaskforceWorkflow,
};
use crate::engine::activity::{Activity, ActivityError, ActivityRequest, ActivityResult};
use crate::engine::child::{SingleWorkerInput, SingleWorkerOutput};
use crate::engine::clock::RealClock;
use crate::engine::journal::Journal;
use crate::engine::journal_sqlite::SqliteJournal;
use crate::engine::runner::{self, RunnerOptions, RunnerOutcome};
use crate::engine::workflow::WorkflowEvent;
use crate::state::WorkerMetadata;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

struct NoopActivity;

impl Activity for NoopActivity {
    fn execute(&self, _req: &ActivityRequest) -> Result<ActivityResult, ActivityError> {
        Err(ActivityError::ExecutionFailed(
            "no activities scheduled in taskforce workflow".to_string(),
        ))
    }
}

/// Abstract runner for taskforce child members (spawn, prompt, cleanup).
pub trait CouncilMemberRunner: Send + Sync {
    fn spawn_member(
        &self,
        name: &str,
        kind: &str,
        repo: &Path,
        task: &str,
        approve_kinds: &[String],
        base: Option<&str>,
    ) -> Result<(), (i32, String)>;

    fn prompt_member(
        &self,
        name: &str,
        brief: &str,
        repo: &Path,
    ) -> Result<SingleWorkerOutput, (i32, String)>;

    fn cleanup_member(&self, name: &str, repo: &Path) -> Result<(), (i32, String)>;
}

/// Production member runner using live Quaqua worker functions.
pub struct LiveCouncilMemberRunner<'a, G: crate::git::GitOps> {
    pub git: &'a G,
    pub herdr: &'a crate::herdr::Herdr,
    pub cfg: &'a crate::config::Config,
    pub base: Option<String>,
}

impl<'a, G: crate::git::GitOps + Clone + Send + Sync + 'static> CouncilMemberRunner
    for LiveCouncilMemberRunner<'a, G>
{
    fn spawn_member(
        &self,
        name: &str,
        kind: &str,
        repo: &Path,
        task: &str,
        approve_kinds: &[String],
        base: Option<&str>,
    ) -> Result<(), (i32, String)> {
        let opts = crate::worker::spawn::SpawnOptions {
            base: base.map(|s| s.to_string()).or_else(|| self.base.clone()),
            review: false,
            name: name.to_string(),
            kind: Some(kind.to_string()),
            purpose: None,
            repo_path: repo.to_path_buf(),
            approve_kinds: approve_kinds.to_vec(),
            solo: None,
        };

        std::env::set_var("PIPELINE_TASK", task);
        let res = crate::worker::spawn::spawn_worker(
            opts,
            self.cfg,
            self.git,
            self.herdr,
            crate::layout::SpawnOrigin::TaskforceMember,
        );
        std::env::remove_var("PIPELINE_TASK");
        res
    }

    fn prompt_member(
        &self,
        name: &str,
        brief: &str,
        _repo: &Path,
    ) -> Result<SingleWorkerOutput, (i32, String)> {
        let prompt_res = crate::worker::prompt::prompt_worker(
            name,
            Some(brief),
            false,
            None,
            self.git,
            self.herdr,
        );
        let runs_dir = crate::worker::cleanup::default_runs_dir();
        let report_path = runs_dir.join(format!("{name}.md"));

        match prompt_res {
            Ok(()) => Ok(SingleWorkerOutput::new(0, report_path).unwrap()),
            Err((code, msg)) => Err((code, msg)),
        }
    }

    fn cleanup_member(&self, name: &str, repo: &Path) -> Result<(), (i32, String)> {
        crate::worker::cleanup::cleanup_worker(name, repo, self.git, self.herdr)
    }
}

/// Compute state file path according to mode (A1).
pub fn state_file_path(runs_dir: &Path, mode: &str, task: &str) -> PathBuf {
    match mode {
        "council" => runs_dir.join(format!("council-{task}.state")),
        "dual-lens" => runs_dir.join(format!("dual-lens-{task}.state")),
        _ => runs_dir.join(format!("taskforce-{task}.state")),
    }
}

/// Write state file content matching legacy formats and new council fields (A1).
pub fn write_state_file(
    path: &Path,
    mode: &str,
    task: &str,
    owner: &str,
    members: &[String],
    n: usize,
    quorum: usize,
    finished: bool,
    pid: u32,
) -> std::io::Result<()> {
    let finished_val = if finished { 1 } else { 0 };
    let members_csv = members.join(",");

    let content = match mode {
        "council" => format!(
            "task={task}\nkind=council\nowner={owner}\nmembers={members_csv}\nn={n}\nquorum={quorum}\nfinished={finished_val}\ntaskforce_pid={pid}\n"
        ),
        "dual-lens" => format!(
            "task={task}\nkind=dual-lens\nowner={owner}\nmembers={members_csv}\nfinished={finished_val}\ntaskforce_pid={pid}\n"
        ),
        _ => format!(
            "task={task}\nkind=advisor\nowner={owner}\nmembers={members_csv}\nfinished={finished_val}\ntaskforce_pid={pid}\n"
        ),
    };
    crate::state::atomic_write(path, content)
}

/// Build member prompt text for a given taskforce mode.
pub fn build_member_prompt(mode: &str, task: &str, worker_name: &str, brief: &str) -> String {
    match mode {
        "advisor" => format!(
            "Goal: advisor recommendation for task '{task}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {brief}\nEvidence: recommendation + confidence, evidence, alternatives, strongest downside, and the smallest check that would prove it wrong (marked not run). Write to a temp file and rename it to ~/quaqua/runs/{worker_name}.md — publish atomically, never a partial file."
        ),
        "dual-lens" => format!(
            "Goal: dual-lens assessment for task '{task}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {brief}\nEvidence: assessment, agreements, and concerns. Write to a temp file and rename it to ~/quaqua/runs/{worker_name}.md — publish atomically, never a partial file."
        ),
        _ => format!(
            "Goal: council review assessment for task '{task}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {brief}\nEvidence: assessment, agreements, and concerns. Conclude with exactly one 'Verdict: clean | <N> findings' line followed by '## Lessons\nnone'. Write to a temp file and rename it to ~/quaqua/runs/{worker_name}.md — publish atomically, never a partial file."
        ),
    }
}

/// Generalized driver executing advisor, dual-lens, or council workflows.
pub fn run_taskforce_workflow<R: CouncilMemberRunner>(
    mode: &str,
    task: &str,
    repo: &Path,
    brief: &str,
    role_name: &str,
    member_specs: &[(String, String)], // (name, kind)
    quorum: usize,
    chair_owner: &str,
    approve_kinds: &[String],
    base: Option<&str>,
    runs_dir: &Path,
    runner: &R,
) -> Result<TaskforceOutput, (i32, String)> {
    let pid = std::process::id();
    let n = member_specs.len();
    let member_names: Vec<String> = member_specs.iter().map(|(n, _)| n.clone()).collect();
    let st_file = state_file_path(runs_dir, mode, task);

    // Write initial state file (finished = 0)
    write_state_file(
        &st_file,
        mode,
        task,
        chair_owner,
        &member_names,
        n,
        quorum,
        false,
        pid,
    )
    .map_err(|e| (1, format!("cannot write state file: {e}\n")))?;

    let mut spawned_members: Vec<String> = Vec::new();
    for (name, kind) in member_specs {
        let spawn_res = runner.spawn_member(name, kind, repo, task, approve_kinds, base);
        if let Err(e) = spawn_res {
            // Rollback already spawned members
            for spawned in &spawned_members {
                let _ = runner.cleanup_member(spawned, repo);
            }
            let _ = write_state_file(
                &st_file,
                mode,
                task,
                chair_owner,
                &member_names,
                n,
                quorum,
                true,
                pid,
            );
            return Err(e);
        }
        spawned_members.push(name.clone());

        // Write .taskforce marker
        let _ = WorkerMetadata::save_taskforce(runs_dir, name, task);
    }

    // 2. Setup engine runner & SqliteJournal
    let workflow = TaskforceWorkflow;
    let workflow_id = format!("{mode}:{task}");
    let db_path = runs_dir.join("quaqua.db");
    let mut journal =
        SqliteJournal::open(&db_path).map_err(|e| (1, format!("sqlite journal error: {e}\n")))?;
    let mut noop_activity = NoopActivity;
    let mut clock = RealClock::new();
    let runner_opts = RunnerOptions::default();

    // Check if resuming from existing events
    let past_events = journal.load_events(&workflow_id, 1).unwrap_or_default();
    let is_resumed = !past_events.is_empty();

    let single_worker_inputs: Vec<SingleWorkerInput> = member_specs
        .iter()
        .map(|(name, kind)| {
            let prompt_text = build_member_prompt(mode, task, name, brief);
            SingleWorkerInput::new(
                name.clone(),
                kind.clone(),
                repo.to_string_lossy().to_string(),
                prompt_text,
                2700,
                None,
            )
        })
        .collect();

    let input = TaskforceInput {
        mode: mode.to_string(),
        task: task.to_string(),
        repo: repo.to_string_lossy().to_string(),
        brief: brief.to_string(),
        members: single_worker_inputs,
        quorum: Some(quorum),
        role: Some(role_name.to_string()),
        chair_owner: Some(chair_owner.to_string()),
    };
    let initial_state = TaskforceState::from_input(&input);

    let start_state = if is_resumed {
        None
    } else {
        Some(initial_state.clone())
    };

    let first_outcome = runner::run_until_blocked(
        &workflow,
        &workflow_id,
        start_state,
        &mut journal,
        &mut noop_activity,
        &mut clock,
        &runner_opts,
        vec![],
    )
    .map_err(|e| (1, format!("runner error: {e}\n")))?;

    // If already terminal upon start/resume:
    match first_outcome {
        RunnerOutcome::Completed(out) => {
            let _ = write_state_file(
                &st_file,
                mode,
                task,
                chair_owner,
                &member_names,
                n,
                quorum,
                true,
                pid,
            );
            return Ok(out);
        }
        RunnerOutcome::Failed { code, reason } => {
            let _ = write_state_file(
                &st_file,
                mode,
                task,
                chair_owner,
                &member_names,
                n,
                quorum,
                true,
                pid,
            );
            return Err((code, reason));
        }
        RunnerOutcome::Blocked(_) => {}
    }

    // 3. Identify already settled members to avoid re-prompting (A3)
    let current_events = journal.load_events(&workflow_id, 1).unwrap_or_default();
    let mut settled_members: BTreeSet<String> = BTreeSet::new();
    for rec in &current_events {
        if rec.kind == "child_completed" || rec.kind == "child_failed" {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&rec.payload_json) {
                if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                    settled_members.insert(id.to_string());
                }
            }
        }
    }

    // 4. Concurrently prompt pending members
    let pending_members: Vec<(String, String)> = member_specs
        .iter()
        .filter(|(name, _)| !settled_members.contains(name))
        .cloned()
        .collect();

    let (tx, rx) = mpsc::channel();
    std::thread::scope(|s| {
        for (name, _) in &pending_members {
            let tx_c = tx.clone();
            let name_c = name.clone();
            let brief_text = build_member_prompt(mode, task, name, brief);
            s.spawn(move || {
                let res = runner.prompt_member(&name_c, &brief_text, repo);
                let _ = tx_c.send((name_c, res));
            });
        }
    });
    drop(tx);

    let mut fail_fast_error: Option<(i32, String)> = None;
    let mut terminal_output: Option<TaskforceOutput> = None;
    while let Ok((name, prompt_res)) = rx.recv() {
        settled_members.insert(name.clone());

        let event = match prompt_res {
            Ok(out) => {
                let report_content = std::fs::read_to_string(&out.report_path).unwrap_or_default();
                if mode == "council" {
                    match parse_member_verdict(&report_content) {
                        Ok((verdict_str, findings, is_challenge)) => {
                            WorkflowEvent::ChildCompleted {
                                id: name.clone(),
                                output: serde_json::json!({
                                    "exit": 0,
                                    "report_path": out.report_path,
                                    "verdict": verdict_str,
                                    "findings": findings,
                                    "is_challenge": is_challenge,
                                }),
                            }
                        }
                        Err(err) => {
                            // Member report unparseable -> Failed{6} (D-P8-5)
                            WorkflowEvent::ChildFailed {
                                id: name.clone(),
                                code: 6,
                                reason: format!("unparseable verdict in member report: {err}"),
                            }
                        }
                    }
                } else {
                    WorkflowEvent::ChildCompleted {
                        id: name.clone(),
                        output: serde_json::to_value(&out).unwrap(),
                    }
                }
            }
            Err((code, reason)) => WorkflowEvent::ChildFailed {
                id: name.clone(),
                code,
                reason,
            },
        };

        let step_res = runner::run_until_blocked(
            &workflow,
            &workflow_id,
            None,
            &mut journal,
            &mut noop_activity,
            &mut clock,
            &runner_opts,
            vec![event],
        )
        .map_err(|e| (1, format!("runner resume error: {e}\n")))?;

        match step_res {
            RunnerOutcome::Failed { code, reason } => {
                fail_fast_error = Some((code, reason));
                break;
            }
            RunnerOutcome::Completed(out) => {
                terminal_output = Some(out);
                break;
            }
            RunnerOutcome::Blocked(_) => {
                // Continue waiting for more members
            }
        }
    }

    // 5. Handle fail-fast: kill/cleanup live members (A2)
    if let Some((code, reason)) = fail_fast_error {
        // Cleanup live members that haven't settled
        for (m_name, _) in member_specs {
            if !settled_members.contains(m_name) {
                let _ = runner.cleanup_member(m_name, repo);
            }
        }
        let _ = write_state_file(
            &st_file,
            mode,
            task,
            chair_owner,
            &member_names,
            n,
            quorum,
            true,
            pid,
        );
        return Err((code, reason));
    }

    // Reconstruct final_state from journal events to ensure accurate member statuses
    let final_events = journal.load_events(&workflow_id, 1).unwrap_or_default();
    let mut final_state = initial_state;
    for rec in &final_events {
        if rec.kind == "child_completed" {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&rec.payload_json) {
                if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                    if let Some(m) = final_state.members.iter_mut().find(|slot| slot.id == id) {
                        let rpath = data
                            .get("output")
                            .and_then(|o| o.get("report_path"))
                            .and_then(|p| p.as_str())
                            .map(PathBuf::from)
                            .unwrap_or_else(|| runs_dir.join(format!("{id}.md")));
                        let v = data
                            .get("output")
                            .and_then(|o| o.get("verdict"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let findings = data
                            .get("output")
                            .and_then(|o| o.get("findings"))
                            .and_then(|f| f.as_u64())
                            .map(|n| n as u32);
                        let is_challenge = data
                            .get("output")
                            .and_then(|o| o.get("is_challenge"))
                            .and_then(|c| c.as_bool());
                        m.status = MemberStatus::Completed {
                            report_path: rpath,
                            verdict: v,
                            findings_count: findings,
                            is_challenge,
                        };
                    }
                }
            }
        } else if rec.kind == "child_failed" {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&rec.payload_json) {
                if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                    if let Some(m) = final_state.members.iter_mut().find(|slot| slot.id == id) {
                        let code = data.get("code").and_then(|c| c.as_i64()).unwrap_or(1) as i32;
                        let reason = data
                            .get("reason")
                            .and_then(|r| r.as_str())
                            .unwrap_or("failed")
                            .to_string();
                        m.status = MemberStatus::Failed { code, reason };
                    }
                }
            }
        }
    }

    let mut final_out = terminal_output.unwrap_or_else(|| build_taskforce_output(&final_state));

    // 6. Merged report emission for council mode
    if mode == "council" {
        let merged_path = write_council_report(runs_dir, task, role_name, &final_state, &final_out)
            .map_err(|e| (1, format!("cannot write merged report: {e}\n")))?;
        final_out.merged_report = Some(merged_path);
    }
    let _ = write_state_file(
        &st_file,
        mode,
        task,
        chair_owner,
        &member_names,
        n,
        quorum,
        true,
        pid,
    );

    Ok(final_out)
}
