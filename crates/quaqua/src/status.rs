//! Status overview: workflow journal + orchestration state.
//!
//! Queries `<workspace_root>/runs/quaqua.db` for durable event-sourced workflows,
//! scans active workers from `herdr.agent_list()` and `runs/*.owner`,
//! and flags coordination anomalies like `WAITER-LOST` (issue #47).

use crate::worker::cleanup::WorkerTelemetryRow;
use crate::worker::prompt::cost::Tokens;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cap::is_task_worker_name;
use crate::config::{load_team_config, Config};
use crate::engine::journal_sqlite::{SqliteJournal, VerdictLevel, VerdictRecord, VerdictSource};
use crate::git::{Git, GitOps};
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

/// A branch verdict row showing the strongest verdict and freshness flags (§ S2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchVerdictRow {
    pub branch: String,
    pub head_sha: String,
    pub verdict_sha: String,
    pub level: VerdictLevel,
    pub is_verified: bool,
    pub stale: bool,
    pub same_harness: bool,
}

/// Returns true if the verifier's harness matches the builder kind's harness in team.toml (§ S2).
pub fn is_same_harness(verdict: &VerdictRecord, cfg: Option<&Config>) -> bool {
    if verdict.source != VerdictSource::Verifier {
        return false;
    }

    let verifier_harness = if !verdict.harness.is_empty() {
        verdict.harness.clone()
    } else if let Some(vk) = &verdict.verifier_kind {
        cfg.and_then(|c| c.kinds.get(vk))
            .map(|k| k.harness.clone())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let builder_harness = if let Some(bk) = &verdict.builder_kind {
        cfg.and_then(|c| c.kinds.get(bk))
            .map(|k| k.harness.clone())
            .unwrap_or_default()
    } else {
        let b_part = verdict
            .branch
            .strip_prefix("quaqua/")
            .unwrap_or(&verdict.branch);
        let inferred_kind = b_part.split('-').next().unwrap_or(b_part);
        cfg.and_then(|c| c.kinds.get(inferred_kind))
            .map(|k| k.harness.clone())
            .unwrap_or_default()
    };

    !verifier_harness.is_empty()
        && !builder_harness.is_empty()
        && verifier_harness == builder_harness
}

fn resolve_branch_head<G: GitOps>(git: &G, repo: &Path, branch: &str) -> Option<String> {
    let ref_name = if branch.starts_with("refs/") {
        branch.to_string()
    } else {
        format!("refs/heads/{branch}")
    };
    if let Ok(out) = git.run_in(repo, &["rev-parse", "--verify", "--quiet", &ref_name]) {
        if out.status.success() {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !sha.is_empty() {
                return Some(sha);
            }
        }
    }
    if let Ok(out) = git.run_in(repo, &["rev-parse", "--verify", "--quiet", branch]) {
        if out.status.success() {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !sha.is_empty() {
                return Some(sha);
            }
        }
    }
    None
}

/// Query branch verdict rows from the journal and repository (§ S2).
pub fn query_branch_verdicts<G: GitOps>(
    db_path: &Path,
    repo: &Path,
    git: &G,
) -> Result<Vec<BranchVerdictRow>, String> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let journal = SqliteJournal::open(db_path)
        .map_err(|e| format!("db error: cannot open {}: {e}", db_path.display()))?;

    let all_verdicts = journal
        .load_all_verdicts()
        .map_err(|e| format!("db error: loading verdicts: {e}"))?;

    if all_verdicts.is_empty() {
        return Ok(Vec::new());
    }

    let team_cfg = load_team_config().ok();

    let mut branches_seen = BTreeSet::new();
    let mut branch_list = Vec::new();
    for v in &all_verdicts {
        if branches_seen.insert(v.branch.clone()) {
            branch_list.push(v.branch.clone());
        }
    }
    branch_list.sort();

    let mut rows = Vec::new();
    for branch in branch_list {
        let records_for_branch: Vec<&VerdictRecord> =
            all_verdicts.iter().filter(|v| v.branch == branch).collect();
        if records_for_branch.is_empty() {
            continue;
        }

        let head_sha_opt = resolve_branch_head(git, repo, &branch);

        match head_sha_opt {
            Some(head_sha) => {
                let verdict_at_head = journal
                    .strongest_verdict(&branch, &head_sha)
                    .map_err(|e| format!("db error: {e}"))?;

                if let Some(v) = verdict_at_head {
                    let same_harness = is_same_harness(&v, team_cfg.as_ref());
                    rows.push(BranchVerdictRow {
                        branch: branch.clone(),
                        head_sha: head_sha.clone(),
                        verdict_sha: v.sha.clone(),
                        level: v.level,
                        is_verified: v.is_verified(),
                        stale: false,
                        same_harness,
                    });
                } else {
                    let last_record = records_for_branch.last().unwrap();
                    let strongest_old = journal
                        .strongest_verdict(&branch, &last_record.sha)
                        .map_err(|e| format!("db error: {e}"))?;

                    if let Some(v) = strongest_old {
                        let same_harness = is_same_harness(&v, team_cfg.as_ref());
                        let stale = head_sha != v.sha;
                        rows.push(BranchVerdictRow {
                            branch: branch.clone(),
                            head_sha: head_sha.clone(),
                            verdict_sha: v.sha.clone(),
                            level: v.level,
                            is_verified: v.is_verified(),
                            stale,
                            same_harness,
                        });
                    }
                }
            }
            None => {
                let last_record = records_for_branch.last().unwrap();
                let strongest = journal
                    .strongest_verdict(&branch, &last_record.sha)
                    .map_err(|e| format!("db error: {e}"))?;

                if let Some(v) = strongest {
                    let same_harness = is_same_harness(&v, team_cfg.as_ref());
                    rows.push(BranchVerdictRow {
                        branch: branch.clone(),
                        head_sha: v.sha.clone(),
                        verdict_sha: v.sha.clone(),
                        level: v.level,
                        is_verified: v.is_verified(),
                        stale: false,
                        same_harness,
                    });
                }
            }
        }
    }

    Ok(rows)
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
                if let (Ok(m_meta), Ok(r_meta)) =
                    (read_marker_path.metadata(), report_path.metadata())
                {
                    if let (Ok(m_mtime), Ok(r_mtime)) = (m_meta.modified(), r_meta.modified()) {
                        m_mtime >= r_mtime
                    } else {
                        false
                    }
                } else {
                    false
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

        let has_session = {
            let db_path = runs_dir.join("quaqua.db");
            if db_path.exists() {
                rusqlite::Connection::open_with_flags(
                    &db_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .ok()
                .and_then(|conn| {
                    let pattern = format!("worker:{name}#%");
                    conn.query_row(
                        "SELECT 1 FROM events WHERE workflow_id LIKE ? LIMIT 1;",
                        [&pattern],
                        |_| Ok(()),
                    )
                    .ok()
                })
                .is_some()
            } else {
                false
            }
        };
        let has_progress = WorkerMetadata::progress_path(runs_dir, &name).exists();
        if is_task_worker_name(&name) && agent_status == "idle" && !has_session && !has_progress {
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
    verdicts: Result<&[BranchVerdictRow], &str>,
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

    out.push_str("\n=== Verdicts ===\n");
    match verdicts {
        Ok(v_rows) if v_rows.is_empty() => {
            out.push_str("no verdicts\n");
        }
        Ok(v_rows) => {
            out.push_str(&format!(
                "{:<24} {:<12} {:<20} {}\n",
                "BRANCH", "HEAD", "VERDICT", "FLAGS"
            ));
            for r in v_rows {
                let mut flags = Vec::new();
                if r.stale {
                    flags.push("stale");
                }
                if r.same_harness {
                    flags.push("same-harness");
                }
                let flags_str = if flags.is_empty() {
                    "-".to_string()
                } else {
                    flags.join(", ")
                };
                let head_str = if r.head_sha.len() >= 7 {
                    &r.head_sha[..7]
                } else if r.head_sha.is_empty() {
                    "-"
                } else {
                    &r.head_sha
                };
                out.push_str(&format!(
                    "{:<24} {:<12} {:<20} {}\n",
                    r.branch,
                    head_str,
                    r.level.as_str(),
                    flags_str
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
    let git = Git::new();
    render_status_with_git(db_path, runs_dir, herdr, &git)
}

/// Render status with explicit Git implementation.
pub fn render_status_with_git<H: HerdrOps, G: GitOps>(
    db_path: &Path,
    runs_dir: &Path,
    herdr: &H,
    git: &G,
) -> Result<String, String> {
    let wf_res = query_workflows(db_path);
    let orch_res = scan_orchestration(runs_dir, herdr);
    let repo = runs_dir.parent().unwrap_or_else(|| Path::new("."));
    let verdicts_res = query_branch_verdicts(db_path, repo, git);
    Ok(format_status_report(
        wf_res.as_deref().map_err(|e| e.as_str()),
        orch_res.as_deref().map_err(|e| e.as_str()),
        verdicts_res.as_deref().map_err(|e| e.as_str()),
    ))
}

/// CLI entrypoint for `quaqua status`.
pub fn handle_status<H: HerdrOps>(args: &[String], herdr: &H) -> Result<i32, String> {
    if args
        .iter()
        .any(|a| a == "--metrics" || a.starts_with("--metrics="))
    {
        let metrics_args = parse_metrics_args(args)?;
        let runs_dir = if let Ok(dir) = std::env::var("QUAQUA_RUNS_DIR") {
            PathBuf::from(dir)
        } else {
            crate::workspace::workspace_root().join("runs")
        };
        let out = render_metrics(&runs_dir, &metrics_args)?;
        print!("{out}");
        return Ok(0);
    }

    let runs_dir = if let Ok(dir) = std::env::var("QUAQUA_RUNS_DIR") {
        PathBuf::from(dir)
    } else {
        crate::workspace::workspace_root().join("runs")
    };
    let db_path = runs_dir.join("quaqua.db");
    let report = render_status(&db_path, &runs_dir, herdr)?;
    print!("{report}");
    Ok(0)
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MetricsArgs {
    pub metrics: bool,
    pub since: String,
    pub owner: Option<String>,
    pub json: bool,
}

pub fn parse_metrics_args(args: &[String]) -> Result<MetricsArgs, String> {
    let mut parsed = MetricsArgs {
        metrics: false,
        since: "24h".to_string(),
        owner: None,
        json: false,
    };

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--metrics" {
            parsed.metrics = true;
        } else if let Some(val) = arg.strip_prefix("--metrics=") {
            parsed.metrics = true;
            if !val.is_empty() {
                parsed.since = val.to_string();
            }
        } else if arg == "--json" {
            parsed.json = true;
        } else if arg == "--since" {
            i += 1;
            if i < args.len() {
                parsed.since = args[i].clone();
            } else {
                return Err("missing value for --since".to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--since=") {
            parsed.since = val.to_string();
        } else if arg == "--owner" {
            i += 1;
            if i < args.len() {
                parsed.owner = Some(args[i].clone());
            } else {
                return Err("missing value for --owner".to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--owner=") {
            parsed.owner = Some(val.to_string());
        }
        i += 1;
    }
    let now_sec = crate::util::now_sec();
    let now_ms = now_sec * 1000;
    parse_since_cutoff(&parsed.since, now_ms)?;

    Ok(parsed)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineTelemetryItem {
    pub ts: String,
    #[serde(default = "default_pipeline_rec")]
    pub rec: String,
    pub task: String,
    pub repo: String,
    pub builder: String,
    pub reviewer: String,
    pub review: String,
    pub rounds: u32,
    #[serde(default)]
    pub review_changed_outcome: bool,
    pub status: String,
    pub minutes: u64,
}

fn default_pipeline_rec() -> String {
    "pipeline".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsReport {
    pub since: String,
    pub workers: Vec<WorkerTelemetryRow>,
    pub pipelines: Vec<PipelineTelemetryItem>,
    pub total: MetricsTotal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsTotal {
    pub workers: usize,
    pub usd: f64,
    #[serde(default)]
    pub unpriced: usize,
    #[serde(default)]
    pub claude_unpriced: usize,
    pub wall_sec: u64,
}

pub fn parse_since_cutoff(since: &str, now_ms: u64) -> Result<u64, String> {
    let trimmed = since.trim();
    if let Some(num_str) = trimmed.strip_suffix('h') {
        if let Ok(hours) = num_str.parse::<u64>() {
            let delta = hours.saturating_mul(3600).saturating_mul(1000);
            return Ok(now_ms.saturating_sub(delta));
        }
    } else if let Some(num_str) = trimmed.strip_suffix('d') {
        if let Ok(days) = num_str.parse::<u64>() {
            let delta = days.saturating_mul(86400).saturating_mul(1000);
            return Ok(now_ms.saturating_sub(delta));
        }
    } else if let Some(num_str) = trimmed.strip_suffix('m') {
        if let Ok(mins) = num_str.parse::<u64>() {
            let delta = mins.saturating_mul(60).saturating_mul(1000);
            return Ok(now_ms.saturating_sub(delta));
        }
    } else if let Some(ms) = crate::worker::prompt::cost::parse_iso_timestamp_ms(trimmed) {
        return Ok(ms);
    }
    Err(format!(
        "invalid --since value '{since}', expected <N>h, <N>d, <N>m, or ISO timestamp"
    ))
}

pub fn read_metrics(runs_dir: &Path, args: &MetricsArgs) -> MetricsReport {
    let now_sec = crate::util::now_sec();
    let now_ms = now_sec * 1000;
    let cutoff_ms = parse_since_cutoff(&args.since, now_ms).unwrap_or(0);

    let tele_path = runs_dir.join("telemetry.jsonl");
    let mut workers = Vec::new();
    let mut pipelines = Vec::new();

    if let Ok(file) = std::fs::File::open(&tele_path) {
        use std::io::BufRead;
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                continue;
            };

            let ts_ms = v
                .get("ts")
                .and_then(|t| t.as_str())
                .and_then(crate::worker::prompt::cost::parse_iso_timestamp_ms)
                .unwrap_or(0);

            if ts_ms < cutoff_ms {
                continue;
            }

            let rec = v.get("rec").and_then(|r| r.as_str());
            if rec == Some("worker") {
                if let Ok(row) = serde_json::from_str::<WorkerTelemetryRow>(trimmed) {
                    if let Some(o) = &args.owner {
                        if row.owner.as_deref() != Some(o.as_str()) {
                            continue;
                        }
                    }
                    workers.push(row);
                }
            } else {
                if let Ok(row) = serde_json::from_str::<PipelineTelemetryItem>(trimmed) {
                    pipelines.push(row);
                }
            }
        }
    }

    let mut total_usd = 0.0;
    let mut unpriced = 0usize;
    let mut claude_unpriced = 0usize;
    let mut total_wall_sec = 0u64;

    for w in &workers {
        if let Some(usd) = w.usd {
            total_usd += usd;
        } else {
            unpriced += 1;
            if w.harness
                .as_deref()
                .and_then(crate::harness::Harness::parse)
                == Some(crate::harness::Harness::Claude)
                || w.spend_src == "claude-tokens"
            {
                claude_unpriced += 1;
            }
        }
        if let Some(wall) = w.wall_sec {
            total_wall_sec += wall;
        }
    }
    let worker_count = workers.len();
    MetricsReport {
        since: args.since.clone(),
        workers,
        pipelines,
        total: MetricsTotal {
            workers: worker_count,
            usd: (total_usd * 10000.0).round() / 10000.0,
            unpriced,
            claude_unpriced,
            wall_sec: total_wall_sec,
        },
    }
}

fn format_token_metric(tok: Option<&Tokens>) -> String {
    let Some(t) = tok else {
        return "-".to_string();
    };
    let in_str = format_compact_number(t.in_tokens);
    let out_str = format_compact_number(t.out_tokens);
    let cache_str = format_compact_number(t.cache_read);
    format!("{in_str}/{out_str}/{cache_str}")
}

fn format_compact_number(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1_000_000.0;
        format!("{:.1}M", m)
    } else if n >= 1_000 {
        let k = (n as f64 / 1000.0).round() as u64;
        format!("{k}k")
    } else {
        format!("{n}")
    }
}

fn format_duration_metric(sec: u64) -> String {
    if sec >= 3600 {
        let h = sec / 3600;
        let m = ((sec % 3600) as f64 / 60.0).round() as u64;
        if m > 0 {
            format!("{h}h{m}m")
        } else {
            format!("{h}h")
        }
    } else if sec >= 60 {
        let m = (sec as f64 / 60.0).round() as u64;
        format!("{m}m")
    } else {
        format!("{sec}s")
    }
}

pub fn render_metrics_text(report: &MetricsReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "=== Metrics (runs/telemetry.jsonl, since {}) ===\n",
        report.since
    ));
    let name_width = report
        .workers
        .iter()
        .map(|w| w.name.len())
        .max()
        .unwrap_or(4)
        .max(17);

    out.push_str(&format!(
        "{:<name_width$} kind/harness       owner  exit  wall/budget  $/budget      tokens in/out/cache   pipeline\n",
        "name"
    ));

    for w in &report.workers {
        let name = &w.name;
        let kind_harness = format!(
            "{}/{}",
            w.kind.as_deref().unwrap_or("-"),
            w.harness.as_deref().unwrap_or("-")
        );
        let owner = w.owner.as_deref().unwrap_or("-");
        let exit = w
            .exit
            .map(|e| e.to_string())
            .unwrap_or_else(|| "-".to_string());

        let wall_str = w
            .wall_sec
            .map(format_duration_metric)
            .unwrap_or_else(|| "-".to_string());
        let budget_str = w
            .budget_sec
            .map(format_duration_metric)
            .unwrap_or_else(|| "-".to_string());
        let wall_budget = format!("{wall_str}/{budget_str}");

        let cost_str = match w.usd {
            Some(c) if c < 0.1 => format!("{c:.3}"),
            Some(c) => format!("{c:.2}"),
            None => "-".to_string(),
        };
        let cost_budget_str = match w.usd_budget {
            Some(b) => format!("{b:.2}"),
            None => "-".to_string(),
        };
        let cost_budget = format!("{cost_str}/{cost_budget_str}");

        let tokens_str = format_token_metric(w.tokens.as_ref());
        let pipeline_str = w.pipeline.as_deref().unwrap_or("-");

        out.push_str(&format!(
            "{name:<name_width$} {kind_harness:<18} {owner:<6} {exit:<5} {wall_budget:<12} {cost_budget:<13} {tokens_str:<21} {pipeline_str}\n"
        ));
    }

    if !report.pipelines.is_empty() {
        let len = report.pipelines.len();
        let (slice, extra) = if len > 8 {
            (&report.pipelines[len - 8..], len - 8)
        } else {
            (&report.pipelines[..], 0)
        };
        let mut p_strs: Vec<String> = slice
            .iter()
            .map(|p| format!("{} {} r{} {}m", p.task, p.status, p.rounds, p.minutes))
            .collect();
        if extra > 0 {
            p_strs.push(format!("+{extra} more"));
        }
        out.push_str(&format!("pipelines: {}\n", p_strs.join(" · ")));
    } else {
        out.push_str("pipelines: none\n");
    }

    let unpriced_str = if report.total.unpriced == 0 {
        String::new()
    } else if report.total.unpriced == report.total.claude_unpriced {
        format!(" (+{} claude unpriced)", report.total.unpriced)
    } else {
        format!(" (+{} unpriced)", report.total.unpriced)
    };
    let total_wall_str = format_duration_metric(report.total.wall_sec);
    out.push_str(&format!(
        "total: {} workers · ${:.2}{} · {}\n",
        report.total.workers, report.total.usd, unpriced_str, total_wall_str
    ));

    out
}

pub fn render_metrics(runs_dir: &Path, args: &MetricsArgs) -> Result<String, String> {
    let report = read_metrics(runs_dir, args);
    if args.json {
        serde_json::to_string_pretty(&report)
            .map(|s| format!("{s}\n"))
            .map_err(|e| format!("json serialization error: {e}"))
    } else {
        Ok(render_metrics_text(&report))
    }
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

        let report = format_status_report(Ok(&workflows), Ok(&[]), Ok(&[]));
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

        let report = format_status_report(Ok(&workflows), Ok(&[]), Ok(&[]));
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
    fn test_status_unreadable_read_marker_treated_as_unread() {
        let tmp = temp_test_dir("unreadable-read-marker");
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

        let now = std::time::SystemTime::now();
        let report_file = fs::File::open(&report_path).unwrap();
        let mut report_times = fs::FileTimes::new();
        report_times = report_times
            .set_modified(now)
            .set_accessed(now - Duration::from_secs(10));
        report_file.set_times(report_times).unwrap();

        // If read marker is a directory or otherwise unreadable for file metadata mtime
        let marker_path = tmp.join(".sude-e1.read");
        fs::create_dir(&marker_path).unwrap(); // is_file() is false, metadata/modified will not match a valid read marker

        let rows = scan_orchestration(&tmp, &mock).expect("scan orchestration");
        let e1 = rows
            .iter()
            .find(|r| r.name == "sude-e1")
            .expect("sude-e1 found");

        assert!(
            e1.flags.contains(&WarningFlag::WaiterLost),
            "unreadable or invalid .read marker must NOT suppress WAITER-LOST"
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

        let report = format_status_report(Err(&err_msg), Ok(&[]), Ok(&[]));
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

        let report = format_status_report(Ok(&[]), Err(&err_msg), Ok(&[]));
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

        let report = format_status_report(Ok(&[]), Err(&err_msg), Ok(&[]));
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

    #[test]
    fn test_status_verdict_stale_on_new_commit() {
        let tmp = temp_test_dir("stale-commit");
        let _cleaner = TempDirCleaner(tmp.clone());
        let repo = tmp.join("repo");
        fs::create_dir_all(&repo).unwrap();

        // Init git repo
        let git = Git::new();
        let _ = git.run_in(&repo, &["init", "-q"]).unwrap();
        let _ = git
            .run_in(&repo, &["config", "user.email", "test@test"])
            .unwrap();
        let _ = git.run_in(&repo, &["config", "user.name", "test"]).unwrap();
        let _ = git
            .run_in(&repo, &["commit", "--allow-empty", "-qm", "initial commit"])
            .unwrap();

        // Create branch quaqua/feat-stale at commit 1
        let _ = git
            .run_in(&repo, &["checkout", "-qb", "quaqua/feat-stale"])
            .unwrap();
        let _ = git
            .run_in(&repo, &["commit", "--allow-empty", "-qm", "commit 1"])
            .unwrap();
        let sha1 =
            String::from_utf8_lossy(&git.run_in(&repo, &["rev-parse", "HEAD"]).unwrap().stdout)
                .trim()
                .to_string();

        // Record verdict in journal at sha1
        let db_path = tmp.join("quaqua.db");
        let mut journal = SqliteJournal::open(&db_path).expect("open journal");
        let v1 = VerdictRecord {
            branch: "quaqua/feat-stale".to_string(),
            sha: sha1.clone(),
            level: VerdictLevel::ReviewClean,
            evidence: "verdict evidence".to_string(),
            verifier: "suhuynh-rev".to_string(),
            source: VerdictSource::Verifier,
            builder_kind: Some("sude".to_string()),
            verifier_kind: Some("suhuynh".to_string()),
            harness: "pi".to_string(),
        };
        journal
            .record_verdict("wf-stale", &v1)
            .expect("record verdict");

        // Before new commit: head == verdict SHA -> not stale
        let rows_before = query_branch_verdicts(&db_path, &repo, &git).expect("query before");
        assert_eq!(rows_before.len(), 1);
        assert_eq!(rows_before[0].branch, "quaqua/feat-stale");
        assert_eq!(rows_before[0].head_sha, sha1);
        assert_eq!(rows_before[0].verdict_sha, sha1);
        assert_eq!(rows_before[0].level, VerdictLevel::ReviewClean);
        assert!(!rows_before[0].stale);

        // Make new commit on the branch
        let _ = git
            .run_in(&repo, &["commit", "--allow-empty", "-qm", "commit 2 (new)"])
            .unwrap();
        let sha2 =
            String::from_utf8_lossy(&git.run_in(&repo, &["rev-parse", "HEAD"]).unwrap().stdout)
                .trim()
                .to_string();
        assert_ne!(sha1, sha2, "new commit must produce different SHA");

        // Query branch verdicts after new commit
        let rows_after = query_branch_verdicts(&db_path, &repo, &git).expect("query after");
        assert_eq!(rows_after.len(), 1);
        assert_eq!(rows_after[0].branch, "quaqua/feat-stale");
        assert_eq!(rows_after[0].head_sha, sha2);
        assert_eq!(rows_after[0].verdict_sha, sha1);
        assert_eq!(rows_after[0].level, VerdictLevel::ReviewClean);
        assert!(
            rows_after[0].stale,
            "verdict must be marked stale on new commit"
        );

        // Check format_status_report output contains stale flag
        let report = format_status_report(Ok(&[]), Ok(&[]), Ok(&rows_after));
        assert!(report.contains("quaqua/feat-stale"));
        assert!(report.contains("review-clean"));
        assert!(report.contains("stale"));
    }

    #[test]
    fn test_status_verdict_strongest_per_branch_and_sha() {
        let tmp = temp_test_dir("strongest-sha");
        let _cleaner = TempDirCleaner(tmp.clone());
        let repo = tmp.join("repo");
        fs::create_dir_all(&repo).unwrap();

        let git = Git::new();
        let _ = git.run_in(&repo, &["init", "-q"]).unwrap();
        let _ = git
            .run_in(&repo, &["config", "user.email", "test@test"])
            .unwrap();
        let _ = git.run_in(&repo, &["config", "user.name", "test"]).unwrap();
        let _ = git
            .run_in(&repo, &["commit", "--allow-empty", "-qm", "initial"])
            .unwrap();

        let _ = git
            .run_in(&repo, &["checkout", "-qb", "quaqua/feat-strong"])
            .unwrap();
        let _ = git
            .run_in(&repo, &["commit", "--allow-empty", "-qm", "work"])
            .unwrap();
        let sha =
            String::from_utf8_lossy(&git.run_in(&repo, &["rev-parse", "HEAD"]).unwrap().stdout)
                .trim()
                .to_string();

        let db_path = tmp.join("quaqua.db");
        let mut journal = SqliteJournal::open(&db_path).expect("open journal");

        // 1. Builder self-report
        let v_builder = VerdictRecord {
            branch: "quaqua/feat-strong".to_string(),
            sha: sha.clone(),
            level: VerdictLevel::UnitTestVerified,
            evidence: "builder unit tests pass".to_string(),
            verifier: "sude-build".to_string(),
            source: VerdictSource::SelfReport,
            builder_kind: Some("sude".to_string()),
            verifier_kind: None,
            harness: "omp".to_string(),
        };
        journal.record_verdict("wf-builder", &v_builder).unwrap();

        // 2. Verifier verdict (review-clean overrides self-report)
        let v_rev_clean = VerdictRecord {
            branch: "quaqua/feat-strong".to_string(),
            sha: sha.clone(),
            level: VerdictLevel::ReviewClean,
            evidence: "Verdict: clean".to_string(),
            verifier: "suhuynh-rev".to_string(),
            source: VerdictSource::Verifier,
            builder_kind: Some("sude".to_string()),
            verifier_kind: Some("suhuynh".to_string()),
            harness: "pi".to_string(),
        };
        journal
            .record_verdict("wf-rev-clean", &v_rev_clean)
            .unwrap();

        let rows = query_branch_verdicts(&db_path, &repo, &git).expect("query verdicts");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].branch, "quaqua/feat-strong");
        assert_eq!(rows[0].level, VerdictLevel::ReviewClean);
        assert!(rows[0].is_verified);

        // 3. Negative dominates: second verifier records verifier-failed
        let v_rev_failed = VerdictRecord {
            branch: "quaqua/feat-strong".to_string(),
            sha: sha.clone(),
            level: VerdictLevel::VerifierFailed,
            evidence: "Verdict: 1 findings".to_string(),
            verifier: "truonglao-rev".to_string(),
            source: VerdictSource::Verifier,
            builder_kind: Some("sude".to_string()),
            verifier_kind: Some("truonglao".to_string()),
            harness: "claude".to_string(),
        };
        journal
            .record_verdict("wf-rev-fail", &v_rev_failed)
            .unwrap();

        let rows2 = query_branch_verdicts(&db_path, &repo, &git).expect("query verdicts 2");
        assert_eq!(rows2.len(), 1);
        assert_eq!(
            rows2[0].level,
            VerdictLevel::VerifierFailed,
            "verifier-failed must dominate over review-clean"
        );
        assert!(!rows2[0].is_verified);

        let report = format_status_report(Ok(&[]), Ok(&[]), Ok(&rows2));
        assert!(report.contains("quaqua/feat-strong"));
        assert!(report.contains("verifier-failed"));
    }

    #[test]
    fn test_status_verdict_same_harness_shown() {
        let tmp = temp_test_dir("same-harness");
        let _cleaner = TempDirCleaner(tmp.clone());
        let repo = tmp.join("repo");
        fs::create_dir_all(&repo).unwrap();

        let git = Git::new();
        let _ = git.run_in(&repo, &["init", "-q"]).unwrap();
        let _ = git
            .run_in(&repo, &["config", "user.email", "test@test"])
            .unwrap();
        let _ = git.run_in(&repo, &["config", "user.name", "test"]).unwrap();
        let _ = git
            .run_in(&repo, &["commit", "--allow-empty", "-qm", "initial"])
            .unwrap();

        // Branch 1: detu (verifier, omp) verifying sude (builder, omp) -> same-harness
        let _ = git
            .run_in(&repo, &["checkout", "-qb", "quaqua/feat-same"])
            .unwrap();
        let _ = git
            .run_in(&repo, &["commit", "--allow-empty", "-qm", "work same"])
            .unwrap();
        let sha_same =
            String::from_utf8_lossy(&git.run_in(&repo, &["rev-parse", "HEAD"]).unwrap().stdout)
                .trim()
                .to_string();

        // Branch 2: suhuynh (verifier, pi) verifying sude (builder, omp) -> diff harness
        let _ = git
            .run_in(&repo, &["checkout", "-qb", "quaqua/feat-diff"])
            .unwrap();
        let _ = git
            .run_in(&repo, &["commit", "--allow-empty", "-qm", "work diff"])
            .unwrap();
        let sha_diff =
            String::from_utf8_lossy(&git.run_in(&repo, &["rev-parse", "HEAD"]).unwrap().stdout)
                .trim()
                .to_string();

        let db_path = tmp.join("quaqua.db");
        let mut journal = SqliteJournal::open(&db_path).expect("open journal");

        let v_same = VerdictRecord {
            branch: "quaqua/feat-same".to_string(),
            sha: sha_same,
            level: VerdictLevel::ReviewClean,
            evidence: "clean".to_string(),
            verifier: "detu-rev".to_string(),
            source: VerdictSource::Verifier,
            builder_kind: Some("sude".to_string()),
            verifier_kind: Some("detu".to_string()),
            harness: "omp".to_string(), // detu is omp, sude is omp
        };
        journal.record_verdict("wf-same", &v_same).unwrap();

        let v_diff = VerdictRecord {
            branch: "quaqua/feat-diff".to_string(),
            sha: sha_diff,
            level: VerdictLevel::ReviewClean,
            evidence: "clean".to_string(),
            verifier: "suhuynh-rev".to_string(),
            source: VerdictSource::Verifier,
            builder_kind: Some("sude".to_string()),
            verifier_kind: Some("suhuynh".to_string()),
            harness: "pi".to_string(), // suhuynh is pi, sude is omp
        };
        journal.record_verdict("wf-diff", &v_diff).unwrap();

        let rows = query_branch_verdicts(&db_path, &repo, &git).expect("query verdicts");
        assert_eq!(rows.len(), 2);

        let row_diff = rows
            .iter()
            .find(|r| r.branch == "quaqua/feat-diff")
            .unwrap();
        assert!(!row_diff.same_harness);

        let row_same = rows
            .iter()
            .find(|r| r.branch == "quaqua/feat-same")
            .unwrap();
        assert!(
            row_same.same_harness,
            "must show same-harness when harnesses match"
        );

        let report = format_status_report(Ok(&[]), Ok(&[]), Ok(&rows));
        assert!(report.contains("same-harness"));
        let lines: Vec<&str> = report
            .lines()
            .filter(|l| l.contains("quaqua/feat-same"))
            .collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("same-harness"));
    }
}

#[cfg(test)]
pub(crate) mod metrics {
    use super::*;

    #[test]
    fn test_status_metrics_rendering_and_json_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("status-metrics-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let runs_dir = tmp.join("runs");
        std::fs::create_dir_all(&runs_dir).unwrap();

        let tele_path = runs_dir.join("telemetry.jsonl");

        // Old legacy row without rec
        let old_pipeline_row = r#"{"ts":"2026-10-21T10:00:00+07:00","task":"w2","repo":"/Personal/baroiplayer","builder":"sude","reviewer":"quansu","review":"run","rounds":2,"review_changed_outcome":false,"status":"clean","minutes":84}"#;

        // New pipeline row with rec: "pipeline"
        let new_pipeline_row = r#"{"ts":"2026-10-21T11:00:00+07:00","rec":"pipeline","task":"w6","repo":"/Personal/baroiplayer","builder":"sude","reviewer":"quansu","review":"run","rounds":1,"review_changed_outcome":true,"status":"blocked","minutes":27}"#;

        // Worker rows
        let worker_row1 = r#"{"ts":"2026-10-21T12:00:00+07:00","rec":"worker","name":"tieude-p74lock","kind":"tieude","harness":"opencode","owner":"cos","pipeline":null,"repo":"~/quaqua","prompts":1,"exit":0,"wall_sec":409,"budget_sec":1500,"usd":0.0458,"usd_budget":1.0,"tokens":{"in":94574,"out":16991,"reasoning":0,"cache_read":3754496,"cache_write":0},"spend_src":"opencode-db","model":"deepseek-v4.1-flash"}"#;

        let worker_row2 = r#"{"ts":"2026-10-21T12:10:00+07:00","rec":"worker","name":"detu-claudediff","kind":"detu","harness":"omp","owner":"cos","pipeline":null,"repo":"~/Personal/baroiplayer","prompts":1,"exit":0,"wall_sec":420,"budget_sec":2400,"usd":0.41,"usd_budget":2.0,"tokens":{"in":17371,"out":952,"reasoning":0,"cache_read":0,"cache_write":0},"spend_src":"omp-session","model":"gemini-3.8-flash"}"#;

        let worker_row3 = r#"{"ts":"2026-10-21T12:20:00+07:00","rec":"worker","name":"truonglao-metrics","kind":"truonglao","harness":"claude","owner":"cos","pipeline":null,"repo":"~/Personal/baroiplayer","prompts":1,"exit":0,"wall_sec":2280,"budget_sec":2400,"usd":null,"usd_budget":3.0,"tokens":{"in":6995,"out":1262,"reasoning":0,"cache_read":1262078,"cache_write":0},"spend_src":"claude-tokens","model":"claude-opus-5-5"}"#;

        std::fs::write(
            &tele_path,
            format!("{old_pipeline_row}\n{new_pipeline_row}\n{worker_row1}\n{worker_row2}\n{worker_row3}\n"),
        ).unwrap();

        // Test text rendering
        let args = MetricsArgs {
            metrics: true,
            since: "2026-10-21T00:00:00Z".to_string(),
            owner: None,
            json: false,
        };

        let rendered = render_metrics(&runs_dir, &args).unwrap();
        assert!(
            rendered.contains("=== Metrics (runs/telemetry.jsonl, since 2026-10-21T00:00:00Z) ===")
        );
        assert!(rendered.contains("tieude-p74lock"));
        assert!(rendered.contains("detu-claudediff"));
        assert!(rendered.contains("truonglao-metrics"));
        assert!(rendered.contains("0.046/1.00"));
        assert!(rendered.contains("0.41/2.00"));
        assert!(rendered.contains("-/3.00"));
        assert!(rendered.contains("95k/17k/3.8M"));
        assert!(rendered.contains("pipelines: w2 clean r2 84m · w6 blocked r1 27m"));
        assert!(rendered.contains("total: 3 workers · $0.46 (+1 claude unpriced)"));

        // Test --json round-trip
        let json_args = MetricsArgs {
            metrics: true,
            since: "2026-10-21T00:00:00Z".to_string(),
            owner: None,
            json: true,
        };
        let json_str = render_metrics(&runs_dir, &json_args).unwrap();
        let parsed_report: MetricsReport = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed_report.workers.len(), 3);
        assert_eq!(parsed_report.pipelines.len(), 2);
        assert_eq!(parsed_report.total.workers, 3);
        assert!((parsed_report.total.usd - 0.4558).abs() < 1e-3);
        assert_eq!(parsed_report.total.claude_unpriced, 1);

        // Test --owner filter
        let cos_args = MetricsArgs {
            metrics: true,
            since: "2026-10-21T00:00:00Z".to_string(),
            owner: Some("cos".to_string()),
            json: false,
        };
        let cos_rendered = render_metrics(&runs_dir, &cos_args).unwrap();
        assert!(cos_rendered.contains("tieude-p74lock"));

        let other_args = MetricsArgs {
            metrics: true,
            since: "2026-10-21T00:00:00Z".to_string(),
            owner: Some("other-owner".to_string()),
            json: false,
        };
        let other_rendered = render_metrics(&runs_dir, &other_args).unwrap();
        assert!(!other_rendered.contains("tieude-p74lock"));
        assert!(other_rendered.contains("total: 0 workers"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_worker_telemetry_row_roundtrip_unpriced_and_pipeline_cap() {
        let tmp = std::env::temp_dir().join(format!("status-metrics-ext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let runs_dir = tmp.join("runs");
        std::fs::create_dir_all(&runs_dir).unwrap();
        let tele_path = runs_dir.join("telemetry.jsonl");

        // 1. Round-trip WorkerTelemetryRow with repo ~/quaqua and unpriced non-claude worker
        let row = WorkerTelemetryRow {
            ts: "2026-10-21T12:00:00+07:00".to_string(),
            rec: "worker".to_string(),
            name: "sude-rt".to_string(),
            kind: Some("sude".to_string()),
            harness: Some("omp".to_string()),
            owner: Some("cos".to_string()),
            pipeline: None,
            repo: Some("~/quaqua".to_string()),
            prompts: Some(1),
            exit: Some(0),
            wall_sec: Some(300),
            budget_sec: Some(1500),
            usd: None, // OMP reader failed
            usd_budget: Some(1.0),
            tokens: None,
            spend_src: "omp-session-not-found".to_string(),
            model: None,
        };
        let row_json = serde_json::to_string(&row).unwrap();

        // 2. Ten pipeline items to test newest 8 cap and +N more
        let mut lines = vec![row_json];
        for i in 1..=10 {
            let p = PipelineTelemetryItem {
                ts: format!("2026-10-21T12:{:02}:00+07:00", i),
                rec: "pipeline".to_string(),
                task: format!("task-{i}"),
                repo: "~/quaqua".to_string(),
                builder: "sude".to_string(),
                reviewer: "suhuynh".to_string(),
                review: "run".to_string(),
                rounds: 1,
                review_changed_outcome: false,
                status: "clean".to_string(),
                minutes: 10 + i as u64,
            };
            lines.push(serde_json::to_string(&p).unwrap());
        }
        std::fs::write(&tele_path, lines.join("\n") + "\n").unwrap();

        let args = MetricsArgs {
            metrics: true,
            since: "24h".to_string(),
            owner: None,
            json: false,
        };

        let rendered = render_metrics(&runs_dir, &args).unwrap();
        // Round-trip repo was read properly
        let report = read_metrics(&runs_dir, &args);
        assert_eq!(report.workers.len(), 1);
        assert_eq!(report.workers[0].repo.as_deref(), Some("~/quaqua"));
        // Unpriced marker is general (+1 unpriced), not (+1 claude unpriced)
        assert_eq!(report.total.unpriced, 1);
        assert_eq!(report.total.claude_unpriced, 0);
        assert!(rendered.contains("total: 1 workers · $0.00 (+1 unpriced)"));

        // Pipelines capped at newest 8 (task-3 to task-10), task-1 and task-2 omitted
        assert!(rendered.contains("+2 more"));
        assert!(rendered.contains("task-10 clean r1 20m"));
        assert!(rendered.contains("task-3 clean r1 13m"));
        assert!(!rendered.contains("task-1 clean"));
        assert!(!rendered.contains("task-2 clean"));

        // 3. Test --since cutoff
        let now_sec = crate::util::now_sec();
        let now_ms = now_sec * 1000;
        let c_24h = parse_since_cutoff("24h", now_ms).unwrap();
        let c_7d = parse_since_cutoff("7d", now_ms).unwrap();
        assert!(c_24h > c_7d);

        // Nit 10: large days value saturates without overflow
        let c_overflow = parse_since_cutoff("999999999999d", now_ms).unwrap();
        assert_eq!(c_overflow, 0);

        // Nit 11: invalid --since value is rejected with Err
        assert!(parse_metrics_args(&[
            "--metrics".to_string(),
            "--since".to_string(),
            "1w".to_string()
        ])
        .is_err());
        assert!(
            parse_metrics_args(&["--metrics".to_string(), "--since=invalid".to_string()]).is_err()
        );

        // Nit 14: long names expand name_width
        let long_worker = WorkerTelemetryRow {
            ts: "2026-10-21T12:00:00+07:00".to_string(),
            rec: "worker".to_string(),
            name: "suhuynh-metricsrev".to_string(),
            kind: Some("suhuynh".to_string()),
            harness: Some("omp".to_string()),
            owner: Some("cos".to_string()),
            pipeline: None,
            repo: Some("~/quaqua".to_string()),
            prompts: Some(1),
            exit: Some(0),
            wall_sec: Some(300),
            budget_sec: Some(1500),
            usd: Some(1.23),
            usd_budget: Some(2.0),
            tokens: None,
            spend_src: "omp-session".to_string(),
            model: None,
        };
        let long_report = MetricsReport {
            since: "24h".to_string(),
            workers: vec![long_worker],
            pipelines: vec![],
            total: MetricsTotal {
                workers: 1,
                usd: 1.23,
                unpriced: 0,
                claude_unpriced: 0,
                wall_sec: 300,
            },
        };
        let rendered_long = render_metrics_text(&long_report);
        // 18 chars for suhuynh-metricsrev means header "name" is padded to 18 chars
        assert!(rendered_long.contains("name               kind/harness"));
        assert!(rendered_long.contains("suhuynh-metricsrev suhuynh/omp"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
