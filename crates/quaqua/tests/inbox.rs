//! Tests for `quaqua inbox` consumer digest and per-reader cursor idempotence.

use quaqua::bus::mail::{self, SendMailOpts};
use quaqua::bus::FarmBus;
use quaqua::herdr::Herdr;
use std::path::PathBuf;

fn hermetic_temp_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "quaqua-inbox-test-{prefix}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create hermetic temp dir");
    dir
}

#[test]
fn test_inbox_cursor_read_idempotent() {
    let runs_dir = hermetic_temp_dir("cursor-idempotent");
    let db_path = runs_dir.join("quaqua.db");

    // Initialize FarmBus with schema
    let bus = FarmBus::open(&db_path).expect("open bus");

    // Record an incident event
    let ev1 = quaqua::bus::FarmEvent::new(
        "blocked",
        Some("herdr"),
        Some("worker-1"),
        Some("cos"),
        Some("myproj"),
    )
    .with_incident_key("blocked:p1:1000");

    let inserted = bus.record(&ev1).expect("record event");
    assert!(inserted, "ev1 must be newly inserted");

    // First inbox read for 'cos'
    let digest1 = bus.inbox_digest(&runs_dir, "cos").expect("first digest");
    assert_eq!(digest1.incidents.len(), 1, "first digest must see 1 incident");
    assert_eq!(digest1.incidents[0].incident_key.as_deref(), Some("blocked:p1:1000"));
    let cursor_after_first = bus.get_cursor("cos").expect("get cursor").expect("cursor set");
    assert_eq!(cursor_after_first, digest1.incidents[0].seq);

    // Repeated inbox read for 'cos' with no new events
    let digest2 = bus.inbox_digest(&runs_dir, "cos").expect("second digest");
    assert_eq!(digest2.incidents.len(), 0, "repeated read with no new events must yield 0 new incidents");
    let cursor_after_second = bus.get_cursor("cos").expect("get cursor").expect("cursor set");
    assert_eq!(
        cursor_after_second, cursor_after_first,
        "cursor must remain at same sequence across repeated reads"
    );
    // Record an incident owned by mu-lead
    let ev2 = quaqua::bus::FarmEvent::new(
        "blocked",
        Some("herdr"),
        Some("worker-2"),
        Some("mu-lead"),
        Some("myproj"),
    )
    .with_incident_key("blocked:p2:2000");
    bus.record(&ev2).expect("record ev2");

    // Second consumer 'mu-lead' has isolated cursor
    let digest_lead = bus.inbox_digest(&runs_dir, "mu-lead").expect("lead digest");
    assert_eq!(digest_lead.incidents.len(), 1, "lead must see its 1 incident");
    let lead_cursor = bus.get_cursor("mu-lead").expect("get lead cursor");
    assert_eq!(lead_cursor, Some(ev2_seq(&bus)));
}

fn ev2_seq(bus: &FarmBus) -> u64 {
    bus.load_events(0).unwrap().iter().find(|e| e.incident_key.as_deref() == Some("blocked:p2:2000")).unwrap().seq
}

#[test]
fn test_inbox_digest_with_mail_and_incidents() {
    let runs_dir = hermetic_temp_dir("mail-and-incidents");
    let db_path = runs_dir.join("quaqua.db");
    let bus = FarmBus::open(&db_path).expect("open bus");
    let herdr = Herdr::new();

    std::env::set_var("QUAQUA_TEST", "1");
    std::env::set_var("QUAQUA_OWNER", "mu-lead");

    std::env::remove_var("HERDR_PANE_ID");
    let opts = SendMailOpts {
        to: "cos".to_string(),
        kind: "accept".to_string(),
        body: "verification passed on sha 12345".to_string(),
        project: Some("mu".to_string()),
        reply_to: None,
        client_id: None,
    };

    let send_res = mail::send_mail(&runs_dir, &opts, &herdr).expect("send mail");
    assert!(send_res.id > 0);

    // Record an incident
    let ev = quaqua::bus::FarmEvent::new(
        "stalled",
        Some("watchdog"),
        Some("sude-worker"),
        Some("cos"),
        Some("mu"),
    )
    .with_incident_key("stall:sude-worker:5000");
    bus.record(&ev).expect("record incident");

    // Read inbox digest
    let digest = bus.inbox_digest(&runs_dir, "cos").expect("digest");
    assert_eq!(digest.unacked_mail.len(), 1, "must have 1 unacked mail");
    assert_eq!(digest.unacked_mail[0].id, send_res.id);
    assert_eq!(digest.incidents.len(), 1, "must have 1 incident");

    let rendered = digest.render("cos");
    assert!(rendered.contains(&format!("#{} mu-lead accept", send_res.id)), "rendered mail header");
    assert!(rendered.contains("stall:sude-worker:5000"), "rendered incident");
}
