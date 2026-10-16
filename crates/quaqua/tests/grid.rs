use quaqua::grid::{
    apply_ops, parse_snapshot_value, plan, run_reconcile, FakeHerdrDriver, ReconcileOptions, Target,
};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/herdr-grid")
}

fn plan_jq_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/herdr-grid/plan.jq")
}

#[test]
fn test_0_workers_yields_empty_ops() {
    let fix = fixtures_dir().join("0-workers.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let snap = parse_snapshot_value(&val).unwrap();
    let ops = plan(&snap).unwrap();
    assert_eq!(ops, vec![]);
}

#[test]
fn test_1_worker_split_beside_anchor() {
    let fix = fixtures_dir().join("1-worker.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let snap = parse_snapshot_value(&val).unwrap();
    let ops = plan(&snap).unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].pane, "w1:p5");
    assert!(!ops[0].deferred);
    match &ops[0].target {
        Target::Split {
            tab,
            anchor,
            direction,
            ratio,
        } => {
            assert_eq!(tab, "w1:t1");
            assert_eq!(anchor, "w1:p1");
            assert_eq!(direction, "right");
            assert_eq!(*ratio, 0.4);
        }
        _ => panic!("expected split target"),
    }
}

#[test]
fn test_2_workers_lands_in_grid_tab_never_orchestrator_tab() {
    let fix = fixtures_dir().join("2-workers.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let snap = parse_snapshot_value(&val).unwrap();
    let ops = plan(&snap).unwrap();
    assert_eq!(ops.len(), 2);
    // op 0: new_tab quaqua-workers (GRID_TAB, not orchestrator tab w1:t1)
    match &ops[0].target {
        Target::NewTab { label } => {
            assert_eq!(label, "quaqua-workers");
        }
        _ => panic!("op 0 expected new_tab, got {:?}", ops[0].target),
    }
    // op 1: split on @workers
    match &ops[1].target {
        Target::Split {
            tab,
            anchor,
            direction,
            ratio,
        } => {
            assert_eq!(tab, "@workers");
            assert_eq!(anchor, "w1:p5");
            assert_eq!(direction, "right");
            assert_eq!(*ratio, 0.5);
        }
        _ => panic!("op 1 expected split, got {:?}", ops[1].target),
    }
}

#[test]
fn test_5_workers_rejected_with_error() {
    let fix = fixtures_dir().join("5-workers.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let snap = parse_snapshot_value(&val).unwrap();
    let err = plan(&snap).unwrap_err();
    assert!(
        err.contains("too many workers in group"),
        "unexpected error message: {err}"
    );
}

#[test]
fn test_differential_all_fixtures_against_plan_jq() {
    let jq_bin = Command::new("which")
        .arg("jq")
        .output()
        .ok()
        .filter(|o| o.status.success());

    if jq_bin.is_none() {
        eprintln!("SKIP: jq not found on PATH; skipping differential comparison against plan.jq");
        return;
    }

    let plan_jq = plan_jq_path();
    assert!(plan_jq.exists(), "missing plan.jq at {:?}", plan_jq);

    let entries = fs::read_dir(fixtures_dir()).unwrap();
    let mut count = 0;

    for entry in entries {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let content = fs::read_to_string(&path).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        let snap_res = parse_snapshot_value(&val);
        if snap_res.is_err() {
            continue;
        }
        let snap = snap_res.unwrap();
        let rust_res = plan(&snap);

        let jq_out = Command::new("jq")
            .arg("-c")
            .arg("-f")
            .arg(&plan_jq)
            .arg(&path)
            .output()
            .unwrap();

        let filename = path.file_name().unwrap().to_string_lossy();

        if !jq_out.status.success() {
            // Both must fail
            assert!(
                rust_res.is_err(),
                "fixture {filename}: plan.jq failed but Rust plan succeeded"
            );
            let jq_err = String::from_utf8_lossy(&jq_out.stderr);
            let rust_err = rust_res.unwrap_err();
            assert!(
                rust_err.contains("too many workers in group")
                    && jq_err.contains("too many workers in group"),
                "fixture {filename}: error mismatch: rust='{rust_err}', jq='{jq_err}'"
            );
        } else {
            assert!(
                rust_res.is_ok(),
                "fixture {filename}: plan.jq succeeded but Rust plan failed: {:?}",
                rust_res.err()
            );
            let rust_ops = rust_res.unwrap();
            let rust_json = serde_json::to_value(&rust_ops).unwrap();
            let jq_json: serde_json::Value =
                serde_json::from_slice(&jq_out.stdout).unwrap_or(serde_json::Value::Null);

            assert_eq!(
                rust_json, jq_json,
                "fixture {filename}: differential mismatch!\nRust:\n{}\njq:\n{}",
                serde_json::to_string_pretty(&rust_json).unwrap(),
                serde_json::to_string_pretty(&jq_json).unwrap(),
            );
        }
        count += 1;
    }

    assert!(
        count >= 23,
        "expected at least 23 fixtures tested, got {count}"
    );
    println!("Differential test passed for all {count} fixtures against plan.jq");
}

#[test]
fn test_reconcile_only_with_custom_tab_converges_immediately() {
    // dec5a30 bugfix test:
    // only-custom-tab.json has workers with grid_group="quaqua".
    // When --tab mono-workers is specified, ops whose group is not mono-workers
    // are ignored, so remaining ops is empty and it converges immediately (exit 0).
    let fix = fixtures_dir().join("only-custom-tab.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let snap = parse_snapshot_value(&val).unwrap();

    let mut driver = FakeHerdrDriver::new(snap);
    let opts = ReconcileOptions {
        check: true,
        repair: false,
        tab: Some("mono-workers".to_string()),
        only: Some("sude-isl,quansu-islr1".to_string()),
        owner: Some("cos".to_string()),
        dry_run: true,
        timeout_sec: Some(0),
    };

    let res = run_reconcile(&mut driver, &opts, "workers");
    assert!(
        res.is_ok(),
        "custom tab mono-workers must converge immediately: {:?}",
        res
    );

    // Conversely, with matching tab quaqua-workers, pending ops must be found
    let opts_matching = ReconcileOptions {
        check: true,
        repair: false,
        tab: Some("quaqua-workers".to_string()),
        only: Some("sude-isl,quansu-islr1".to_string()),
        owner: Some("cos".to_string()),
        dry_run: true,
        timeout_sec: Some(0),
    };
    let res_matching = run_reconcile(&mut driver, &opts_matching, "workers");
    assert!(
        res_matching.is_err(),
        "matching tab quaqua-workers must report pending ops"
    );
}

#[test]
fn test_fake_herdr_driver_applies_ops_cleanly() {
    let fix = fixtures_dir().join("2-workers.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let snap = parse_snapshot_value(&val).unwrap();

    let ops = plan(&snap).unwrap();
    assert_eq!(ops.len(), 2);
    let mut driver = FakeHerdrDriver::new(snap.clone());
    let applied = apply_ops(&mut driver, &ops, Some(&snap)).unwrap();
    assert_eq!(applied, 2);
    assert_eq!(driver.executed_moves.len(), 2);
    assert_eq!(driver.executed_moves[0].0, "w1:p5");
    assert_eq!(driver.executed_moves[1].0, "w1:p6");
}

#[test]
fn test_parse_move_response_and_changed_false() {
    use quaqua::grid::apply::parse_move_response;

    // 1. Success response matching real herdr shape (.result.move_result)
    let ok_json = serde_json::json!({
        "result": {
            "move_result": {
                "changed": true,
                "reason": "ok",
                "pane": {
                    "pane_id": "w1:p10",
                    "tab_id": "w1:t5"
                },
                "previous_pane_id": "w1:p5"
            }
        }
    });
    let res = parse_move_response(&ok_json, "w1:p5").unwrap();
    assert_eq!(res.pane_id, "w1:p10");
    assert_eq!(res.tab_id, "w1:t5");
    assert_eq!(res.previous_pane_id, Some("w1:p5".to_string()));

    // 2. changed=false response (e.g. same_tab or zoomed_tab)
    let failed_json = serde_json::json!({
        "result": {
            "move_result": {
                "changed": false,
                "reason": "same_tab"
            }
        }
    });
    let err = parse_move_response(&failed_json, "w1:p5").unwrap_err();
    assert!(
        err.contains("changed=false, reason=same_tab"),
        "unexpected error message: {err}"
    );
}

#[test]
fn test_deferred_op_propagates_to_dependent_splits() {
    use quaqua::grid::types::Op;
    // When op 0 (new_tab) is deferred, subsequent split ops targeting @workers must also be deferred
    let ops = vec![
        Op {
            op: "move".to_string(),
            pane: "w1:p5".to_string(),
            deferred: true,
            target: Target::NewTab {
                label: "quaqua-workers".to_string(),
            },
        },
        Op {
            op: "move".to_string(),
            pane: "w1:p6".to_string(),
            deferred: false,
            target: Target::Split {
                tab: "@workers".to_string(),
                anchor: "w1:p5".to_string(),
                direction: "right".to_string(),
                ratio: 0.5,
            },
        },
    ];

    let mut driver = FakeHerdrDriver::new(quaqua::grid::Snapshot {
        workspaces: vec![],
        tabs: vec![],
        panes: vec![],
        layouts: vec![],
        agents: vec![],
    });
    let applied = apply_ops(&mut driver, &ops, None).unwrap();
    // Neither op should be applied!
    assert_eq!(applied, 0);
    assert_eq!(driver.executed_moves.len(), 0);
}

#[test]
fn test_fake_clock_now_sec() {
    use quaqua::grid::reconcile::now_sec;
    let tmp_clock = std::env::temp_dir().join("test_fake_clock.txt");
    fs::write(&tmp_clock, "1729000000\n").unwrap();

    std::env::set_var("PROMPT_WORKER_CLOCK_FILE", &tmp_clock);
    let t = now_sec();
    std::env::remove_var("PROMPT_WORKER_CLOCK_FILE");
    let _ = fs::remove_file(&tmp_clock);

    assert_eq!(t, 1729000000);
}
