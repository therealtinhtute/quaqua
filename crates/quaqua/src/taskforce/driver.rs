//! Concurrent child driver and lifecycle execution for taskforce workflows.

use super::report::{parse_member_verdict, write_council_report};
use super::workflow::{
    build_taskforce_output, MemberStatus, TaskforceInput, TaskforceOutput, TaskforceState,
    TaskforceWorkflow,
};
use crate::engine::activity::{Activity, ActivityError, ActivityRequest, ActivityResult};
use crate::engine::child::{SingleWorkerInput, SingleWorkerOutput};
use crate::engine::clock::RealClock;
use crate::engine::journal::{EventRecord, Journal, SnapshotRecord};
use crate::engine::journal_sqlite::SqliteJournal;
use crate::engine::runner::{self, RunnerOptions, RunnerOutcome};
use crate::engine::workflow::WorkflowEvent;
use crate::state::WorkerMetadata;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

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
        cancel: Arc<AtomicBool>,
    ) -> Result<SingleWorkerOutput, (i32, String)>;

    fn cleanup_member(&self, name: &str, repo: &Path) -> Result<(), (i32, String)>;
}

/// RAII cleanup guard that ensures any spawned members on error paths are cleaned up (F1 residual).
struct TaskforceSpawnGuard<'a, R: CouncilMemberRunner> {
    runner: &'a R,
    repo: &'a Path,
    spawned: Vec<String>,
    success: bool,
}

impl<'a, R: CouncilMemberRunner> Drop for TaskforceSpawnGuard<'a, R> {
    fn drop(&mut self) {
        if !self.success {
            for name in &self.spawned {
                let _ = self.runner.cleanup_member(name, self.repo);
            }
        }
    }
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
            review: true, // Taskforce members are reviewers (F5)
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
        _cancel: Arc<AtomicBool>,
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

/// Build member prompt text for a given taskforce mode (F5).
pub fn build_member_prompt(mode: &str, task: &str, worker_name: &str, brief: &str) -> String {
    match mode {
        "advisor" => format!(
            "Goal: advisor recommendation for task '{task}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {brief}\nEvidence: recommendation + confidence, evidence, alternatives, strongest downside, and the smallest check that would prove it wrong (marked not run). Write to a temp file and rename it to ~/quaqua/runs/{worker_name}.md — publish atomically, never a partial file."
        ),
        "dual-lens" => format!(
            "Goal: dual-lens assessment for task '{task}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {brief}\nEvidence: assessment, agreements, and concerns. Write to a temp file and rename it to ~/quaqua/runs/{worker_name}.md — publish atomically, never a partial file."
        ),
        _ => format!(
            "Goal: council review assessment for task '{task}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {brief}\nEvidence: assessment, agreements, and concerns. End with exactly one 'Verdict: clean | <N> findings' line. Write to a temp file and rename it to ~/quaqua/runs/{worker_name}.md — publish atomically, never a partial file."
        ),
    }
}

/// Reconstruct taskforce state from loaded journal events.
fn rebuild_state_from_events(
    initial_state: &TaskforceState,
    events: &[EventRecord],
    runs_dir: &Path,
) -> TaskforceState {
    let mut st = initial_state.clone();
    for rec in events {
        if rec.kind == "child_completed" {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&rec.payload_json) {
                if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                    if let Some(m) = st.members.iter_mut().find(|slot| slot.id == id) {
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
                    if let Some(m) = st.members.iter_mut().find(|slot| slot.id == id) {
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
    st
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

    let workflow = TaskforceWorkflow;
    let workflow_id = format!("{mode}:{task}");
    let db_path = runs_dir.join("quaqua.db");
    let mut journal =
        SqliteJournal::open(&db_path).map_err(|e| (1, format!("sqlite journal error: {e}\n")))?;
    let mut noop_activity = NoopActivity;
    let mut clock = RealClock::new();

    // Disable snapshot_at_blocked to prevent UNIQUE constraint duplicate seq errors on resume (F1)
    let runner_opts = RunnerOptions {
        max_steps: 10_000,
        snapshot_interval: 20,
        snapshot_at_blocked: false,
    };

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

    // 1. Inspect journal BEFORE spawning (F1, F6)
    let past_events = journal.load_events(&workflow_id, 1).unwrap_or_default();

    let mut already_completed: Option<TaskforceOutput> = None;
    let mut already_failed: Option<(i32, String)> = None;
    let mut settled_members: BTreeSet<String> = BTreeSet::new();

    for rec in &past_events {
        if rec.kind == "workflow_completed" {
            if let Ok(out) = serde_json::from_str::<TaskforceOutput>(&rec.payload_json) {
                already_completed = Some(out);
            }
        } else if rec.kind == "workflow_failed" {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&rec.payload_json) {
                let code = val.get("code").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
                let reason = val
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("failed")
                    .to_string();
                already_failed = Some((code, reason));
            }
        } else if rec.kind == "child_completed" || rec.kind == "child_failed" {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&rec.payload_json) {
                if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                    settled_members.insert(id.to_string());
                }
            }
        }
    }

    // Authoritative journal terminal check BEFORE spawn loop (F6, F7):
    // If the journal already recorded completion or failure, return it immediately without spawning!
    if let Some(mut out) = already_completed {
        if mode == "council" {
            let merged_path = runs_dir.join(format!("council-{task}.md"));
            if !merged_path.is_file() {
                // Idempotent report generation if missing on terminal replay (F7)
                let final_state = rebuild_state_from_events(&initial_state, &past_events, runs_dir);
                let written_path =
                    write_council_report(runs_dir, task, role_name, &final_state, &out)
                        .map_err(|e| (1, format!("cannot write merged report: {e}\n")))?;
                out.merged_report = Some(written_path);
            } else {
                out.merged_report = Some(merged_path);
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
        return Ok(out);
    }
    if let Some((code, reason)) = already_failed {
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

    // 2. Initialize RAII spawn guard to prevent leaks on any error path (F1 residual)
    let mut guard = TaskforceSpawnGuard {
        runner,
        repo,
        spawned: Vec::new(),
        success: false,
    };

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

    // Spawn ONLY unsettled members
    for (name, kind) in member_specs {
        if settled_members.contains(name) {
            continue;
        }
        let spawn_res = runner.spawn_member(name, kind, repo, task, approve_kinds, base);
        if let Err(e) = spawn_res {
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
            return Err(e); // guard cleans up already-spawned members on drop
        }
        guard.spawned.push(name.clone());
        let _ = WorkerMetadata::save_taskforce(runs_dir, name, task);
    }

    // 3. Setup TaskforceWorkflow and seed initial snapshot
    if journal
        .load_latest_snapshot(&workflow_id)
        .map(|s| s.is_none())
        .unwrap_or(true)
    {
        let snap = SnapshotRecord::new(
            0,
            serde_json::to_string(&initial_state)
                .map_err(|e| (1, format!("cannot serialize initial state snapshot: {e}\n")))?,
        );
        let _ = journal.append_snapshot(&workflow_id, &snap);
    }

    let is_resumed = !past_events.is_empty();
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

    match first_outcome {
        RunnerOutcome::Completed(mut out) => {
            if mode == "council" {
                let merged_path = runs_dir.join(format!("council-{task}.md"));
                if !merged_path.is_file() {
                    let final_state =
                        rebuild_state_from_events(&initial_state, &past_events, runs_dir);
                    let written_path =
                        write_council_report(runs_dir, task, role_name, &final_state, &out)
                            .map_err(|e| (1, format!("cannot write merged report: {e}\n")))?;
                    out.merged_report = Some(written_path);
                } else {
                    out.merged_report = Some(merged_path);
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
            guard.success = true;
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

    // 4. Concurrently prompt pending members with real-time fail-fast cancellation (F2, F3)
    let pending_members: Vec<(String, String)> = member_specs
        .iter()
        .filter(|(name, _)| !settled_members.contains(name))
        .cloned()
        .collect();

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let mut fail_fast_error: Option<(i32, String)> = None;

    std::thread::scope(|s| {
        for (name, _) in &pending_members {
            let tx_c = tx.clone();
            let name_c = name.clone();
            let brief_text = build_member_prompt(mode, task, name, brief);
            let cancel_c = cancel_flag.clone();
            s.spawn(move || {
                let res = runner.prompt_member(&name_c, &brief_text, repo, cancel_c);
                let _ = tx_c.send((name_c, res));
            });
        }
        drop(tx);

        while let Ok((name, prompt_res)) = rx.recv() {
            settled_members.insert(name.clone());
            guard.spawned.retain(|n| n != &name);

            let event = match prompt_res {
                Ok(out) => {
                    let report_content =
                        std::fs::read_to_string(&out.report_path).unwrap_or_default();
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
                                // Member report missing/unparseable Verdict -> Failed{6} (F9)
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
            .map_err(|e| (1, format!("runner resume error: {e}\n")));

            match step_res {
                Ok(RunnerOutcome::Failed { code, reason }) => {
                    fail_fast_error = Some((code, reason));
                    // Cancel live threads and clean up live unsettled members immediately (F2, F3)
                    cancel_flag.store(true, Ordering::SeqCst);
                    for (m_name, _) in member_specs {
                        if !settled_members.contains(m_name) {
                            let _ = runner.cleanup_member(m_name, repo);
                            guard.spawned.retain(|n| n != m_name);
                        }
                    }
                    break;
                }
                Err(err) => {
                    fail_fast_error = Some(err);
                    cancel_flag.store(true, Ordering::SeqCst);
                    for (m_name, _) in member_specs {
                        if !settled_members.contains(m_name) {
                            let _ = runner.cleanup_member(m_name, repo);
                            guard.spawned.retain(|n| n != m_name);
                        }
                    }
                    break;
                }
                Ok(RunnerOutcome::Completed(_)) | Ok(RunnerOutcome::Blocked(_)) => {}
            }
        }
    });

    // 5. Handle fail-fast outcome
    if let Some((code, reason)) = fail_fast_error {
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

    // 6. Reconstruct final state from journal events (F7)
    let final_events = journal.load_events(&workflow_id, 1).unwrap_or_default();
    let final_state = rebuild_state_from_events(&initial_state, &final_events, runs_dir);
    let mut final_out = build_taskforce_output(&final_state);

    // 7. Write merged report BEFORE final state file completion (F7)
    if mode == "council" {
        let merged_path = write_council_report(runs_dir, task, role_name, &final_state, &final_out)
            .map_err(|e| (1, format!("cannot write merged report: {e}\n")))?;
        final_out.merged_report = Some(merged_path);
    }

    // Mark state finished only after the merged report is on disk (F7)
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

    guard.success = true;
    Ok(final_out)
}
