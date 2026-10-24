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

static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct EnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    vars: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn new(keys: &[&'static str]) -> Self {
        let lock = TEST_ENV_LOCK.lock().unwrap();
        let vars = keys.iter().map(|&k| (k, std::env::var(k).ok())).collect();
        Self { _lock: lock, vars }
    }
    fn set(&self, key: &'static str, val: &str) {
        std::env::set_var(key, val);
    }
    fn remove(&self, key: &'static str) {
        std::env::remove_var(key);
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.vars {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }
}

#[test]
fn test_finding1_ack_inbox_identity_fail_closed() {
    let guard = EnvGuard::new(&["QUAQUA_TEST", "QUAQUA_OWNER", "HERDR_PANE_ID", "QUAQUA_RUNS_DIR"]);
    guard.remove("QUAQUA_TEST");
    guard.remove("QUAQUA_OWNER");
    guard.remove("HERDR_PANE_ID");

    let runs = hermetic_temp_dir("finding1-identity");
    guard.set("QUAQUA_RUNS_DIR", runs.to_str().unwrap());
    let herdr = Herdr::new();

    // 1. ack without identity must fail (not default to cos)
    let res = mail::handle_mail(&["ack".to_string(), "1".to_string()], &herdr);
    assert!(res.is_err(), "ack without caller identity must fail closed");

    // 2. ack with --as must be refused (no forgeable override)
    let res = mail::handle_mail(
        &["ack".to_string(), "1".to_string(), "--as".to_string(), "cos".to_string()],
        &herdr,
    );
    assert!(res.is_err(), "ack with --as override must be refused");

    // 3. inbox without --for must fail closed when caller identity is unknown (no cos default)
    let res = mail::handle_mail(&["inbox".to_string()], &herdr);
    assert!(res.is_err(), "inbox without caller identity must fail closed without cos default");
}

#[test]
fn test_finding2_sender_identity_fail_closed_no_from() {
    let guard = EnvGuard::new(&["QUAQUA_TEST", "QUAQUA_OWNER", "HERDR_PANE_ID", "QUAQUA_RUNS_DIR"]);
    guard.remove("QUAQUA_TEST");
    guard.remove("QUAQUA_OWNER");
    guard.remove("HERDR_PANE_ID");

    let runs = hermetic_temp_dir("finding2-sender");
    guard.set("QUAQUA_RUNS_DIR", runs.to_str().unwrap());
    let herdr = Herdr::new();

    // 1. send with --from must be refused (cannot forge sender)
    let res = mail::handle_mail(
        &[
            "send".to_string(),
            "--to".to_string(),
            "cos".to_string(),
            "--kind".to_string(),
            "accept".to_string(),
            "--from".to_string(),
            "mu-lead".to_string(),
            "--body".to_string(),
            "forged accept".to_string(),
        ],
        &herdr,
    );
    assert!(res.is_err(), "send with --from override must be refused");

    // 2. send without harness identity must fail closed
    let res = mail::handle_mail(
        &[
            "send".to_string(),
            "--to".to_string(),
            "cos".to_string(),
            "--kind".to_string(),
            "accept".to_string(),
            "--body".to_string(),
            "hello".to_string(),
        ],
        &herdr,
    );
    assert!(res.is_err(), "send without harness identity must fail closed");
}

#[test]
fn test_finding3_path_traversal_refused() {
    let runs = hermetic_temp_dir("finding3-traversal");
    let outside_path = std::env::temp_dir().join("evil-traversal-probe.inbox.md");
    if outside_path.exists() {
        let _ = std::fs::remove_file(&outside_path);
    }

    let res = mail::append_to_markdown_inbox(
        &runs,
        "../../evil-traversal-probe",
        "2026-10-24T00:00:00Z",
        "accept",
        "payload",
    );
    assert!(res.is_err(), "path traversal in sender name must be refused");
    assert!(!outside_path.exists(), "must not create file outside runs_dir");
}

#[test]
fn test_finding4_dual_write_and_import_deduplicated() {
    let guard = EnvGuard::new(&["QUAQUA_TEST", "QUAQUA_OWNER", "HERDR_PANE_ID", "QUAQUA_RUNS_DIR"]);
    guard.set("QUAQUA_TEST", "1");
    guard.set("QUAQUA_OWNER", "mu-lead");

    let runs = hermetic_temp_dir("finding4-dedup");
    let herdr = Herdr::new();

    let opts = SendMailOpts {
        to: "cos".to_string(),
        kind: "accept".to_string(),
        body: "dual write dedup check".to_string(),
        from: Some("mu-lead".to_string()),
        project: Some("mu".to_string()),
        reply_to: None,
        client_id: None,
    };

    let res1 = mail::send_mail(&runs, &opts, &herdr).expect("first send");
    assert!(!res1.already_sent);

    // Retried send across boundary
    let res2 = mail::send_mail(&runs, &opts, &herdr).expect("retried send");
    assert!(res2.already_sent);

    // Verify markdown entry in mu-lead.inbox.md: must have exactly 1 entry
    let md_path = runs.join("mu-lead.inbox.md");
    assert!(md_path.exists(), "mu-lead.inbox.md must exist");
    let md_content = std::fs::read_to_string(&md_path).expect("read md");
    let header_count = md_content.lines().filter(|l| l.starts_with("## ")).count();
    assert_eq!(header_count, 1, "retried send must write exactly 1 md entry");

    // Import the markdown inbox file
    let imp = mail::import_inbox_file(&runs, &md_path).expect("import inbox");
    assert_eq!(imp.total, 1);
    assert_eq!(imp.imported, 0, "already-sent dual-write mail must be skipped on import");

    // Verify SQLite mail table has exactly 1 row
    let bus = FarmBus::open(&runs.join("quaqua.db")).expect("open bus");
    let count: i64 = bus
        .conn_for_test()
        .query_row("SELECT COUNT(*) FROM mail", [], |r| r.get(0))
        .expect("count mail");
    assert_eq!(count, 1, "lead send + import must yield exactly one row");
}

#[test]
fn test_finding5_md_fallback_recipient_and_locked_db() {
    let guard = EnvGuard::new(&["QUAQUA_TEST", "QUAQUA_OWNER", "HERDR_PANE_ID", "QUAQUA_RUNS_DIR"]);
    guard.set("QUAQUA_TEST", "1");
    guard.set("QUAQUA_OWNER", "tieude-1");

    let runs = hermetic_temp_dir("finding5-fallback");
    let herdr = Herdr::new();

    // 1. Locked db must return Err containing database path (exit 1), not silent fallback
    let db_path = runs.join("quaqua.db");
    {
        let bus = FarmBus::open(&db_path).expect("create db");
        // Hold exclusive lock
        let conn = bus.conn_for_test();
        conn.execute_batch("BEGIN EXCLUSIVE;").expect("begin exclusive lock");

        let opts = SendMailOpts {
            to: "mu-lead".to_string(),
            kind: "accept".to_string(),
            body: "locked db test".to_string(),
            from: Some("tieude-1".to_string()),
            project: Some("tieude".to_string()),
            reply_to: None,
            client_id: None,
        };

        // send_mail must fail with an error naming the db path
        let err = mail::send_mail(&runs, &opts, &herdr).expect_err("locked db must return error");
        assert!(
            err.contains("quaqua.db") || err.contains("database locked"),
            "error must include path and mention lock: {err}"
        );
    }

    // 2. Fallback on corrupt DB: must keep recipient mu-lead
    let corrupt_runs = hermetic_temp_dir("finding5-corrupt");
    std::fs::write(corrupt_runs.join("quaqua.db"), b"not a valid sqlite file").unwrap();

    let opts2 = SendMailOpts {
        to: "mu-lead".to_string(),
        kind: "handoff".to_string(),
        body: "peer to lead handoff".to_string(),
        from: Some("tieude-1".to_string()),
        project: Some("tieude".to_string()),
        reply_to: None,
        client_id: None,
    };
    let res = mail::send_mail(&corrupt_runs, &opts2, &herdr).expect("send fallback");
    assert_eq!(res.id, 0);

    // Must be in mu-lead.inbox.md, NOT tieude-1.inbox.md
    assert!(!corrupt_runs.join("tieude-1.inbox.md").exists(), "must not write to sender inbox");
    let target_md = corrupt_runs.join("mu-lead.inbox.md");
    assert!(target_md.exists(), "must write to recipient inbox mu-lead.inbox.md");

    // Repair the db to test import
    std::fs::remove_file(corrupt_runs.join("quaqua.db")).unwrap();
    let imp = mail::import_inbox_file(&corrupt_runs, &target_md).expect("import fallback");
    assert_eq!(imp.imported, 1);

    let bus = FarmBus::open(&corrupt_runs.join("quaqua.db")).expect("open repaired bus");
    let to_name: String = bus
        .conn_for_test()
        .query_row("SELECT to_name FROM mail LIMIT 1", [], |r| r.get(0))
        .expect("query to_name");
    assert_eq!(to_name, "mu-lead", "import must preserve recipient mu-lead, not default to cos");
}

#[test]
fn test_finding6_import_multibyte_char_boundary_truncation() {
    let runs = hermetic_temp_dir("finding6-utf8");
    let inbox_file = runs.join("test-lead.inbox.md");

    // 4095 ASCII bytes + 2-byte Vietnamese character 'ế' (starts at 4095, ends at 4097)
    let mut body = "a".repeat(4095);
    body.push_str("ế is multibyte UTF-8");

    let content = format!("## 2026-10-24T00:00:00Z test-lead handoff\n{body}\n");
    std::fs::write(&inbox_file, content).expect("write inbox file");

    // Must not panic on char boundary
    let res = mail::import_inbox_file(&runs, &inbox_file).expect("import must not panic");
    assert_eq!(res.imported, 1);

    let bus = FarmBus::open(&runs.join("quaqua.db")).expect("open bus");
    let imported_body: String = bus
        .conn_for_test()
        .query_row("SELECT body FROM mail LIMIT 1", [], |r| r.get(0))
        .expect("query body");
    assert!(imported_body.chars().count() <= 4096);
}

#[test]
fn test_finding7_checked_in_fixtures_idempotent() {
    let runs = hermetic_temp_dir("finding7-fixtures");
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/inbox");
    assert!(fixtures_dir.exists(), "fixtures dir must exist");

    let entries: Vec<PathBuf> = std::fs::read_dir(&fixtures_dir)
        .expect("read fixtures dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    assert!(entries.len() >= 5, "must have at least 5 checked-in fixture files, got {}", entries.len());

    // First import run
    let mut total_imported = 0;
    for path in &entries {
        let res = mail::import_inbox_file(&runs, path).expect("import fixture");
        total_imported += res.imported;
    }
    assert!(total_imported >= 5, "must import all fixture rows");

    // Verify bus-lead fixture with +07:00 offset and multibyte body was imported
    let bus = FarmBus::open(&runs.join("quaqua.db")).expect("open bus");
    let bus_lead_body: String = bus
        .conn_for_test()
        .query_row(
            "SELECT body FROM mail WHERE from_name = 'bus-lead' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("query bus-lead body");
    assert!(bus_lead_body.contains("Bàn giao P2 bus foundation"));

    // Second import run: strictly idempotent (0 imported)
    let mut second_imported = 0;
    for path in &entries {
        let res = mail::import_inbox_file(&runs, path).expect("re-import fixture");
        second_imported += res.imported;
    }
    assert_eq!(second_imported, 0, "second import must be idempotent (0 new rows)");
}

#[test]
fn test_finding8_mail_unacked_check_on_every_verb() {
    let runs = hermetic_temp_dir("finding8-verbs");
    let bus = FarmBus::open(&runs.join("quaqua.db")).expect("open bus");

    // Insert an unacked mail from 35 minutes ago (ack timeout is 30m)
    let old_ts = (quaqua::bus::now_ms()).saturating_sub(35 * 60 * 1000);
    bus.conn_for_test()
        .execute(
            r#"
            INSERT INTO mail (
                msg_key, ts_ms, from_name, from_role, to_name, kind, project, reply_to, body
            ) VALUES ('test-key-f8', ?1, 'mu-lead', 'lead', 'cos', 'escalate', 'mu', NULL, 'urgent')
            "#,
            rusqlite::params![old_ts as i64],
        )
        .expect("insert old unacked mail");

    // Call mail::list_mail
    let _ = mail::list_mail(&runs, true, None, None, None).expect("list mail");

    // Verify mail_unacked incident was inserted into farm_events
    let count: i64 = bus
        .conn_for_test()
        .query_row(
            "SELECT COUNT(*) FROM farm_events WHERE type = 'mail_unacked'",
            [],
            |r| r.get(0),
        )
        .expect("count unacked incidents");
    assert_eq!(count, 1, "list_mail must trigger mail_unacked incident check");
}
