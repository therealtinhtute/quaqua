use quaqua::cap::{count_owner_slots, is_task_worker_name};
use quaqua::herdr::Herdr;
use quaqua::worker::prompt::{brief_budget_sec, brief_headers_ok, resolve_budget_sec};
use quaqua::worker::spawn::parse_spawn_args;
use std::process::Command;

#[test]
fn test_task_worker_name_validation() {
    assert!(is_task_worker_name("tieude-task1"));
    assert!(is_task_worker_name("sude-hardimpl"));
    assert!(is_task_worker_name("quansu-review2"));
    assert!(is_task_worker_name("ni-quickfix"));

    // Leads do not count as task workers
    assert!(!is_task_worker_name("coucou-lead"));
    assert!(!is_task_worker_name("sude-lead"));
    assert!(!is_task_worker_name("quaqua-rs-lead"));

    // Unknown kinds
    assert!(!is_task_worker_name("random-worker"));
    assert!(!is_task_worker_name("unknown-task"));
}

#[test]
fn test_brief_headers() {
    let valid = "Goal: accomplish task\nHard constraints: none\nCurrent design (open to challenge): plan";
    assert!(brief_headers_ok(valid));

    let valid_alt = "Goal: accomplish task\nHard constraints: none\nCurrent design: direct";
    assert!(brief_headers_ok(valid_alt));

    let invalid_missing_goal = "Hard constraints: none\nCurrent design: direct";
    assert!(!brief_headers_ok(invalid_missing_goal));

    let invalid_missing_design = "Goal: accomplish task\nHard constraints: none";
    assert!(!brief_headers_ok(invalid_missing_design));
}

#[test]
fn test_brief_budget_parsing() {
    let brief = "Goal: x\nHard constraints: none\nCurrent design: direct\nTime budget: 25 min";
    assert_eq!(brief_budget_sec(brief), Some(1500));

    let brief2 = "Goal: x\nHard constraints: none\nCurrent design: direct\nTime budget: 10m";
    assert_eq!(brief_budget_sec(brief2), Some(600));

    let brief_none = "Goal: x\nHard constraints: none\nCurrent design: direct";
    assert_eq!(brief_budget_sec(brief_none), None);
    assert_eq!(resolve_budget_sec(brief_none), (2700, "default"));
}

#[test]
fn test_spawn_args_parsing() {
    let args = vec![
        "--base".to_string(),
        "main".to_string(),
        "--review".to_string(),
        "quansu-rev1".to_string(),
        "quansu".to_string(),
        "/repo".to_string(),
    ];
    let opts = parse_spawn_args(&args).expect("valid args");
    assert_eq!(opts.base.as_deref(), Some("main"));
    assert!(opts.review);
    assert_eq!(opts.name, "quansu-rev1");
    assert_eq!(opts.kind, "quansu");

    // Trailing base without value
    let bad = vec![
        "quansu-rev1".to_string(),
        "quansu".to_string(),
        "/repo".to_string(),
        "--base".to_string(),
    ];
    assert!(parse_spawn_args(&bad).is_err());
}

#[test]
fn test_cli_cap_smoke() {
    let bin = env!("CARGO_BIN_EXE_quaqua");
    let out = Command::new(bin)
        .arg("cap")
        .output()
        .expect("runs quaqua cap");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.trim().parse::<i64>().is_ok());
}
