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
    /// Queries the latest event type and incident key recorded for `subject`.
    pub fn latest_event_for_subject(&self, subject: &str) -> Option<(String, Option<String>)> {
        let conn = self.conn_for_test();
        crate::engine::sqlite::retry_while_busy(|| {
            conn.query_row(
                "SELECT type, incident_key FROM farm_events WHERE subject = ?1 ORDER BY seq DESC LIMIT 1",
                rusqlite::params![subject],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .ok()
    }

    /// Checks for new items for `consumer` during `quaqua inbox --follow`.
    ///
    /// Yields only unacked mail with id > `last_seen_mail_id` without bumping deliveries,
    /// and new incidents since cursor.
    pub fn follow_tick(
        &self,
        _runs_dir: &Path,
        consumer: &str,
        last_seen_mail_id: &mut u64,
    ) -> Result<InboxDigest, String> {
        let prev_cursor = self
            .get_cursor(consumer)
            .map_err(|e| format!("cannot get cursor: {e}"))?
            .unwrap_or(0);

        {
            let conn = self.conn_for_test();
            let timeout = crate::bus::mail::get_mail_ack_timeout_ms();
            let _ = crate::bus::mail::check_unacked_incidents(&conn, timeout);
        }

        let new_mail = {
            let conn = self.conn_for_test();
            let mut stmt = conn
                .prepare_cached(
                    r#"
                    SELECT id, msg_key, ts_ms, from_name, from_role, to_name, kind, project, reply_to,
                           body, deliveries, last_delivered_ms, acked_ms, acked_by
                    FROM mail
                    WHERE to_name = ?1 AND acked_ms IS NULL AND id > ?2
                    ORDER BY id ASC
                    "#,
                )
                .map_err(|e| e.to_string())?;

            let rows = stmt
                .query_map(rusqlite::params![consumer, *last_seen_mail_id as i64], |row| {
                    Ok(MailMessage {
                        id: row.get::<_, i64>(0)? as u64,
                        msg_key: row.get(1)?,
                        ts_ms: row.get::<_, i64>(2)? as u64,
                        from_name: row.get(3)?,
                        from_role: row.get(4)?,
                        to_name: row.get(5)?,
                        kind: row.get(6)?,
                        project: row.get(7)?,
                        reply_to: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
                        body: row.get(9)?,
                        deliveries: row.get::<_, i64>(10)? as u64,
                        last_delivered_ms: row.get::<_, Option<i64>>(11)?.map(|v| v as u64),
                        acked_ms: row.get::<_, Option<i64>>(12)?.map(|v| v as u64),
                        acked_by: row.get(13)?,
                    })
                })
                .map_err(|e| e.to_string())?;

            let mut msgs = Vec::new();
            for r in rows {
                msgs.push(r.map_err(|e| e.to_string())?);
            }
            msgs
        };

        if let Some(max_mail_id) = new_mail.iter().map(|m| m.id).max() {
            *last_seen_mail_id = max_mail_id;
        }

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
            unacked_mail: new_mail,
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

    let subject_str = if !pid.is_empty() {
        pid.to_string()
    } else if let Some(a) = agent {
        if !a.is_empty() {
            a.to_string()
        } else {
            format!("anon-{}", crate::bus::now_ms())
        }
    } else {
        format!("anon-{}", crate::bus::now_ms())
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
            let incident_key = if let Some(k) = val.get("incident_key").and_then(|v| v.as_str()) {
                k.to_string()
            } else if let Some(ts) = val
                .get("status_since")
                .or_else(|| val.get("data").and_then(|d| d.get("status_since")))
                .or_else(|| val.get("state_labels").and_then(|l| l.get("status_since")))
                .and_then(|v| v.as_str().map(String::from).or_else(|| v.as_i64().map(|n| n.to_string())))
            {
                format!("blocked:{subject_str}:{ts}")
            } else {
                let latest = bus.latest_event_for_subject(&subject_str);
                match &latest {
                    Some((typ, Some(prev_key))) if typ == "blocked" => {
                        prev_key.clone()
                    }
                    _ => {
                        let now = crate::bus::now_ms();
                        format!("blocked:{subject_str}:{now}")
                    }
                }
            };

            let mut ev = FarmEvent::new(
                "blocked",
                Some("herdr"),
                Some(&subject_str),
                None::<String>,
                None::<String>,
            )
            .with_incident_key(incident_key);

            if !ej.is_empty() && ej != "{}" {
                ev = ev.with_payload_json(ej);
            }

            let inserted = bus.record(&ev).map_err(|e| format!("cannot record event: {e}"))?;
            if inserted {
                let title = "quaqua: blocked";
                let body = format!("Agent '{subject_str}' is blocked");
                let _ = herdr.notification_show(title, &body, Some("request"));
                return Ok(true);
            }
            return Ok(false);
        } else if !agent_status.is_empty() {
            let prev = bus.latest_event_for_subject(&subject_str);
            if let Some((typ, _)) = &prev {
                if typ == "blocked" {
                    let ev = FarmEvent::new(
                        "agent_status",
                        Some("herdr"),
                        Some(&subject_str),
                        None::<String>,
                        None::<String>,
                    )
                    .with_payload_json(ej);
                    let _ = bus.record(&ev);
                }
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
            Some(&subject_str),
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
    let timeout_ms: u64 = std::env::var("QUAQUA_HOOK_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1500);

    // Watchdog thread ensuring hard deadline: exits 0 if exceeded
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(timeout_ms));
        eprintln!("quaqua events hook: hard deadline exceeded; exiting 0 to avoid blocking herdr");
        std::process::exit(0);
    });

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
