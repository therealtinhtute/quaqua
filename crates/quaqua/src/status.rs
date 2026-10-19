//! Status overview: workflow journal + orchestration state.
//!
//! Queries `<workspace_root>/runs/quaqua.db` for durable event-sourced workflows,
//! scans active workers from `herdr.agent_list()` and `runs/*.owner`,
//! and flags coordination anomalies like `WAITER-LOST` (issue #47).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use crate::cap::is_task_worker_name;
use crate::herdr::HerdrOps;
use crate::state::WorkerMetadata;

/// A workflow row extracted from the journal SQLite database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStatusRow {
    pub workflow_id: String,
    pub snap_seq: Option<u64>,
    pub last_event_kind: Option<String>,
    pub last_event_seq: Option<u64>,
    pub last_event_ts_ms: Option<u64>,
    pub phase: Option<String>,
}

/// Status of the worker report file (`runs/<name>.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportStatus {
    None,
    Present,
    Unread,
}

/// Warning flags representing coordination defects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WarningFlag {
    WaiterLost,
    DoneNotCleaned,
    IdleUnprompted,
    Orphan,
}

impl WarningFlag {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WaiterLost => "WAITER-LOST",
            Self::DoneNotCleaned => "DONE-NOT-CLEANED",
            Self::IdleUnprompted => "IDLE-UNPROMPTED",
            Self::Orphan => "ORPHAN",
        }
    }
}

/// An orchestration row representing a Lead or task worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchestrationRow {
    pub name: String,
    pub owner: String,
    pub agent_status: String,
    pub report_status: ReportStatus,
    pub phase: String,
    pub wait_time: String,
    pub flags: Vec<WarningFlag>,
}

/// Query workflow rows from `<workspace_root>/runs/quaqua.db`.
///
/// Missing or empty DB yields `Ok(Vec::new())` cleanly.
/// An existing but corrupted, unreadable, or schema-drifted DB returns `Err("db error: ...")`.
pub fn query_workflows(db_path: &Path) -> Result<Vec<WorkflowStatusRow>, String> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("db error: cannot open {}: {e}", db_path.display()))?;

    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(|e| {
            format!(
                "db error: failed to set busy timeout on {}: {e}",
                db_path.display()
            )
        })?;

    let has_events: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='events');",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("db error: checking events table: {e}"))?;

    let has_snapshots: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='snapshots');",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("db error: checking snapshots table: {e}"))?;

    if !has_events && !has_snapshots {
        return Ok(Vec::new());
    }

    let mut map: BTreeMap<String, WorkflowStatusRow> = BTreeMap::new();

    if has_events {
        let mut stmt = conn
            .prepare(
                "SELECT e.workflow_id, e.seq, e.ts_ms, e.kind \
                 FROM events e \
                 INNER JOIN ( \
                     SELECT workflow_id, MAX(seq) AS max_seq \
                     FROM events \
                     GROUP BY workflow_id \
                 ) m ON e.workflow_id = m.workflow_id AND e.seq = m.max_seq;",
            )
            .map_err(|e| format!("db error: preparing events query: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                let wf: String = row.get(0)?;
                let seq: i64 = row.get(1)?;
                let ts_ms: i64 = row.get(2)?;
                let kind: String = row.get(3)?;
                Ok((wf, seq as u64, ts_ms as u64, kind))
            })
            .map_err(|e| format!("db error: executing events query: {e}"))?;

        for r in rows {
            let (wf, seq, ts_ms, kind) =
                r.map_err(|e| format!("db error: decoding event row: {e}"))?;
            let entry = map.entry(wf.clone()).or_insert_with(|| WorkflowStatusRow {
                workflow_id: wf,
                snap_seq: None,
                last_event_kind: None,
                last_event_seq: None,
                last_event_ts_ms: None,
                phase: None,
            });
            entry.last_event_seq = Some(seq);
            entry.last_event_ts_ms = Some(ts_ms);
            entry.last_event_kind = Some(kind);
        }
    }

    if has_snapshots {
        let mut stmt = conn
            .prepare(
                "SELECT s.workflow_id, s.seq, s.state_json \
                 FROM snapshots s \
                 INNER JOIN ( \
                     SELECT workflow_id, MAX(seq) AS max_seq \
                     FROM snapshots \
                     GROUP BY workflow_id \
                 ) m ON s.workflow_id = m.workflow_id AND s.seq = m.max_seq;",
            )
            .map_err(|e| format!("db error: preparing snapshots query: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                let wf: String = row.get(0)?;
                let seq: i64 = row.get(1)?;
                let state_json: String = row.get(2)?;
                Ok((wf, seq as u64, state_json))
            })
            .map_err(|e| format!("db error: executing snapshots query: {e}"))?;

        for r in rows {
            let (wf, seq, state_json) =
                r.map_err(|e| format!("db error: decoding snapshot row: {e}"))?;
            let entry = map.entry(wf.clone()).or_insert_with(|| WorkflowStatusRow {
                workflow_id: wf,
                snap_seq: None,
                last_event_kind: None,
                last_event_seq: None,
                last_event_ts_ms: None,
                phase: None,
            });
            entry.snap_seq = Some(seq);
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&state_json) {
                if let Some(phase_str) = val.get("phase").and_then(|p| p.as_str()) {
                    entry.phase = Some(phase_str.to_string());
                }
            }
        }
    }

    Ok(map.into_values().collect())
}

/// Scan orchestration entities across `herdr.agent_list()` and `runs_dir`.
///
/// Propagates `herdr error: ...` on I/O or JSON decoding errors to avoid
/// emitting misleading `missing` statuses and false `ORPHAN` flags.
pub fn scan_orchestration<H: HerdrOps>(
    runs_dir: &Path,
    herdr: &H,
) -> Result<Vec<OrchestrationRow>, String> {
    let json_str = herdr
        .agent_list()
        .map_err(|e| format!("herdr error: failed to query agent list: {e}"))?;

    let v: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| format!("herdr error: invalid agent list JSON: {e}"))?;

    let arr = v
        .get("result")
        .and_then(|r| r.get("agents"))
        .and_then(|a| a.as_array())
        .ok_or_else(|| "herdr error: missing result.agents array in response".to_string())?;

    let mut live_agents: BTreeMap<String, String> = BTreeMap::new();
    for agent in arr {
        if let Some(name) = agent.get("name").and_then(|n| n.as_str()) {
            let st = agent
                .get("agent_status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            live_agents.insert(name.to_string(), st.to_string());
        }
    }

    let mut entity_names: BTreeSet<String> = BTreeSet::new();
    for name in live_agents.keys() {
        entity_names.insert(name.clone());
    }

    if let Ok(entries) = std::fs::read_dir(runs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(fname) = path.file_name().and_then(|n| n.to_str()) {
                if let Some(worker) = fname.strip_suffix(".owner") {
                    if !worker.starts_with('.') {
                        entity_names.insert(worker.to_string());
                    }
                }
            }
        }
    }

    let mut rows = Vec::new();
    for name in entity_names {
        let owner = WorkerMetadata::load_owner(runs_dir, &name);
        let agent_status = live_agents
            .get(&name)
            .cloned()
            .unwrap_or_else(|| "missing".to_string());

        let report_path = runs_dir.join(format!("{name}.md"));
        let report_exists = report_path.is_file();

        // Check quaqua-owned `.read` marker first.
        // The marker is only valid if its mtime >= report file mtime (not stale).
        // Fallback: check if report file access time is at or before modified time.
        let read_marker_path = runs_dir.join(format!(".{name}.read"));
        let is_unread = if report_exists {
            let marker_valid = if read_marker_path.is_file() {
                if let (Ok(m_meta), Ok(r_meta)) = (read_marker_path.metadata(), report_path.metadata()) {
                    if let (Ok(m_mtime), Ok(r_mtime)) = (m_meta.modified(), r_meta.modified()) {
                        m_mtime >= r_mtime
                    } else {
                        true
                    }
                } else {
                    true
                }
            } else {
                false
            };

            if marker_valid {
                false
            } else if let Ok(meta) = std::fs::metadata(&report_path) {
                if let (Ok(atime), Ok(mtime)) = (meta.accessed(), meta.modified()) {
                    atime <= mtime
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        let owner_status = live_agents.get(&owner).map(|s| s.as_str());

        let mut flags = Vec::new();

        let worker_running = agent_status == "working";
        let owner_is_cos = owner == "cos";

        let owner_inactive = match owner_status {
            Some("idle") | Some("done") => true,
            None if !owner_is_cos => true,
            _ => false,
        };

        // WAITER-LOST:
        // 1. Orchestrator is inactive while its worker is still running.
        // 2. Report exists and nobody has read it (atime <= mtime).
        let waiter_lost_running = worker_running && owner_inactive;
        let waiter_lost_report = report_exists && is_unread;

        if waiter_lost_running || waiter_lost_report {
            flags.push(WarningFlag::WaiterLost);
        }

        let owner_file_exists = WorkerMetadata::owner_path(runs_dir, &name).exists();
        if owner_file_exists
            && (agent_status == "done"
                || agent_status == "dead"
                || (report_exists && agent_status != "working"))
        {
            flags.push(WarningFlag::DoneNotCleaned);
        }

        let has_budget = WorkerMetadata::budget_path(runs_dir, &name).exists();
        let has_progress = WorkerMetadata::progress_path(runs_dir, &name).exists();
        if is_task_worker_name(&name) && agent_status == "idle" && !has_budget && !has_progress {
            flags.push(WarningFlag::IdleUnprompted);
        }

        if !owner_is_cos && !live_agents.contains_key(&owner) && owner_file_exists {
            flags.push(WarningFlag::Orphan);
        }

        let report_status = if !report_exists {
            ReportStatus::None
        } else if is_unread {
            ReportStatus::Unread
        } else {
            ReportStatus::Present
        };

        let phase = std::fs::read_to_string(WorkerMetadata::pipeline_path(runs_dir, &name))
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "-".to_string());

        let elapsed = WorkerMetadata::load_elapsed(runs_dir, &name);
        let wait_time = if elapsed > 0 {
            if elapsed >= 60 {
                format!("{}m", elapsed / 60)
            } else {
                format!("{elapsed}s")
            }
        } else {
            "-".to_string()
        };

        flags.sort();
        flags.dedup();

        rows.push(OrchestrationRow {
            name,
            owner,
            agent_status,
            report_status,
            phase,
            wait_time,
            flags,
        });
    }

    Ok(rows)
}

/// Format workflow and orchestration status as a clean text table.
pub fn format_status_report(
    workflows: Result<&[WorkflowStatusRow], &str>,
    orchestration: Result<&[OrchestrationRow], &str>,
) -> String {
    let mut out = String::new();

    out.push_str("=== Workflows (runs/quaqua.db) ===\n");
    match workflows {
        Ok(wfs) if wfs.is_empty() => {
            out.push_str("no workflows\n");
        }
        Ok(wfs) => {
            out.push_str(&format!(
                "{:<20} {:<10} {:<20} {:<10} {:<12} {:<10}\n",
                "WORKFLOW_ID", "SNAP_SEQ", "LAST_EVENT", "EVENT_SEQ", "LAST_TS", "PHASE"
            ));
            for wf in wfs {
                let snap = wf
                    .snap_seq
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let ev_kind = wf.last_event_kind.as_deref().unwrap_or("-");
                let ev_seq = wf
                    .last_event_seq
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let ev_ts = wf
                    .last_event_ts_ms
                    .map(|t| format!("{t}ms"))
                    .unwrap_or_else(|| "-".to_string());
                let phase = wf.phase.as_deref().unwrap_or("-");
                out.push_str(&format!(
                    "{:<20} {:<10} {:<20} {:<10} {:<12} {:<10}\n",
                    wf.workflow_id, snap, ev_kind, ev_seq, ev_ts, phase
                ));
            }
        }
        Err(err) => {
            out.push_str(&format!("{err}\n"));
        }
    }

    out.push_str("\n=== Orchestration ===\n");
    match orchestration {
        Ok(orch_rows) if orch_rows.is_empty() => {
            out.push_str("no active agents or workers\n");
        }
        Ok(orch_rows) => {
            out.push_str(&format!(
                "{:<18} {:<16} {:<10} {:<8} {:<10} {:<8} {}\n",
                "NAME", "OWNER", "STATUS", "REPORT", "PHASE", "WAIT", "WARNINGS"
            ));
            for r in orch_rows {
                let rep_str = match r.report_status {
                    ReportStatus::None => "none",
                    ReportStatus::Present => "present",
                    ReportStatus::Unread => "unread",
                };
                let flags_str = if r.flags.is_empty() {
                    "-".to_string()
                } else {
                    r.flags
                        .iter()
                        .map(|f| f.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                out.push_str(&format!(
                    "{:<18} {:<16} {:<10} {:<8} {:<10} {:<8} {}\n",
                    r.name, r.owner, r.agent_status, rep_str, r.phase, r.wait_time, flags_str
                ));
            }
        }
        Err(err) => {
            out.push_str(&format!("{err}\n"));
        }
    }

    out
}

/// Render full status from explicit DB and runs directory paths.
pub fn render_status<H: HerdrOps>(
    db_path: &Path,
    runs_dir: &Path,
    herdr: &H,
) -> Result<String, String> {
    let wf_res = query_workflows(db_path);
    let orch_res = scan_orchestration(runs_dir, herdr);
    Ok(format_status_report(
        wf_res.as_deref().map_err(|e| e.as_str()),
        orch_res.as_deref().map_err(|e| e.as_str()),
    ))
}

/// CLI entrypoint for `quaqua status`.
pub fn handle_status<H: HerdrOps>(_args: &[String], herdr: &H) -> Result<i32, String> {
    let db_path = crate::workspace::workspace_root()
        .join("runs")
        .join("quaqua.db");
    let runs_dir = crate::workspace::workspace_root().join("runs");
    let report = render_status(&db_path, &runs_dir, herdr)?;
    print!("{report}");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{Child, Output};

    struct MockHerdr {
        agent_list_result: std::io::Result<String>,
    }

    impl MockHerdr {
        fn new(json: impl Into<String>) -> Self {
            Self {
                agent_list_result: Ok(json.into()),
            }
        }

        fn with_err(err: std::io::Error) -> Self {
            Self {
                agent_list_result: Err(err),
            }
        }
    }

    impl HerdrOps for MockHerdr {
        fn run(&self, _args: &[&str]) -> std::io::Result<Output> {
            unimplemented!()
        }

        fn agent_list(&self) -> std::io::Result<String> {
            match &self.agent_list_result {
                Ok(s) => Ok(s.clone()),
                Err(e) => Err(std::io::Error::new(e.kind(), e.to_string())),
            }
        }

        fn agent_get(&self, _name: &str) -> std::io::Result<(bool, String)> {
            unimplemented!()
        }

        fn agent_read(&self, _name: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
            unimplemented!()
        }

        fn agent_start(
            &self,
            _name: &str,
            _kind: &str,
            _pane: &str,
            _timeout_ms: u32,
            _extra_args: &[&str],
        ) -> std::io::Result<Output> {
            unimplemented!()
        }

        fn pane_split(
            &self,
            _cwd: &str,
            _owner: &str,
            _envs: &[(&str, &str)],
        ) -> std::io::Result<Output> {
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

        fn agent_prompt_wait_background(
            &self,
            _name: &str,
            _prompt: &str,
            _timeout_ms: u32,
            _log_file: fs::File,
        ) -> std::io::Result<Child> {
            unimplemented!()
        }
    }

    fn temp_test_dir(prefix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("quaqua-status-{prefix}-{nanos}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    struct TempDirCleaner(PathBuf);
    impl Drop for TempDirCleaner {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_status_missing_db_yields_no_workflows_cleanly() {
        let tmp = temp_test_dir("missing");
        let _cleaner = TempDirCleaner(tmp.clone());

        let missing_db = tmp.join("does_not_exist.db");
        let res = query_workflows(&missing_db);
        assert!(res.is_ok(), "missing db must not error");
        let workflows = res.unwrap();
        assert!(workflows.is_empty(), "expected empty workflows");

        let report = format_status_report(Ok(&workflows), Ok(&[]));
        assert!(
            report.contains("no workflows"),
            "must contain 'no workflows'"
        );
        assert!(!missing_db.exists(), "must not create missing file");
    }

    #[test]
    fn test_status_empty_db_yields_no_workflows() {
        let tmp = temp_test_dir("empty");
        let _cleaner = TempDirCleaner(tmp.clone());

        let db_path = tmp.join("empty.db");
        {
            let conn = rusqlite::Connection::open(&db_path).expect("create db");
            conn.execute_batch(
                r#"
                CREATE TABLE events (
                    workflow_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    ts_ms INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY (workflow_id, seq)
                );
                CREATE TABLE snapshots (
                    workflow_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    state_json TEXT NOT NULL,
                    PRIMARY KEY (workflow_id, seq)
                );
                "#,
            )
            .expect("create schema");
        }

        let res = query_workflows(&db_path);
        assert!(res.is_ok(), "empty db must not error");
        let workflows = res.unwrap();
        assert!(workflows.is_empty(), "expected 0 workflows in empty db");

        let report = format_status_report(Ok(&workflows), Ok(&[]));
        assert!(report.contains("no workflows"));
    }

    #[test]
    fn test_status_parses_workflow_rows_from_events_and_snapshots() {
        let tmp = temp_test_dir("parsed");
        let _cleaner = TempDirCleaner(tmp.clone());

        let db_path = tmp.join("test.db");
        {
            let conn = rusqlite::Connection::open(&db_path).expect("create db");
            conn.execute_batch(
                r#"
                CREATE TABLE events (
                    workflow_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    ts_ms INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY (workflow_id, seq)
                );
                CREATE TABLE snapshots (
                    workflow_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    state_json TEXT NOT NULL,
                    PRIMARY KEY (workflow_id, seq)
                );

                INSERT INTO events VALUES ('wf-pipe-1', 1, 1000, 'workflow_started', '{}');
                INSERT INTO events VALUES ('wf-pipe-1', 2, 1020, 'activity_scheduled', '{}');
                INSERT INTO events VALUES ('wf-pipe-1', 3, 1050, 'activity_completed', '{}');
                INSERT INTO snapshots VALUES ('wf-pipe-1', 2, '{"phase": "review"}');

                INSERT INTO events VALUES ('wf-worker-2', 1, 2000, 'workflow_started', '{}');
                "#,
            )
            .expect("insert test rows");
        }

        let workflows = query_workflows(&db_path).expect("query workflows");
        assert_eq!(workflows.len(), 2, "expected 2 workflows");

        let p1 = workflows
            .iter()
            .find(|w| w.workflow_id == "wf-pipe-1")
            .expect("wf-pipe-1 found");
        assert_eq!(p1.snap_seq, Some(2));
        assert_eq!(p1.phase, Some("review".to_string()));
        assert_eq!(p1.last_event_seq, Some(3));
        assert_eq!(p1.last_event_kind, Some("activity_completed".to_string()));
        assert_eq!(p1.last_event_ts_ms, Some(1050));

        let w2 = workflows
            .iter()
            .find(|w| w.workflow_id == "wf-worker-2")
            .expect("wf-worker-2 found");
        assert_eq!(w2.snap_seq, None);
        assert_eq!(w2.phase, None);
        assert_eq!(w2.last_event_seq, Some(1));
        assert_eq!(w2.last_event_kind, Some("workflow_started".to_string()));
        assert_eq!(w2.last_event_ts_ms, Some(2000));
    }

    #[test]
    fn test_status_raises_waiter_lost_when_orchestrator_idle_and_worker_working() {
        let tmp = temp_test_dir("waiter-lost-1");
        let _cleaner = TempDirCleaner(tmp.clone());

        // mu-lead is idle; its worker sude-p4 is working
        let mock_json = r#"{
            "result": {
                "agents": [
                    {"name": "mu-lead", "agent_status": "idle"},
                    {"name": "sude-p4", "agent_status": "working"}
                ]
            }
        }"#;
        let mock = MockHerdr::new(mock_json);

        fs::write(tmp.join("sude-p4.owner"), "mu-lead\n").unwrap();

        let rows = scan_orchestration(&tmp, &mock).expect("scan orchestration");
        let p4 = rows
            .iter()
            .find(|r| r.name == "sude-p4")
            .expect("sude-p4 found");
        assert!(
            p4.flags.contains(&WarningFlag::WaiterLost),
            "sude-p4 must raise WAITER-LOST because mu-lead is idle"
        );
    }

    #[test]
    fn test_status_raises_waiter_lost_when_report_exists_and_unread() {
        let tmp = temp_test_dir("waiter-lost-2");
        let _cleaner = TempDirCleaner(tmp.clone());

        // arch-lead is idle; worker sude-e1 finished and submitted report (atime <= mtime)
        let mock_json = r#"{
            "result": {
                "agents": [
                    {"name": "arch-lead", "agent_status": "idle"},
                    {"name": "sude-e1", "agent_status": "done"}
                ]
            }
        }"#;
        let mock = MockHerdr::new(mock_json);

        fs::write(tmp.join("sude-e1.owner"), "arch-lead\n").unwrap();
        let report_path = tmp.join("sude-e1.md");
        fs::write(
            &report_path,
            "Goal: done\n## Blockers\nnone\n## Lessons\nnone\n",
        )
        .unwrap();

        // Explicitly set atime <= mtime (accessed before modified -> unread)
        let file = fs::File::open(&report_path).unwrap();
        let now = std::time::SystemTime::now();
        let mut times = fs::FileTimes::new();
        times = times
            .set_modified(now)
            .set_accessed(now - Duration::from_secs(5));
        file.set_times(times).unwrap();

        let rows = scan_orchestration(&tmp, &mock).expect("scan orchestration");
        let e1 = rows
            .iter()
            .find(|r| r.name == "sude-e1")
            .expect("sude-e1 found");
        assert!(
            e1.flags.contains(&WarningFlag::WaiterLost),
            "must raise WAITER-LOST when report exists unread"
        );
        assert!(
            e1.flags.contains(&WarningFlag::DoneNotCleaned),
            "must raise DONE-NOT-CLEANED"
        );
        assert_eq!(e1.report_status, ReportStatus::Unread);
    }

    #[test]
    fn test_status_negative_report_read_with_idle_owner_does_not_raise_waiter_lost() {
        let tmp = temp_test_dir("neg-report-read");
        let _cleaner = TempDirCleaner(tmp.clone());

        // arch-lead is idle; worker sude-e1 has a report that HAS been read (atime > mtime)
        let mock_json = r#"{
            "result": {
                "agents": [
                    {"name": "arch-lead", "agent_status": "idle"},
                    {"name": "sude-e1", "agent_status": "done"}
                ]
            }
        }"#;
        let mock = MockHerdr::new(mock_json);

        fs::write(tmp.join("sude-e1.owner"), "arch-lead\n").unwrap();
        let report_path = tmp.join("sude-e1.md");
        fs::write(&report_path, "Goal: done\n## Lessons\nnone\n").unwrap();

        // Set atime > mtime (accessed 10s after modified -> read)
        let file = fs::File::open(&report_path).unwrap();
        let now = std::time::SystemTime::now();
        let mtime = now - Duration::from_secs(10);
        let atime = now;
        let mut times = fs::FileTimes::new();
        times = times.set_modified(mtime).set_accessed(atime);
        file.set_times(times).unwrap();

        let rows = scan_orchestration(&tmp, &mock).expect("scan orchestration");
        let e1 = rows
            .iter()
            .find(|r| r.name == "sude-e1")
            .expect("sude-e1 found");

        assert!(
            !e1.flags.contains(&WarningFlag::WaiterLost),
            "read report must NOT raise WAITER-LOST even if owner is idle"
        );
        assert_eq!(e1.report_status, ReportStatus::Present);
        assert!(e1.flags.contains(&WarningFlag::DoneNotCleaned));
    }
    #[test]
    fn test_status_read_marker_overrides_atime_unread() {
        let tmp = temp_test_dir("read-marker-override");
        let _cleaner = TempDirCleaner(tmp.clone());

        let mock_json = r#"{
            "result": {
                "agents": [
                    {"name": "arch-lead", "agent_status": "idle"},
                    {"name": "sude-e1", "agent_status": "done"}
                ]
            }
        }"#;
        let mock = MockHerdr::new(mock_json);

        fs::write(tmp.join("sude-e1.owner"), "arch-lead\n").unwrap();
        let report_path = tmp.join("sude-e1.md");
        fs::write(&report_path, "Goal: done\n## Lessons\nnone\n").unwrap();

        // Force atime <= mtime (unaccessed heuristic)
        let file = fs::File::open(&report_path).unwrap();
        let now = std::time::SystemTime::now();
        let report_mtime = now - Duration::from_secs(5);
        let mut times = fs::FileTimes::new();
        times = times
            .set_modified(report_mtime)
            .set_accessed(now - Duration::from_secs(10));
        file.set_times(times).unwrap();

        // Write .read marker with mtime >= report mtime
        let marker_path = tmp.join(".sude-e1.read");
        fs::write(&marker_path, "").unwrap();
        let marker_file = fs::File::open(&marker_path).unwrap();
        let mut marker_times = fs::FileTimes::new();
        marker_times = marker_times.set_modified(now);
        marker_file.set_times(marker_times).unwrap();
        let rows = scan_orchestration(&tmp, &mock).expect("scan orchestration");
        let e1 = rows
            .iter()
            .find(|r| r.name == "sude-e1")
            .expect("sude-e1 found");

        assert!(
            !e1.flags.contains(&WarningFlag::WaiterLost),
            ".read marker must override atime heuristic and not flag WAITER-LOST"
        );
        assert_eq!(e1.report_status, ReportStatus::Present);
    }
    #[test]
    fn test_status_stale_read_marker_treated_as_unread() {
        let tmp = temp_test_dir("stale-read-marker");
        let _cleaner = TempDirCleaner(tmp.clone());

        let mock_json = r#"{
            "result": {
                "agents": [
                    {"name": "arch-lead", "agent_status": "idle"},
                    {"name": "sude-e1", "agent_status": "done"}
                ]
            }
        }"#;
        let mock = MockHerdr::new(mock_json);

        fs::write(tmp.join("sude-e1.owner"), "arch-lead\n").unwrap();
        let marker_path = tmp.join(".sude-e1.read");
        fs::write(&marker_path, "").unwrap();

        let report_path = tmp.join("sude-e1.md");
        fs::write(&report_path, "Goal: done\n## Lessons\nnone\n").unwrap();

        let now = std::time::SystemTime::now();
        let marker_file = fs::File::open(&marker_path).unwrap();
        let mut marker_times = fs::FileTimes::new();
        marker_times = marker_times.set_modified(now - Duration::from_secs(20));
        marker_file.set_times(marker_times).unwrap();

        let report_file = fs::File::open(&report_path).unwrap();
        let mut report_times = fs::FileTimes::new();
        report_times = report_times
            .set_modified(now)
            .set_accessed(now - Duration::from_secs(10));
        report_file.set_times(report_times).unwrap();

        let rows = scan_orchestration(&tmp, &mock).expect("scan orchestration");
        let e1 = rows
            .iter()
            .find(|r| r.name == "sude-e1")
            .expect("sude-e1 found");

        assert!(
            e1.flags.contains(&WarningFlag::WaiterLost),
            "stale .read marker must NOT suppress WAITER-LOST"
        );
        assert_eq!(e1.report_status, ReportStatus::Unread);
    }

    #[test]
    fn test_status_negative_working_owner_with_working_worker_does_not_raise_waiter_lost() {
        let tmp = temp_test_dir("neg-working");
        let _cleaner = TempDirCleaner(tmp.clone());

        // arch-lead is actively working; worker sude-e1 is working; no report
        let mock_json = r#"{
            "result": {
                "agents": [
                    {"name": "arch-lead", "agent_status": "working"},
                    {"name": "sude-e1", "agent_status": "working"}
                ]
            }
        }"#;
        let mock = MockHerdr::new(mock_json);

        fs::write(tmp.join("sude-e1.owner"), "arch-lead\n").unwrap();

        let rows = scan_orchestration(&tmp, &mock).expect("scan orchestration");
        let e1 = rows
            .iter()
            .find(|r| r.name == "sude-e1")
            .expect("sude-e1 found");

        assert!(
            !e1.flags.contains(&WarningFlag::WaiterLost),
            "working worker under working owner must NOT raise WAITER-LOST"
        );
        assert!(
            e1.flags.is_empty(),
            "expected 0 flags on healthy working worker"
        );
    }

    #[test]
    fn test_status_corrupted_or_unreadable_db_surfaces_db_error() {
        let tmp = temp_test_dir("corrupt");
        let _cleaner = TempDirCleaner(tmp.clone());

        let db_path = tmp.join("corrupt.db");
        fs::write(&db_path, b"not a sqlite database file at all").unwrap();

        let res = query_workflows(&db_path);
        assert!(res.is_err(), "corrupt db must return Err");
        let err_msg = res.unwrap_err();
        assert!(
            err_msg.starts_with("db error:"),
            "error message must start with 'db error:' but got: {err_msg}"
        );

        let report = format_status_report(Err(&err_msg), Ok(&[]));
        assert!(
            report.contains("db error:"),
            "rendered report must show db error"
        );
        assert!(
            !report.contains("no workflows"),
            "rendered report must NOT claim 'no workflows' when DB is corrupted"
        );
    }

    #[test]
    fn test_status_schema_drifted_undecodable_row_surfaces_db_error() {
        let tmp = temp_test_dir("schema-drift");
        let _cleaner = TempDirCleaner(tmp.clone());

        let db_path = tmp.join("drift.db");
        {
            let conn = rusqlite::Connection::open(&db_path).expect("create db");
            conn.execute_batch(
                r#"
                CREATE TABLE events (
                    workflow_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    ts_ms TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY (workflow_id, seq)
                );
                INSERT INTO events VALUES ('wf-drift', 1, 'not_an_integer_timestamp', 'kind', '{}');
                "#,
            )
            .expect("init drift schema");
        }

        let res = query_workflows(&db_path);
        assert!(
            res.is_err(),
            "schema drift / decoding failure must return Err"
        );
        let err_msg = res.unwrap_err();
        assert!(
            err_msg.contains("db error: decoding event row"),
            "must report decoding error, got: {err_msg}"
        );
    }

    #[test]
    fn test_status_herdr_agent_list_error_surfaces_herdr_error() {
        let tmp = temp_test_dir("herdr-err");
        let _cleaner = TempDirCleaner(tmp.clone());

        let io_err = std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "herdr socket unavailable",
        );
        let mock = MockHerdr::with_err(io_err);

        // Even with an owner file present, it must NOT emit false ORPHAN rows!
        fs::write(tmp.join("sude-test.owner"), "arch-lead\n").unwrap();

        let res = scan_orchestration(&tmp, &mock);
        assert!(
            res.is_err(),
            "scan_orchestration must fail when herdr fails"
        );
        let err_msg = res.unwrap_err();
        assert!(
            err_msg.starts_with("herdr error:"),
            "error must start with 'herdr error:' but got: {err_msg}"
        );

        let report = format_status_report(Ok(&[]), Err(&err_msg));
        assert!(
            report.contains("herdr error:"),
            "report must show herdr error"
        );
        assert!(
            !report.contains("ORPHAN"),
            "must NOT emit false ORPHAN flags when herdr is unreachable"
        );
    }

    #[test]
    fn test_status_herdr_invalid_json_surfaces_herdr_error() {
        let tmp = temp_test_dir("herdr-json-err");
        let _cleaner = TempDirCleaner(tmp.clone());

        let mock = MockHerdr::new("invalid json output from herdr");
        let res = scan_orchestration(&tmp, &mock);
        assert!(
            res.is_err(),
            "scan_orchestration must fail on unparseable JSON"
        );
        let err_msg = res.unwrap_err();
        assert!(
            err_msg.contains("herdr error: invalid agent list JSON"),
            "error must identify invalid json, got: {err_msg}"
        );

        let report = format_status_report(Ok(&[]), Err(&err_msg));
        assert!(report.contains("herdr error: invalid agent list JSON"));
    }

    #[test]
    fn test_status_flags_done_not_cleaned_idle_unprompted_and_orphan() {
        let tmp = temp_test_dir("flags");
        let _cleaner = TempDirCleaner(tmp.clone());

        let mock_json = r#"{
            "result": {
                "agents": [
                    {"name": "active-lead", "agent_status": "working"},
                    {"name": "sude-done", "agent_status": "done"},
                    {"name": "sude-idle", "agent_status": "idle"},
                    {"name": "sude-orphan", "agent_status": "working"}
                ]
            }
        }"#;
        let mock = MockHerdr::new(mock_json);

        fs::write(tmp.join("sude-done.owner"), "active-lead\n").unwrap();
        fs::write(tmp.join("sude-idle.owner"), "active-lead\n").unwrap();
        fs::write(tmp.join("sude-orphan.owner"), "ghost-lead\n").unwrap();

        let rows = scan_orchestration(&tmp, &mock).expect("scan orchestration");

        let done = rows
            .iter()
            .find(|r| r.name == "sude-done")
            .expect("sude-done found");
        assert!(done.flags.contains(&WarningFlag::DoneNotCleaned));

        let idle = rows
            .iter()
            .find(|r| r.name == "sude-idle")
            .expect("sude-idle found");
        assert!(idle.flags.contains(&WarningFlag::IdleUnprompted));

        let orphan = rows
            .iter()
            .find(|r| r.name == "sude-orphan")
            .expect("sude-orphan found");
        assert!(orphan.flags.contains(&WarningFlag::Orphan));
        assert!(orphan.flags.contains(&WarningFlag::WaiterLost));
    }

    #[test]
    fn test_status_table_formatting_text_output() {
        let tmp = temp_test_dir("format");
        let _cleaner = TempDirCleaner(tmp.clone());

        let db_path = tmp.join("test.db");
        {
            let conn = rusqlite::Connection::open(&db_path).expect("create db");
            conn.execute_batch(
                r#"
                CREATE TABLE events (
                    workflow_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    ts_ms INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY (workflow_id, seq)
                );
                CREATE TABLE snapshots (
                    workflow_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    state_json TEXT NOT NULL,
                    PRIMARY KEY (workflow_id, seq)
                );
                INSERT INTO events VALUES ('pipe-test', 1, 1000, 'workflow_started', '{}');
                INSERT INTO snapshots VALUES ('pipe-test', 1, '{"phase":"build"}');
                "#,
            )
            .expect("init rows");
        }

        let mock_json = r#"{
            "result": {
                "agents": [
                    {"name": "arch-lead", "agent_status": "working"},
                    {"name": "sude-w1", "agent_status": "working"}
                ]
            }
        }"#;
        let mock = MockHerdr::new(mock_json);
        fs::write(tmp.join("sude-w1.owner"), "arch-lead\n").unwrap();

        let out = render_status(&db_path, &tmp, &mock).expect("render status");
        assert!(out.contains("=== Workflows (runs/quaqua.db) ==="));
        assert!(out.contains("pipe-test"));
        assert!(out.contains("=== Orchestration ==="));
        assert!(out.contains("arch-lead"));
        assert!(out.contains("sude-w1"));
        assert!(out.contains("WAIT"));
    }
}
