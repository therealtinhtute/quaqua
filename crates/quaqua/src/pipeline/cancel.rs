//! Pipeline cancellation (`quaqua pipeline --cancel <task> [--reason <text>]`).
//!
//! Closes an abandoned pipeline:
//! 1. Appends a terminal `workflow_cancelled` event to the parent workflow and any
//!    non-terminal child in the SQLite journal (`runs/quaqua.db`).
//! 2. Refuses if any child worker pane is still alive in herdr (`agent_status != "dead"`).
//! 3. Idempotent: repeated runs converge to the exact same state without duplicate events.
//! 4. Updates parent workflow state snapshot and state file to phase `cancelled`.

use crate::engine::journal::{EventRecord, Journal, SnapshotRecord};
use crate::engine::journal_sqlite::SqliteJournal;
use crate::herdr::HerdrOps;
use crate::pipeline::state::PipelineState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// Payload stored in the journal for `workflow_cancelled` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCancelledPayload {
    pub reason: String,
}

/// Returns true if the event kind represents a terminal lifecycle event.
pub fn is_terminal_event(kind: &str) -> bool {
    matches!(
        kind,
        "workflow_completed" | "workflow_failed" | "workflow_cancelled"
    )
}

/// Query distinct workflow IDs matching parent or children of `<task>`.
fn find_pipeline_workflows(
    conn: &rusqlite::Connection,
    task: &str,
) -> Result<(bool, Vec<String>), String> {
    let parent_id = format!("pipeline:{task}");
    let child_pattern = format!("pipeline:{task}:%");
    let has_events: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='events');",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);

    let has_snapshots: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='snapshots');",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);

    if !has_events && !has_snapshots {
        return Ok((false, Vec::new()));
    }

    let sql = match (has_events, has_snapshots) {
        (true, true) => {
            "SELECT DISTINCT workflow_id FROM (
                SELECT workflow_id FROM events WHERE workflow_id = ?1 OR workflow_id LIKE ?2
                UNION
                SELECT workflow_id FROM snapshots WHERE workflow_id = ?1 OR workflow_id LIKE ?2
            ) ORDER BY workflow_id;"
        }
        (true, false) => {
            "SELECT DISTINCT workflow_id FROM events WHERE workflow_id = ?1 OR workflow_id LIKE ?2 ORDER BY workflow_id;"
        }
        (false, true) => {
            "SELECT DISTINCT workflow_id FROM snapshots WHERE workflow_id = ?1 OR workflow_id LIKE ?2 ORDER BY workflow_id;"
        }
        (false, false) => unreachable!(),
    };

    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("db error: preparing workflow query: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params![parent_id, child_pattern], |r| {
            let id: String = r.get(0)?;
            Ok(id)
        })
        .map_err(|e| format!("db error: querying workflows: {e}"))?;

    let mut parent_found = false;
    let mut children = Vec::new();

    for r in rows {
        let wf = r.map_err(|e| format!("db error: decoding workflow row: {e}"))?;
        if wf == parent_id {
            parent_found = true;
        } else if wf.starts_with(&format!("pipeline:{task}:")) {
            children.push(wf);
        }
    }

    Ok((parent_found, children))
}

/// Collect names of all potential child workers for this pipeline.
fn collect_pipeline_workers(
    task: &str,
    runs_dir: &Path,
    journal: &SqliteJournal,
    child_wf_ids: &[String],
) -> BTreeSet<String> {
    let mut workers = BTreeSet::new();

    // 1. Check runs/*.pipeline sidecar files
    if let Ok(entries) = std::fs::read_dir(runs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(fname) = path.file_name().and_then(|n| n.to_str()) {
                if let Some(wname) = fname.strip_suffix(".pipeline") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if content.trim() == task {
                            workers.insert(wname.to_string());
                        }
                    }
                }
            }
        }
    }

    // 2. Check pipeline state file if present
    let state_file = runs_dir.join(format!("pipeline-{task}.state"));
    if let Ok(st) = PipelineState::load(&state_file) {
        if !st.builder.is_empty() {
            workers.insert(format!("{}-{}", st.builder, task));
        }
        if !st.reviewer.is_empty() {
            let rounds = st.rounds.max(5);
            for r in 1..=rounds {
                workers.insert(format!("{}-{}r{}", st.reviewer, task, r));
            }
        }
    }

    // 3. Check parent snapshot input
    let parent_id = format!("pipeline:{task}");
    if let Ok(Some(snap)) = journal.load_latest_snapshot(&parent_id) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&snap.state_json) {
            if let Some(input) = val.get("input") {
                if let Some(b) = input.get("builder").and_then(|b| b.as_str()) {
                    workers.insert(format!("{b}-{task}"));
                }
                if let Some(r) = input.get("reviewer").and_then(|r| r.as_str()) {
                    for rnd in 1..=10 {
                        workers.insert(format!("{r}-{task}r{rnd}"));
                    }
                }
            }
        }
    }

    // 4. Extract worker names from child workflow IDs and event histories
    for cid in child_wf_ids {
        if let Some(sub) = cid.strip_prefix(&format!("pipeline:{task}:")) {
            let base_child = sub.split('#').next().unwrap_or(sub);
            // Patterns: <builder>-<task>-plan, <builder>-<task>-build, <builder>-<task>-fix-r<N>, <reviewer>-<task>-review-r<N>
            if let Some(b_part) = base_child.strip_suffix("-plan") {
                workers.insert(b_part.to_string());
            } else if let Some(b_part) = base_child.strip_suffix("-build") {
                workers.insert(b_part.to_string());
            } else if let Some(idx) = base_child.find("-fix-r") {
                workers.insert(base_child[..idx].to_string());
            } else if let Some(idx) = base_child.find("-review-r") {
                let r_part = &base_child[..idx];
                let round_part = &base_child[idx + 9..];
                workers.insert(format!("{r_part}r{round_part}"));
                workers.insert(base_child.to_string());
            } else {
                workers.insert(base_child.to_string());
            }
        }

        // Search child events for AgentWait activity names
        if let Ok(evs) = journal.load_events(cid, 1) {
            for ev in evs {
                if ev.kind == "activity_scheduled" {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&ev.payload_json) {
                        if let Some(name) = val
                            .pointer("/activity/Herdr/AgentWait/name")
                            .and_then(|n| n.as_str())
                        {
                            workers.insert(name.to_string());
                        }
                    }
                }
            }
        }
    }

    workers
}

/// Cancels an abandoned pipeline.
///
/// Refuses if any child worker pane is alive in herdr.
/// Idempotent across multiple runs.
pub fn cancel_pipeline<H: HerdrOps>(
    task: &str,
    reason: Option<&str>,
    runs_dir: &Path,
    herdr: &H,
) -> Result<(), (i32, String)> {
    if task.is_empty()
        || !task
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err((
            1,
            format!("invalid task name '{task}': slug required ([a-z0-9-])\n"),
        ));
    }

    let db_path = runs_dir.join("quaqua.db");
    if !db_path.exists() {
        return Err((
            1,
            format!(
                "no database found at {} — nothing to cancel\n",
                db_path.display()
            ),
        ));
    }

    let mut journal = SqliteJournal::open(&db_path).map_err(|e| {
        (
            1,
            format!("cannot open journal at {}: {e}\n", db_path.display()),
        )
    })?;

    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| {
        (
            1,
            format!("db error: cannot open {}: {e}\n", db_path.display()),
        )
    })?;

    let (parent_exists, child_wf_ids) =
        find_pipeline_workflows(&conn, task).map_err(|e| (1, format!("{e}\n")))?;

    if !parent_exists && child_wf_ids.is_empty() {
        return Err((
            1,
            format!("no pipeline found in journal for task '{task}'\n"),
        ));
    }

    // Check herdr for live child worker panes
    let candidate_workers = collect_pipeline_workers(task, runs_dir, &journal, &child_wf_ids);
    let agent_list_raw = herdr
        .agent_list()
        .map_err(|e| (1, format!("herdr error: failed to query agent list: {e}\n")))?;

    let val: serde_json::Value = serde_json::from_str(&agent_list_raw)
        .map_err(|e| (1, format!("herdr error: invalid agent list JSON: {e}\n")))?;

    let agents_arr = val
        .pointer("/result/agents")
        .and_then(|a| a.as_array())
        .or_else(|| val.get("agents").and_then(|a| a.as_array()));

    if let Some(agents) = agents_arr {
        for agent in agents {
            let name = agent.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let status = agent
                .get("agent_status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");

            if name.is_empty() || status == "dead" {
                continue;
            }

            let is_child = candidate_workers.contains(name)
                || name.ends_with(&format!("-{task}"))
                || crate::cap::matches_pipeline_reviewer(name, task);

            if is_child {
                return Err((
                    1,
                    format!(
                        "cannot cancel pipeline '{task}': child worker '{name}' is still alive in herdr (status: {status})\n"
                    ),
                ));
            }
        }
    }

    let reason_str = reason.unwrap_or("cancelled");
    let payload = serde_json::to_string(&WorkflowCancelledPayload {
        reason: reason_str.to_string(),
    })
    .unwrap_or_else(|_| "{}".to_string());

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let parent_id = format!("pipeline:{task}");

    // 1. Cancel parent workflow if non-terminal
    if parent_exists {
        let parent_events = journal
            .load_events(&parent_id, 1)
            .map_err(|e| (1, format!("journal error loading events: {e}\n")))?;
        let is_term = parent_events.iter().any(|e| is_terminal_event(&e.kind));
        if !is_term {
            journal
                .append_event(
                    &parent_id,
                    &EventRecord::new(0, now_ms, "workflow_cancelled", &payload),
                )
                .map_err(|e| (1, format!("journal error appending event: {e}\n")))?;
        }
    }

    // 2. Cancel any non-terminal child workflows
    for child_id in &child_wf_ids {
        let child_events = journal.load_events(child_id, 1).map_err(|e| {
            (
                1,
                format!("journal error loading events for {child_id}: {e}\n"),
            )
        })?;
        let is_term = child_events.iter().any(|e| is_terminal_event(&e.kind));
        if !is_term {
            journal
                .append_event(
                    child_id,
                    &EventRecord::new(0, now_ms, "workflow_cancelled", &payload),
                )
                .map_err(|e| {
                    (
                        1,
                        format!("journal error appending event for {child_id}: {e}\n"),
                    )
                })?;
        }
    }

    // 3. Update latest snapshot of parent workflow to phase: cancelled
    if parent_exists {
        if let Ok(Some(snap)) = journal.load_latest_snapshot(&parent_id) {
            if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&snap.state_json) {
                let cur_phase = val.get("phase").and_then(|p| p.as_str());
                if cur_phase != Some("cancelled") {
                    if let Some(obj) = val.as_object_mut() {
                        obj.insert(
                            "phase".to_string(),
                            serde_json::Value::String("cancelled".to_string()),
                        );
                    }
                    let updated_json = serde_json::to_string(&val).unwrap_or(snap.state_json);
                    let max_seq = journal
                        .load_events(&parent_id, 1)
                        .ok()
                        .and_then(|evs| evs.iter().map(|e| e.seq).max())
                        .unwrap_or(snap.seq + 1);
                    let _ = journal
                        .append_snapshot(&parent_id, &SnapshotRecord::new(max_seq, updated_json));
                }
            }
        }
    }

    // 4. Update pipeline state file if present
    let state_file = runs_dir.join(format!("pipeline-{task}.state"));
    if state_file.is_file() {
        if let Ok(mut st) = PipelineState::load(&state_file) {
            if st.phase != "cancelled" || st.finished == 0 {
                st.phase = "cancelled".to_string();
                st.finished = 1;
                let _ = st.save(&state_file);
            }
        }
    }

    println!("pipeline '{task}' cancelled");
    Ok(())
}
