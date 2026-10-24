use quaqua::bus::mail::{self, SendMailOpts};
use quaqua::bus::FarmBus;
use quaqua::herdr::Herdr;
use std::path::{Path, PathBuf};

fn hermetic_temp_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "quaqua-mail-test-{prefix}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create hermetic temp dir");
    dir
}

#[test]
fn test_retried_send_gives_one_row() {
    let runs = hermetic_temp_dir("retry-send");
    let herdr = Herdr::new();

    let opts = SendMailOpts {
        to: "cos".to_string(),
        kind: "accept".to_string(),
        body: "verdict: accepted".to_string(),
        from: Some("bus-lead".to_string()),
        project: Some("bus".to_string()),
        reply_to: None,
        client_id: None,
    };

    let res1 = mail::send_mail(&runs, &opts, &herdr).expect("first send");
    assert!(res1.id > 0);

    // Verify exactly 1 mail row and 1 farm_events row
    let bus = FarmBus::open(&runs.join("quaqua.db")).expect("open bus");
    let mail_count: i64 = bus
        .conn_for_test()
        .query_row("SELECT COUNT(*) FROM mail", [], |r| r.get(0))
        .expect("count mail");
    let event_count: i64 = bus
        .conn_for_test()
        .query_row(
            "SELECT COUNT(*) FROM farm_events WHERE type = 'mail_sent'",
            [],
            |r| r.get(0),
        )
        .expect("count events");
    assert_eq!(mail_count, 1);
    assert_eq!(event_count, 1);

    // Retried send with identical contents
    let res2 = mail::send_mail(&runs, &opts, &herdr).expect("retried send");
    assert_eq!(res1.id, res2.id);

    // Verify still exactly 1 mail row and 1 farm_events row
    let mail_count2: i64 = bus
        .conn_for_test()
        .query_row("SELECT COUNT(*) FROM mail", [], |r| r.get(0))
        .expect("count mail after retry");
    let event_count2: i64 = bus
        .conn_for_test()
        .query_row(
            "SELECT COUNT(*) FROM farm_events WHERE type = 'mail_sent'",
            [],
            |r| r.get(0),
        )
        .expect("count events after retry");
    assert_eq!(mail_count2, 1);
    assert_eq!(event_count2, 1);
}

#[test]
fn test_ack_by_non_recipient_refused() {
    let runs = hermetic_temp_dir("ack-refused");
    let herdr = Herdr::new();

    let opts = SendMailOpts {
        to: "cos".to_string(),
        kind: "accept".to_string(),
        body: "report for cos".to_string(),
        from: Some("bus-lead".to_string()),
        project: Some("bus".to_string()),
        reply_to: None,
        client_id: None,
    };
    let res = mail::send_mail(&runs, &opts, &herdr).expect("send mail");

    // Non-recipient 'gate-lead' tries to ack
    let err = mail::ack_mail(&runs, "gate-lead", &[res.id])
        .expect_err("non-recipient ack must be refused");
    assert!(
        err.contains("refused") || err.contains("recipient"),
        "unexpected error: {err}"
    );

    // Verify still unacked
    let bus = FarmBus::open(&runs.join("quaqua.db")).expect("open bus");
    let acked_ms: Option<i64> = bus
        .conn_for_test()
        .query_row(
            "SELECT acked_ms FROM mail WHERE id = ?1",
            [res.id as i64],
            |r| r.get(0),
        )
        .expect("query acked");
    assert!(acked_ms.is_none());

    // Recipient 'cos' acks
    let acked_ids = mail::ack_mail(&runs, "cos", &[res.id]).expect("recipient ack succeeds");
    assert_eq!(acked_ids, vec![res.id]);

    let acked_by: Option<String> = bus
        .conn_for_test()
        .query_row(
            "SELECT acked_by FROM mail WHERE id = ?1",
            [res.id as i64],
            |r| r.get(0),
        )
        .expect("query acked_by");
    assert_eq!(acked_by.as_deref(), Some("cos"));
}

#[test]
fn test_unacked_mail_redelivered_with_count() {
    let runs = hermetic_temp_dir("redeliver-count");
    let herdr = Herdr::new();

    let opts = SendMailOpts {
        to: "cos".to_string(),
        kind: "handoff".to_string(),
        body: "handoff payload".to_string(),
        from: Some("bus-lead".to_string()),
        project: Some("bus".to_string()),
        reply_to: None,
        client_id: None,
    };
    let res = mail::send_mail(&runs, &opts, &herdr).expect("send mail");

    // First read: deliveries = 1
    let out1 = mail::render_inbox(&runs, "cos").expect("first inbox read");
    assert!(out1.contains(&format!("#{}", res.id)));
    assert!(
        !out1.contains("redelivery #"),
        "first delivery must not say redelivery"
    );

    // Second read: deliveries = 2 -> (redelivery #2)
    let out2 = mail::render_inbox(&runs, "cos").expect("second inbox read");
    assert!(out2.contains(&format!("#{}", res.id)));
    assert!(out2.contains("(redelivery #2)"));

    // Third read: deliveries = 3 -> (redelivery #3)
    let out3 = mail::render_inbox(&runs, "cos").expect("third inbox read");
    assert!(out3.contains(&format!("#{}", res.id)));
    assert!(out3.contains("(redelivery #3)"));

    // Now ack as cos
    mail::ack_mail(&runs, "cos", &[res.id]).expect("ack mail");

    // Fourth read: no unacked mail
    let out4 = mail::render_inbox(&runs, "cos").expect("inbox after ack");
    assert!(!out4.contains(&format!("#{}", res.id)));
}

#[test]
fn test_import_live_inbox_files_idempotent() {
    let runs = hermetic_temp_dir("import-live");
    let inbox_temp = hermetic_temp_dir("inbox-sources");

    // Find and copy live inbox files from ~/quaqua/runs/*.inbox.md
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let live_runs = Path::new(&home).join("quaqua").join("runs");

    let mut copied_count = 0;
    if let Ok(entries) = std::fs::read_dir(&live_runs) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |s| s.ends_with("-lead.inbox.md"))
            {
                let dest = inbox_temp.join(path.file_name().unwrap());
                let _ = std::fs::copy(&path, &dest);
                copied_count += 1;
            }
        }
    }

    // Ensure we have 5 inbox files for testing by providing representative live-format inboxes if fewer
    let extra_inboxes = [
        (
            "arch-lead.inbox.md",
            "## 2026-10-19T14:00:00Z arch-lead handoff\nverdict: clean\n",
        ),
        (
            "quaqua-rs-lead.inbox.md",
            "## 2026-10-16T12:00:00+07:00 quaqua-rs-lead accept\nverdict: passed\n",
        ),
        (
            "bus-sample-lead.inbox.md",
            "## 2026-10-24T00:00:00+07:00 bus-sample-lead handoff\nP2 bus accepted\n",
        ),
        (
            "gate-sample-lead.inbox.md",
            "## 2026-10-24T10:49:42Z gate-sample-lead handoff\nP1 done\n",
        ),
        (
            "mono-sample-lead.inbox.md",
            "## 2026-10-20T14:51:41Z mono-sample-lead accept\nverdict: accepted\n",
        ),
    ];
    let mut extra_idx = 0;
    while copied_count < 5 && extra_idx < extra_inboxes.len() {
        let (name, content) = extra_inboxes[extra_idx];
        let dest = inbox_temp.join(name);
        if !dest.exists() {
            std::fs::write(&dest, content).expect("write extra inbox file");
            copied_count += 1;
        }
        extra_idx += 1;
    }
    assert!(
        copied_count >= 5,
        "must have at least 5 inbox files for test, got {copied_count}"
    );

    // First import run
    let mut total_imported = 0;
    for entry in std::fs::read_dir(&inbox_temp).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let res = mail::import_inbox_file(&runs, &path).expect("import inbox file");
            total_imported += res.imported;
        }
    }
    assert!(total_imported > 0, "must import at least some rows");

    // Second import run: should be completely idempotent (0 new rows)
    let mut second_imported = 0;
    for entry in std::fs::read_dir(&inbox_temp).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let res = mail::import_inbox_file(&runs, &path).expect("re-import inbox file");
            second_imported += res.imported;
        }
    }
    assert_eq!(
        second_imported, 0,
        "second import must add 0 rows (idempotent)"
    );
}

#[test]
fn test_unacked_mail_lazily_inserts_incident() {
    let runs = hermetic_temp_dir("unacked-incident");
    let bus = FarmBus::open(&runs.join("quaqua.db")).expect("open bus");

    // Insert a mail with timestamp 40 minutes in the past
    let now = quaqua::bus::now_ms();
    let past_ts = now.saturating_sub(40 * 60 * 1000);
    let msg_key = mail::sha256_hex(b"old-mail-key");

    bus.conn_for_test()
        .execute(
            r#"
        INSERT INTO mail (
            msg_key, ts_ms, from_name, from_role, to_name, kind, project, reply_to, body
        ) VALUES (?1, ?2, 'bus-lead', 'lead', 'cos', 'accept', 'bus', NULL, 'past deadline')
        "#,
            rusqlite::params![msg_key, past_ts as i64],
        )
        .expect("insert past mail");

    let mail_id: i64 = bus
        .conn_for_test()
        .query_row(
            "SELECT id FROM mail WHERE msg_key = ?1",
            rusqlite::params![msg_key],
            |r| r.get(0),
        )
        .expect("query mail id");

    // Trigger inbox read which lazily runs check_unacked_incidents
    let _ = mail::render_inbox(&runs, "cos").expect("render inbox");

    // Check that mail_unacked:<id> incident exists in farm_events
    let expected_key = format!("mail_unacked:{mail_id}");
    let incident_count: i64 = bus
        .conn_for_test()
        .query_row(
            "SELECT COUNT(*) FROM farm_events WHERE incident_key = ?1 AND type = 'mail_unacked'",
            rusqlite::params![expected_key],
            |r| r.get(0),
        )
        .expect("query incident");
    assert_eq!(
        incident_count, 1,
        "must lazily insert mail_unacked incident"
    );

    // Trigger inbox read again: incident_key is UNIQUE, so insert is ignored
    let _ = mail::render_inbox(&runs, "cos").expect("render inbox again");
    let incident_count2: i64 = bus
        .conn_for_test()
        .query_row(
            "SELECT COUNT(*) FROM farm_events WHERE incident_key = ?1 AND type = 'mail_unacked'",
            rusqlite::params![expected_key],
            |r| r.get(0),
        )
        .expect("query incident again");
    assert_eq!(incident_count2, 1, "incident must not be duplicated");
}

#[test]
fn test_missing_or_corrupt_db_falls_back_to_markdown() {
    let runs = hermetic_temp_dir("corrupt-db-fallback");
    let herdr = Herdr::new();

    // Make quaqua.db a directory so SQLite open fails
    let bad_db = runs.join("quaqua.db");
    std::fs::create_dir_all(&bad_db).expect("create dir where file expected");

    let opts = SendMailOpts {
        to: "cos".to_string(),
        kind: "blocked".to_string(),
        body: "critical failure report".to_string(),
        from: Some("bus-lead".to_string()),
        project: Some("bus".to_string()),
        reply_to: None,
        client_id: None,
    };

    // Send mail should succeed with fallback
    let res = mail::send_mail(&runs, &opts, &herdr).expect("send mail with corrupt db fallback");
    assert_eq!(res.id, 0);

    // Verify runs/bus-lead.inbox.md was appended
    let md_path = runs.join("bus-lead.inbox.md");
    assert!(md_path.exists(), "fallback inbox file must exist");
    let content = std::fs::read_to_string(&md_path).expect("read fallback md");
    assert!(content.contains("bus-lead blocked"));
    assert!(content.contains("critical failure report"));
}

#[test]
fn test_body_length_exceeded_error() {
    let runs = hermetic_temp_dir("body-len");
    let herdr = Herdr::new();

    let long_body = "x".repeat(4097);
    let opts = SendMailOpts {
        to: "cos".to_string(),
        kind: "accept".to_string(),
        body: long_body,
        from: Some("bus-lead".to_string()),
        project: Some("bus".to_string()),
        reply_to: None,
        client_id: None,
    };

    let err = mail::send_mail(&runs, &opts, &herdr).expect_err("body > 4096 must error");
    assert!(
        err.contains("4096"),
        "error must mention 4096 max body length: {err}"
    );
}

#[test]
fn test_cli_mail_send_inbox_ack_ls() {
    let runs = hermetic_temp_dir("cli-mail");
    let herdr = Herdr::new();

    std::env::set_var("QUAQUA_RUNS_DIR", runs.to_str().unwrap());

    // 1. CLI send
    let send_args = vec![
        "send".to_string(),
        "--to".to_string(),
        "cos".to_string(),
        "--kind".to_string(),
        "accept".to_string(),
        "--from".to_string(),
        "bus-lead".to_string(),
        "--body".to_string(),
        "cli send verification".to_string(),
    ];
    let rc = mail::handle_mail(&send_args, &herdr).expect("cli send");
    assert_eq!(rc, 0);

    // 2. CLI ls --md
    let ls_args = vec!["ls".to_string(), "--md".to_string()];
    let rc_ls = mail::handle_mail(&ls_args, &herdr).expect("cli ls");
    assert_eq!(rc_ls, 0);

    // 3. CLI inbox --for cos
    let inbox_args = vec!["inbox".to_string(), "--for".to_string(), "cos".to_string()];
    let rc_inbox = mail::handle_mail(&inbox_args, &herdr).expect("cli inbox");
    assert_eq!(rc_inbox, 0);

    // 4. CLI ack 1 --as cos
    let ack_args = vec![
        "ack".to_string(),
        "1".to_string(),
        "--as".to_string(),
        "cos".to_string(),
    ];
    let rc_ack = mail::handle_mail(&ack_args, &herdr).expect("cli ack");
    assert_eq!(rc_ack, 0);

    std::env::remove_var("QUAQUA_RUNS_DIR");
}
