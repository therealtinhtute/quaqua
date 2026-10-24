//! Farm bus: SQLite event bus for farm coordination and monitoring.
//!
//! Stores `farm_events` and `cursors` in `runs/quaqua.db`.
//! Uses shared SQLite connection opener with WAL mode, busy timeout, and retry_while_busy.

pub mod mail;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use serde::{Deserialize, Serialize};

/// Returns the current timestamp in milliseconds, respecting virtual clock if set.
pub fn now_ms() -> u64 {
    if let Ok(clock_file) = std::env::var("PROMPT_WORKER_CLOCK_FILE") {
        if let Ok(c) = std::fs::read_to_string(&clock_file) {
            if let Ok(v) = c.trim().parse::<u64>() {
                return v.saturating_mul(1000);
            }
        }
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// An event to be recorded in the `farm_events` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FarmEvent {
    pub ts_ms: u64,
    pub event_type: String,
    pub actor: Option<String>,
    pub subject: Option<String>,
    pub owner: Option<String>,
    pub project: Option<String>,
    pub incident_key: Option<String>,
    pub payload_json: String,
}

impl FarmEvent {
    pub fn new(
        event_type: impl Into<String>,
        actor: Option<impl Into<String>>,
        subject: Option<impl Into<String>>,
        owner: Option<impl Into<String>>,
        project: Option<impl Into<String>>,
    ) -> Self {
        Self {
            ts_ms: now_ms(),
            event_type: event_type.into(),
            actor: actor.map(Into::into),
            subject: subject.map(Into::into),
            owner: owner.map(Into::into),
            project: project.map(Into::into),
            incident_key: None,
            payload_json: "{}".to_string(),
        }
    }

    pub fn with_incident_key(mut self, key: impl Into<String>) -> Self {
        self.incident_key = Some(key.into());
        self
    }

    pub fn with_payload_json(mut self, payload: impl Into<String>) -> Self {
        self.payload_json = payload.into();
        self
    }
}

/// A stored farm event loaded from the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FarmEventRecord {
    pub seq: u64,
    pub ts_ms: u64,
    pub event_type: String,
    pub actor: Option<String>,
    pub subject: Option<String>,
    pub owner: Option<String>,
    pub project: Option<String>,
    pub incident_key: Option<String>,
    pub payload_json: String,
}

/// SQLite-backed event bus.
pub struct FarmBus {
    conn: Mutex<rusqlite::Connection>,
    path: PathBuf,
}

impl FarmBus {
    pub fn default_path() -> PathBuf {
        if let Ok(dir) = std::env::var("QUAQUA_RUNS_DIR") {
            PathBuf::from(dir).join("quaqua.db")
        } else {
            crate::workspace::workspace_root()
                .join("runs")
                .join("quaqua.db")
        }
    }

    /// Opens or creates the bus at the default path.
    pub fn new() -> rusqlite::Result<Self> {
        Self::open(&Self::default_path())
    }

    /// Opens or creates the bus at `path`, initializing schema idempotently.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = crate::engine::sqlite::open_conn(path)?;
        crate::engine::sqlite::retry_while_busy(|| {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS farm_events (
                    seq INTEGER PRIMARY KEY AUTOINCREMENT,
                    ts_ms INTEGER NOT NULL,
                    type TEXT NOT NULL,
                    actor TEXT,
                    subject TEXT,
                    owner TEXT,
                    project TEXT,
                    incident_key TEXT UNIQUE,
                    payload_json TEXT NOT NULL DEFAULT '{}'
                );
                CREATE INDEX IF NOT EXISTS farm_events_owner ON farm_events(owner, seq);
                CREATE TABLE IF NOT EXISTS cursors (
                    consumer TEXT PRIMARY KEY,
                    seq INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS mail (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    msg_key     TEXT NOT NULL UNIQUE,
                    ts_ms       INTEGER NOT NULL,
                    from_name   TEXT NOT NULL,
                    from_role   TEXT NOT NULL,
                    to_name     TEXT NOT NULL,
                    kind        TEXT NOT NULL,
                    project     TEXT,
                    reply_to    INTEGER REFERENCES mail(id),
                    body        TEXT NOT NULL CHECK (length(body) <= 4096),
                    deliveries  INTEGER NOT NULL DEFAULT 0,
                    last_delivered_ms INTEGER,
                    acked_ms    INTEGER,
                    acked_by    TEXT
                );
                CREATE INDEX IF NOT EXISTS mail_unacked ON mail(to_name, acked_ms, id);
                "#,
            )
        })?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn conn_for_test(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        self.conn.lock().unwrap()
    }


    /// Records an event in `farm_events`.
    ///
    /// If `event.incident_key` is set and a row with that key already exists,
    /// the insert is ignored and this method returns `Ok(false)`.
    /// Otherwise, the event is inserted and this method returns `Ok(true)`.
    pub fn record(&self, event: &FarmEvent) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        crate::engine::sqlite::retry_while_busy(|| {
            let mut stmt = conn.prepare_cached(
                r#"
                INSERT OR IGNORE INTO farm_events (
                    ts_ms, type, actor, subject, owner, project, incident_key, payload_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
            )?;
            let rows = stmt.execute(rusqlite::params![
                event.ts_ms as i64,
                event.event_type,
                event.actor,
                event.subject,
                event.owner,
                event.project,
                event.incident_key,
                event.payload_json,
            ])?;
            Ok(rows > 0)
        })
    }

    /// Loads all recorded events ordered by `seq` ASC, optionally filtered to `seq > since_seq`.
    pub fn load_events(&self, since_seq: u64) -> rusqlite::Result<Vec<FarmEventRecord>> {
        let conn = self.conn.lock().unwrap();
        crate::engine::sqlite::retry_while_busy(|| {
            let mut stmt = conn.prepare_cached(
                r#"
                SELECT seq, ts_ms, type, actor, subject, owner, project, incident_key, payload_json
                FROM farm_events
                WHERE seq > ?1
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
        })
    }

    /// Returns the cursor for `consumer`, if set.
    pub fn get_cursor(&self, consumer: &str) -> rusqlite::Result<Option<u64>> {
        let conn = self.conn.lock().unwrap();
        crate::engine::sqlite::retry_while_busy(|| {
            let mut stmt = conn.prepare_cached("SELECT seq FROM cursors WHERE consumer = ?1")?;
            let mut rows = stmt.query(rusqlite::params![consumer])?;
            if let Some(row) = rows.next()? {
                let seq: i64 = row.get(0)?;
                Ok(Some(seq as u64))
            } else {
                Ok(None)
            }
        })
    }

    /// Sets or updates the cursor for `consumer`.
    pub fn set_cursor(&self, consumer: &str, seq: u64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        crate::engine::sqlite::retry_while_busy(|| {
            let mut stmt = conn.prepare_cached(
                r#"
                INSERT INTO cursors (consumer, seq) VALUES (?1, ?2)
                ON CONFLICT(consumer) DO UPDATE SET seq = excluded.seq
                "#,
            )?;
            stmt.execute(rusqlite::params![consumer, seq as i64])?;
            Ok(())
        })
    }
}

/// Safely records a farm event to the database at `db_path`.
/// Logs on error and returns `false`, ensuring bus write failures never break callers.
pub fn record_event_safely(db_path: &Path, event: &FarmEvent) -> bool {
    match FarmBus::open(db_path) {
        Ok(bus) => match bus.record(event) {
            Ok(inserted) => inserted,
            Err(e) => {
                eprintln!("warning: bus failed to record event '{}': {e}", event.event_type);
                false
            }
        },
        Err(e) => {
            eprintln!("warning: bus failed to open at {}: {e}", db_path.display());
            false
        }
    }
}

/// Safely records a farm event into `<runs_dir>/quaqua.db`.
pub fn record_event_safely_in_runs(runs_dir: &Path, event: &FarmEvent) -> bool {
    record_event_safely(&runs_dir.join("quaqua.db"), event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    struct TempDb(PathBuf);
    impl Drop for TempDb {
        fn drop(&mut self) {
            if let Some(parent) = self.0.parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
    }

    fn temp_db(tag: &str) -> (PathBuf, TempDb) {
        static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("quaqua-bus-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("quaqua.db");
        (path.clone(), TempDb(path))
    }

    #[test]
    fn test_concurrent_inserts_give_monotonic_seq() {
        let (db_path, _cleaner) = temp_db("concurrent-seq");
        let bus = Arc::new(FarmBus::open(&db_path).expect("open bus"));

        let num_threads = 8;
        let events_per_thread = 20;
        let mut handles = Vec::new();

        for t in 0..num_threads {
            let bus = Arc::clone(&bus);
            handles.push(thread::spawn(move || {
                for i in 0..events_per_thread {
                    let ev = FarmEvent::new(
                        "spawned",
                        Some(format!("thread-{t}")),
                        Some(format!("worker-{t}-{i}")),
                        Some("cos"),
                        Some("quaqua"),
                    );
                    let inserted = bus.record(&ev).expect("record event");
                    assert!(inserted, "new plain event must be inserted");
                }
            }));
        }

        for h in handles {
            h.join().expect("thread join");
        }

        let events = bus.load_events(0).expect("load events");
        assert_eq!(
            events.len(),
            num_threads * events_per_thread,
            "all concurrent events must be recorded"
        );

        for (i, ev) in events.iter().enumerate() {
            let expected_seq = (i + 1) as u64;
            assert_eq!(
                ev.seq, expected_seq,
                "seq must be strictly consecutive and monotonic"
            );
        }
    }

    #[test]
    fn test_duplicate_incident_key_inserts_once() {
        let (db_path, _cleaner) = temp_db("dup-incident");
        let bus = FarmBus::open(&db_path).expect("open bus");

        let key = "stall:sude-task:1000";

        // 1. First insert with incident key must succeed
        let ev1 = FarmEvent::new("stalled", Some("watchdog"), Some("sude-task"), Some("cos"), Some("quaqua"))
            .with_incident_key(key)
            .with_payload_json(r#"{"attempt": 1}"#);
        let inserted1 = bus.record(&ev1).expect("record ev1");
        assert!(inserted1, "first incident_key insert must return true");

        // 2. Duplicate insert with identical incident key must be ignored and return false
        let ev2 = FarmEvent::new("stalled", Some("watchdog"), Some("sude-task"), Some("cos"), Some("quaqua"))
            .with_incident_key(key)
            .with_payload_json(r#"{"attempt": 2}"#);
        let inserted2 = bus.record(&ev2).expect("record ev2");
        assert!(!inserted2, "duplicate incident_key insert must return false");

        // 3. Different incident key must succeed
        let ev3 = FarmEvent::new("stalled", Some("watchdog"), Some("other-task"), Some("cos"), Some("quaqua"))
            .with_incident_key("stall:other-task:1000");
        let inserted3 = bus.record(&ev3).expect("record ev3");
        assert!(inserted3, "different incident_key must return true");

        // 4. Events with None incident_key must always succeed
        let ev4 = FarmEvent::new("spawned", Some("cos"), Some("task-a"), Some("cos"), Some("quaqua"));
        let ev5 = FarmEvent::new("spawned", Some("cos"), Some("task-b"), Some("cos"), Some("quaqua"));
        assert!(bus.record(&ev4).expect("record ev4"));
        assert!(bus.record(&ev5).expect("record ev5"));

        // Verify total count: 4 events (ev1, ev3, ev4, ev5); ev2 was ignored
        let events = bus.load_events(0).expect("load events");
        assert_eq!(events.len(), 4, "duplicate incident key must not produce a row");
    }

    #[test]
    fn test_quaqua_status_output_unchanged_with_new_tables_present() {
        use crate::engine::journal_sqlite::SqliteJournal;
        use crate::engine::journal::{Journal, EventRecord};
        use crate::status::render_status;
        use crate::herdr::Herdr;

        let (db_path, _cleaner) = temp_db("status-unchanged");
        let runs_dir = db_path.parent().unwrap();

        // Populate standard journal data (events table)
        {
            let mut journal = SqliteJournal::open(&db_path).expect("open journal");
            let ev1 = EventRecord::new(1, 1000, "workflow_started", "{}");
            let ev2 = EventRecord::new(2, 2000, "workflow_completed", r#"{"exit":0}"#);
            journal.append_event("wf-status-test", &ev1).expect("append ev1");
            journal.append_event("wf-status-test", &ev2).expect("append ev2");
        }

        let mock_herdr = Herdr::new();
        let status_before = render_status(&db_path, runs_dir, &mock_herdr).expect("render status before");

        // Now open FarmBus on the same database, creating farm_events and cursors tables,
        // and insert several farm events.
        {
            let bus = FarmBus::open(&db_path).expect("open bus");
            bus.record(&FarmEvent::new("spawned", Some("cos"), Some("w1"), Some("cos"), Some("proj"))).expect("rec 1");
            bus.record(&FarmEvent::new("report_written", Some("w1"), Some("w1"), Some("cos"), Some("proj"))).expect("rec 2");
            bus.set_cursor("cos", 42).expect("set cursor");
        }

        // Render status again: must be byte-identical!
        let status_after = render_status(&db_path, runs_dir, &mock_herdr).expect("render status after");

        assert_eq!(
            status_before, status_after,
            "quaqua status output must be byte-unchanged when farm_events/cursors tables and rows exist"
        );
    }
}
