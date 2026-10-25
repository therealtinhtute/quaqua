//! Event-driven monitoring, inbox digest with per-reader cursors, and herdr event hook.

use std::path::Path;
use serde::{Deserialize, Serialize};

use super::{FarmBus, FarmEvent, FarmEventRecord};
use crate::bus::mail::MailMessage;
use crate::herdr::HerdrOps;

/// Digest of unacked mail and incidents for a consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxDigest {
    pub unacked_mail: Vec<MailMessage>,
    pub incidents: Vec<FarmEventRecord>,
    pub prev_cursor: u64,
    pub new_cursor: u64,
}

impl InboxDigest {
    /// Renders the inbox digest into a human- and agent-readable string.
    pub fn render(&self, consumer: &str) -> String {
        let mut out = String::new();
        if !self.unacked_mail.is_empty() {
            for msg in &self.unacked_mail {
                let tag = if msg.deliveries > 1 {
                    format!(" (redelivery #{})", msg.deliveries)
                } else {
                    String::new()
                };
                out.push_str(&format!("#{} {} {}{}\n", msg.id, msg.from_name, msg.kind, tag));
                if !msg.body.trim().is_empty() {
                    out.push_str(&format!("{}\n", msg.body.trim_end()));
                }
                out.push('\n');
            }
        }
        if !self.incidents.is_empty() {
            for ev in &self.incidents {
                let key = ev.incident_key.as_deref().unwrap_or("none");
                let actor = ev.actor.as_deref().unwrap_or("-");
                let subj = ev.subject.as_deref().unwrap_or("-");
                out.push_str(&format!(
                    "! #{} {} {} [{} -> {}]\n",
                    ev.seq, ev.event_type, key, actor, subj
                ));
            }
        }
        if out.is_empty() {
            format!("no unacked mail or new incidents for '{consumer}'\n")
        } else {
            out
        }
    }
}

impl FarmBus {
    /// Loads incidents (events where `incident_key IS NOT NULL`) ordered by `seq` ASC,
    /// strictly after `since_seq`.
    ///
    /// If `consumer` is not `"cos"`, only incidents where `owner` or `project` matches
    /// `consumer`, or where `owner IS NULL`, are returned.
    pub fn load_incidents(&self, since_seq: u64, consumer: &str) -> rusqlite::Result<Vec<FarmEventRecord>> {
        let conn = self.conn_for_test();
        crate::engine::sqlite::retry_while_busy(|| {
            if consumer == "cos" {
                let mut stmt = conn.prepare_cached(
                    r#"
                    SELECT seq, ts_ms, type, actor, subject, owner, project, incident_key, payload_json
                    FROM farm_events
                    WHERE seq > ?1 AND incident_key IS NOT NULL
                    ORDER BY seq ASC
                    "#,
                )?;
                let rows = stmt.query_map(rusqlite::params![since_seq as i64], |row| {
                    Ok(FarmEventRecord {
                        seq: row.get::<_, i64>(0)? as u64,
                        ts_ms: row.get::<_, i64>(1)? as u64,
                        event_type: row.get(2)?,
                        actor: row.get(3)?,
                        subject: row.get(4)?,
                        owner: row.get(5)?,
                        project: row.get(6)?,
                        incident_key: row.get(7)?,
                        payload_json: row.get(8)?,
                    })
                })?;
                let mut res = Vec::new();
                for r in rows {
                    res.push(r?);
                }
                Ok(res)
            } else {
                let mut stmt = conn.prepare_cached(
                    r#"
                    SELECT seq, ts_ms, type, actor, subject, owner, project, incident_key, payload_json
                    FROM farm_events
                    WHERE seq > ?1 AND incident_key IS NOT NULL
                      AND (owner = ?2 OR project = ?2 OR owner IS NULL)
                    ORDER BY seq ASC
                    "#,
                )?;
                let rows = stmt.query_map(rusqlite::params![since_seq as i64, consumer], |row| {
                    Ok(FarmEventRecord {
                        seq: row.get::<_, i64>(0)? as u64,
                        ts_ms: row.get::<_, i64>(1)? as u64,
                        event_type: row.get(2)?,
                        actor: row.get(3)?,
                        subject: row.get(4)?,
                        owner: row.get(5)?,
                        project: row.get(6)?,
                        incident_key: row.get(7)?,
                        payload_json: row.get(8)?,
                    })
                })?;
                let mut res = Vec::new();
                for r in rows {
                    res.push(r?);
                }
                Ok(res)
            }
        })
    }

    /// Reads the inbox digest for `consumer`:
    /// - Checks lazy unacked mail incidents.
    /// - Reads unacked mail addressed to `consumer`.
    /// - Reads incidents since the consumer's cursor.
    /// - Advances the cursor to the latest event sequence.
    ///
    /// A repeated read with no new events is idempotent: the cursor remains unchanged.
    pub fn inbox_digest(&self, runs_dir: &Path, consumer: &str) -> Result<InboxDigest, String> {
        let prev_cursor = self
            .get_cursor(consumer)
            .map_err(|e| format!("cannot get cursor: {e}"))?
            .unwrap_or(0);

        // Check for any unacked mail incidents lazily
        {
            let conn = self.conn_for_test();
            let timeout = crate::bus::mail::get_mail_ack_timeout_ms();
            let _ = crate::bus::mail::check_unacked_incidents(&conn, timeout);
        }

        let unacked_mail = crate::bus::mail::read_inbox(runs_dir, consumer)?;
        let incidents = self
            .load_incidents(prev_cursor, consumer)
            .map_err(|e| format!("cannot load incidents: {e}"))?;

        let new_cursor = if let Some(max_seq) = incidents.iter().map(|e| e.seq).max() {
            if max_seq > prev_cursor {
                self.set_cursor(consumer, max_seq)
                    .map_err(|e| format!("cannot set cursor: {e}"))?;
                max_seq
            } else {
                prev_cursor
            }
        } else {
            prev_cursor
        };

        Ok(InboxDigest {
            unacked_mail,
            incidents,
            prev_cursor,
            new_cursor,
        })
    }
}

/// Handles an incoming herdr plugin hook event:
/// - Extracts pane, agent, and status.
/// - On `pane.agent_status_changed` where status is `blocked`, records an incident with
///   incident_key `blocked:<pane>:<status_since>`.
/// - If newly inserted, sends a desktop notification via `herdr notification show`.
/// - Returns `Ok(true)` if an incident was newly recorded and notification sent,
///   `Ok(false)` if duplicate or non-incident event.
pub fn handle_event_hook<H: HerdrOps>(
    runs_dir: &Path,
    event_name: Option<&str>,
    event_json: Option<&str>,
    herdr: &H,
) -> Result<bool, String> {
    let ej = match event_json {
        Some(s) if !s.trim().is_empty() => s,
        _ => return Ok(false),
    };

    let val: serde_json::Value = match serde_json::from_str(ej) {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };

    let ev_name = event_name
        .or_else(|| val.get("type").and_then(|v| v.as_str()))
        .or_else(|| val.get("event").and_then(|v| v.as_str()))
        .unwrap_or("");

    let pid = val
        .get("pane_id")
        .or_else(|| val.get("data").and_then(|d| d.get("pane_id")))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let agent_status = val
        .get("agent_status")
        .or_else(|| val.get("data").and_then(|d| d.get("agent_status")))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let agent = val
        .get("agent")
        .or_else(|| val.get("data").and_then(|d| d.get("agent")))
        .and_then(|v| v.as_str());

    let subject = if let Some(a) = agent {
        if !a.is_empty() { a } else { pid }
    } else {
        pid
    };

    let db_path = runs_dir.join("quaqua.db");
    let bus = match FarmBus::open(&db_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("quaqua events hook: cannot open bus at {}: {e}", db_path.display());
            return Ok(false);
        }
    };

    if ev_name == "pane.agent_status_changed" || ev_name == "pane_agent_status_changed" {
        if agent_status == "blocked" {
            let incident_key = val
                .get("incident_key")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    let status_since = val
                        .get("status_since")
                        .or_else(|| val.get("data").and_then(|d| d.get("status_since")))
                        .or_else(|| val.get("state_labels").and_then(|l| l.get("status_since")))
                        .and_then(|v| {
                            v.as_str()
                                .map(String::from)
                                .or_else(|| v.as_i64().map(|n| n.to_string()))
                        });
                    match status_since {
                        Some(ts) => format!("blocked:{pid}:{ts}"),
                        None => format!("blocked:{pid}"),
                    }
                });

            let mut ev = FarmEvent::new(
                "blocked",
                Some("herdr"),
                Some(subject),
                None::<String>,
                None::<String>,
            )
            .with_incident_key(incident_key.clone());

            if !ej.is_empty() && ej != "{}" {
                ev = ev.with_payload_json(ej);
            }

            let inserted = bus.record(&ev).map_err(|e| format!("cannot record event: {e}"))?;
            if inserted {
                let title = "quaqua: blocked";
                let body = format!("Agent '{subject}' is blocked");
                let _ = herdr.notification_show(title, &body, Some("request"));
                return Ok(true);
            }
            return Ok(false);
        }
    } else if ev_name == "pane.exited" || ev_name == "pane_exited" {
        let incident_key = val
            .get("incident_key")
            .and_then(|v| v.as_str())
            .map(String::from);

        let mut ev = FarmEvent::new(
            "agent_exited",
            Some("herdr"),
            Some(subject),
            None::<String>,
            None::<String>,
        );
        if let Some(key) = incident_key {
            ev = ev.with_incident_key(key);
        }
        if !ej.is_empty() && ej != "{}" {
            ev = ev.with_payload_json(ej);
        }
        let _ = bus.record(&ev);
        return Ok(false);
    }

    Ok(false)
}

/// Entry point for `quaqua events hook` CLI command.
pub fn run_events_hook() -> Result<i32, String> {
    let runs_dir = if let Ok(dir) = std::env::var("QUAQUA_RUNS_DIR") {
        std::path::PathBuf::from(dir)
    } else {
        crate::workspace::workspace_root().join("runs")
    };

    let event_name = std::env::var("HERDR_PLUGIN_EVENT").ok();
    let event_json = std::env::var("HERDR_PLUGIN_EVENT_JSON").ok();

    let herdr = crate::herdr::Herdr::new();
    if let Err(e) = handle_event_hook(&runs_dir, event_name.as_deref(), event_json.as_deref(), &herdr) {
        eprintln!("quaqua events hook error: {e}");
    }
    Ok(0)
}
