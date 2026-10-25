//! Tests for `quaqua events hook`, incident notifications, and herdr plugin manifest.

use quaqua::herdr::Herdr;
use std::fs;
use std::path::{Path, PathBuf};

fn hermetic_temp_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "quaqua-hook-test-{prefix}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create hermetic temp dir");
    dir
}

#[test]
fn test_herdr_notification_exactly_one_per_incident_key() {
    let runs_dir = hermetic_temp_dir("notif-dedup");
    let notif_log = runs_dir.join("notifications.log");

    // Create a stub herdr binary that logs `notification show` calls
    let bin_dir = runs_dir.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    let herdr_bin = bin_dir.join("herdr");

    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "notification" ] && [ "$2" = "show" ]; then
    echo "$*" >> "{}"
    exit 0
fi
exit 0
"#,
        notif_log.display()
    );
    fs::write(&herdr_bin, script).expect("write stub herdr");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&herdr_bin).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&herdr_bin, perms).expect("chmod");
    }

    std::env::set_var("HERDR_BIN_PATH", &herdr_bin);

    let event_payload = serde_json::json!({
        "type": "pane_agent_status_changed",
        "pane_id": "pane-abc",
        "agent": "worker-1",
        "agent_status": "blocked",
        "status_since": "12345"
    })
    .to_string();

    let herdr_ops = Herdr::new();

    // Call hook first time
    let res1 = quaqua::bus::events::handle_event_hook(
        &runs_dir,
        Some("pane.agent_status_changed"),
        Some(&event_payload),
        &herdr_ops,
    );
    assert_eq!(res1, Ok(true), "first hook call must record incident and notify");

    // Verify 1 notification logged
    let log1 = fs::read_to_string(&notif_log).unwrap_or_default();
    let count1 = log1.lines().filter(|l| l.contains("notification show")).count();
    assert_eq!(count1, 1, "exactly 1 notification on first call");

    // Call hook second time with the exact same payload / incident_key
    let res2 = quaqua::bus::events::handle_event_hook(
        &runs_dir,
        Some("pane.agent_status_changed"),
        Some(&event_payload),
        &herdr_ops,
    );
    assert_eq!(res2, Ok(false), "second hook call must recognize duplicate and return false");

    // Verify notification count is STILL exactly 1
    let log2 = fs::read_to_string(&notif_log).unwrap_or_default();
    let count2 = log2.lines().filter(|l| l.contains("notification show")).count();
    assert_eq!(count2, 1, "must NOT send duplicate notification on repeated hook call");

    // Call hook third time
    let res3 = quaqua::bus::events::handle_event_hook(
        &runs_dir,
        Some("pane.agent_status_changed"),
        Some(&event_payload),
        &herdr_ops,
    );
    assert_eq!(res3, Ok(false));
    let log3 = fs::read_to_string(&notif_log).unwrap_or_default();
    let count3 = log3.lines().filter(|l| l.contains("notification show")).count();
    assert_eq!(count3, 1, "must remain exactly 1 notification across 3 calls");

    // A different incident key (different status_since or pane) should trigger a new notification
    let event_payload_2 = serde_json::json!({
        "type": "pane_agent_status_changed",
        "pane_id": "pane-def",
        "agent": "worker-2",
        "agent_status": "blocked",
        "status_since": "99999"
    })
    .to_string();

    let res4 = quaqua::bus::events::handle_event_hook(
        &runs_dir,
        Some("pane.agent_status_changed"),
        Some(&event_payload_2),
        &herdr_ops,
    );
    assert_eq!(res4, Ok(true), "different incident key must notify");
    let log4 = fs::read_to_string(&notif_log).unwrap_or_default();
    let count4 = log4.lines().filter(|l| l.contains("notification show")).count();
    assert_eq!(count4, 2, "must have exactly 2 notifications total (1 per incident key)");
}

#[test]
fn test_plugin_manifest_contains_events_hook() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .join("plugins/herdr-quaqua/herdr-plugin.toml");

    let text = fs::read_to_string(&manifest_path).expect("read herdr-plugin.toml");

    assert!(
        text.contains("command = [\"quaqua\", \"events\", \"hook\"]"),
        "herdr-plugin.toml must declare quaqua events hook command"
    );
    assert!(
        text.contains("on = \"pane.agent_status_changed\""),
        "herdr-plugin.toml must subscribe to pane.agent_status_changed"
    );
    assert!(
        text.contains("on = \"pane.exited\""),
        "herdr-plugin.toml must subscribe to pane.exited"
    );
}
