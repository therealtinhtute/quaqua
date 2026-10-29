//! Review repros for farm scoping (a1eee9e..0c02a83). Each test asserts the CORRECT behaviour;
//! a failure demonstrates the finding.
use quaqua::bus::FarmBus;
use quaqua::herdr::Herdr;
use std::fs;
use std::path::PathBuf;

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn tmp(prefix: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir()
        .join(format!("farmrev-{prefix}-{}-{n}", std::process::id()))
        .join("runs");
    fs::create_dir_all(&d).unwrap();
    d
}

fn stub(runs: &PathBuf, pane_get_body: &str) -> Herdr {
    let bin = runs.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let p = bin.join("herdr");
    fs::write(&p, format!("#!/bin/sh\nif [ \"$1\" = pane ] && [ \"$2\" = get ]; then\n{pane_get_body}\nfi\nexit 0\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("HERDR_BIN_PATH", &p);
    Herdr::new()
}

fn stub_herdr(runs: &PathBuf, body: &str) -> Herdr {
    let bin = runs.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let p = bin.join("herdr");
    fs::write(&p, format!("#!/bin/sh\n{body}\nexit 0\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("HERDR_BIN_PATH", &p);
    Herdr::new()
}

fn count(runs: &PathBuf, typ: &str) -> i64 {
    let bus = FarmBus::open(&runs.join("quaqua.db")).unwrap();
    let c = bus.conn_for_test();
    c.query_row(
        "SELECT count(*) FROM farm_events WHERE type = ?1",
        [typ],
        |r| r.get(0),
    )
    .unwrap()
}

// F1: pane.exited fires after the pane is gone -> pane get = pane_not_found -> Foreign -> dropped.
#[test]
fn r1_local_pane_exited_is_recorded() {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let runs = tmp("exited");
    fs::write(runs.join("tieude-x.owner"), "cos\n").unwrap();
    let h = stub(&runs, "echo '{\"error\":{\"code\":\"pane_not_found\",\"message\":\"gone\"},\"id\":\"cli:pane:get\"}'; exit 1");
    let ev =
        serde_json::json!({"type":"pane_exited","pane_id":"w1:pX","agent":"tieude-x"}).to_string();
    let _ = quaqua::bus::events::handle_event_hook(&runs, Some("pane.exited"), Some(&ev), &h);
    assert_eq!(
        count(&runs, "agent_exited"),
        1,
        "local worker pane.exited dropped as Foreign"
    );
}

// F2: any herdr error JSON other than pane_not_found has no result.pane -> Foreign, not fail-open.
#[test]
fn r2_other_herdr_error_fails_open() {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let runs = tmp("othererr");
    let h = stub(&runs, "echo '{\"error\":{\"code\":\"server_unavailable\",\"message\":\"x\"},\"id\":\"cli:pane:get\"}'; exit 1");
    let ev = serde_json::json!({"type":"pane_agent_status_changed","pane_id":"w1:pY","agent_status":"blocked","status_since":"1"}).to_string();
    let _ = quaqua::bus::events::handle_event_hook(
        &runs,
        Some("pane.agent_status_changed"),
        Some(&ev),
        &h,
    );
    assert_eq!(
        count(&runs, "blocked"),
        1,
        "herdr error other than pane_not_found must fail open"
    );
}

// F3: worker pane whose cwd is a subdir of its worktree -> basename != worker name -> Foreign.
#[test]
fn r3_worker_cwd_subdir_is_local() {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let runs = tmp("subdir");
    fs::write(runs.join("tieude-y.owner"), "cos\n").unwrap();
    let h = stub(&runs, "echo '{\"id\":\"cli:pane:get\",\"result\":{\"pane\":{\"cwd\":\"/nonexistent/.wt/tieude-y/crates/quaqua\",\"tokens\":{}}}}'; exit 0");
    let ev = serde_json::json!({"type":"pane_agent_status_changed","pane_id":"w1:pZ","agent_status":"blocked","status_since":"2"}).to_string();
    let _ = quaqua::bus::events::handle_event_hook(
        &runs,
        Some("pane.agent_status_changed"),
        Some(&ev),
        &h,
    );
    assert_eq!(
        count(&runs, "blocked"),
        1,
        "local worker in a worktree subdir classified Foreign"
    );
}

// F4: legacy herdr incidents (owner NULL, written by the pre-change hook) stay farm NULL and vanish
// from the CoS read, which is now strict `farm = ?`.
#[test]
fn r4_legacy_null_owner_incident_visible_to_cos() {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let runs = tmp("legacy");
    let db = runs.join("quaqua.db");
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch("CREATE TABLE farm_events (seq INTEGER PRIMARY KEY AUTOINCREMENT, ts_ms INTEGER NOT NULL, type TEXT NOT NULL, actor TEXT, subject TEXT, owner TEXT, project TEXT, incident_key TEXT UNIQUE, payload_json TEXT NOT NULL DEFAULT '{}');
            INSERT INTO farm_events (ts_ms,type,actor,subject,owner,project,incident_key) VALUES (1,'blocked','herdr','w1:p1X',NULL,NULL,'blocked:w1:p1X:1');").unwrap();
    }
    let bus = FarmBus::open(&db).unwrap();
    let n: i64 = bus
        .conn_for_test()
        .query_row("SELECT count(*) FROM farm_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1, "row preserved");
    let inc = bus.load_incidents(0, "cos").unwrap();
    assert_eq!(
        inc.len(),
        1,
        "legacy local herdr incident hidden from cos after migration"
    );
}

// Idempotence: opening twice keeps rows and farm values.
#[test]
fn r5_migration_idempotent() {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let runs = tmp("idem");
    let db = runs.join("quaqua.db");
    for _ in 0..3 {
        let _ = FarmBus::open(&db).unwrap();
    }
    let bus = FarmBus::open(&db).unwrap();
    let cols: i64 = bus
        .conn_for_test()
        .query_row(
            "SELECT count(*) FROM pragma_table_info('farm_events') WHERE name='farm'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cols, 1);
}

// F5: treat "duplicate column name" as success.
#[test]
fn r6_migration_duplicate_column_name_treated_as_success() {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let runs = tmp("dupcol");
    let db = runs.join("quaqua.db");
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE farm_events (seq INTEGER PRIMARY KEY AUTOINCREMENT, ts_ms INTEGER NOT NULL, type TEXT NOT NULL, actor TEXT, subject TEXT, owner TEXT, project TEXT, incident_key TEXT UNIQUE, payload_json TEXT NOT NULL DEFAULT '{}');
             CREATE TABLE mail (id INTEGER PRIMARY KEY AUTOINCREMENT, msg_key TEXT NOT NULL UNIQUE, ts_ms INTEGER NOT NULL, from_name TEXT NOT NULL, from_role TEXT NOT NULL, to_name TEXT NOT NULL, kind TEXT NOT NULL, project TEXT, reply_to INTEGER, body TEXT NOT NULL, deliveries INTEGER NOT NULL DEFAULT 0, last_delivered_ms INTEGER, acked_ms INTEGER, acked_by TEXT);"
        ).unwrap();
        c.execute("ALTER TABLE farm_events ADD COLUMN farm TEXT", [])
            .unwrap();
    }
    let bus = FarmBus::open(&db);
    assert!(
        bus.is_ok(),
        "FarmBus::open must succeed even if column was already added"
    );
}

// F7: a live agent with .pipeline but no .owner must not count against owner capacity.
#[test]
fn r7_cap_live_agent_with_pipeline_but_no_owner() {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let runs = tmp("pipenowner");
    fs::write(runs.join("tieude-pipe.pipeline"), "build\n").unwrap();
    let h = stub_herdr(
        &runs,
        r#"
if [ "$1" = agent ] && [ "$2" = list ]; then
    echo '{"result":{"agents":[{"name":"tieude-pipe","agent_status":"working","status":"running"}]}}'
    exit 0
fi
"#,
    );
    let cfg = quaqua::config::Config::default();
    let count = quaqua::cap::count_owner_slots(&runs, "cos", None, &h, &cfg);
    assert_eq!(
        count, 0,
        "live agent with .pipeline but no .owner must not count against cap"
    );
}
