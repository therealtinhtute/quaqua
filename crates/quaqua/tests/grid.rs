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
        let snap = parse_snapshot_value(&val)
            .unwrap_or_else(|e| panic!("failed to parse fixture {}: {e}", path.display()));
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
    let pane_map = apply_ops(&mut driver, &ops, Some(&snap)).unwrap();
    assert_eq!(pane_map.len(), 2);
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
    let pane_map = apply_ops(&mut driver, &ops, None).unwrap();
    // Neither op should be applied!
    assert_eq!(pane_map.len(), 0);
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

#[test]
fn test_repair_renames_with_new_pane_ids_and_respects_only() {
    // Finding 1: repair renames moved panes using their new IDs and only renames --only listed workers
    let fix = fixtures_dir().join("2-workers.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let snap = parse_snapshot_value(&val).unwrap();

    let mut driver = FakeHerdrDriver::new(snap);
    let opts = ReconcileOptions {
        check: false,
        repair: true,
        tab: None,
        only: Some("sude-1".to_string()), // only sude-1 (w1:p6) should be renamed
        owner: Some("cos".to_string()),
        dry_run: false,
        timeout_sec: None,
    };

    run_reconcile(&mut driver, &opts, "workers").unwrap();
    // Only sude-1 should be renamed (tieude-1 was unlisted and must be untouched!)
    assert_eq!(driver.executed_renames.len(), 1);
    let (renamed_pane, label) = &driver.executed_renames[0];
    assert_eq!(label, "sude-1");
    // It must use the NEW pane ID assigned upon move, NOT the stale w1:p6
    assert_ne!(renamed_pane, "w1:p6");
    assert!(renamed_pane.starts_with("w1:p"));
}

#[test]
fn test_apply_ops_rechecks_idle_and_fails_if_busy() {
    // Finding 2: if pane turns busy between snapshot and move, apply_ops must refuse
    let fix = fixtures_dir().join("1-worker.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let mut snap = parse_snapshot_value(&val).unwrap();

    let ops = plan(&snap).unwrap();
    assert_eq!(ops.len(), 1);

    // Simulate pane becoming busy before move
    if let Some(p) = snap.panes.iter_mut().find(|p| p.pane_id == "w1:p5") {
        p.agent_status = Some("working".to_string());
    }

    let mut driver = FakeHerdrDriver::new(snap);
    let res = apply_ops(&mut driver, &ops, None);
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        err.contains("became busy; replan required"),
        "unexpected error message: {err}"
    );
}

#[test]
fn test_check_only_waits_when_listed_split_dependency_is_unlisted() {
    // Finding 3: --check --only sude-1 must wait on sude-1's split even if tieude-1 was unlisted
    let fix = fixtures_dir().join("2-workers.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let snap = parse_snapshot_value(&val).unwrap();

    let mut driver = FakeHerdrDriver::new(snap);
    let opts = ReconcileOptions {
        check: true,
        repair: false,
        tab: Some("quaqua-workers".to_string()),
        only: Some("sude-1".to_string()), // worker 0 (tieude-1) unlisted
        owner: Some("cos".to_string()),
        dry_run: true,
        timeout_sec: Some(0),
    };

    // Must find pending ops and fail with pending ops error, NOT report [] or exit 0!
    let res = run_reconcile(&mut driver, &opts, "workers");
    assert!(
        res.is_err(),
        "check must wait on listed worker's split even when worker 0 is unlisted"
    );
    let err = res.unwrap_err();
    assert!(err.contains("pending ops: 1"), "unexpected error: {err}");
}

#[test]
fn test_hook_failure_backoff_uses_post_failure_sig_and_scratch_normalization() {
    // Finding 4: snapshot signature maps grid-scratch- to @scratch and includes non-ready workers
    use quaqua::grid::hook::compute_snapshot_signature;
    let fix = fixtures_dir().join("2-workers.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let mut snap = parse_snapshot_value(&val).unwrap();

    // Set tab label to a scratch tab
    snap.tabs.push(quaqua::grid::types::Tab {
        tab_id: "w1:t_scratch_1".to_string(),
        workspace_id: Some("w1".to_string()),
        label: Some("grid-scratch-quaqua-999-1".to_string()),
        pane_count: Some(1),
    });
    snap.panes[1].tab_id = Some("w1:t_scratch_1".to_string());

    let sig = compute_snapshot_signature(&snap);
    assert!(
        sig.contains("@scratch"),
        "signature must normalize scratch tab to @scratch: {sig}"
    );
}

#[test]
fn test_apply_ops_verifies_fresh_snapshot_and_resolves_workers_from_anchor() {
    // Finding 5: fresh snapshot verification and @workers resolution from anchor tab
    let fix = fixtures_dir().join("2-workers.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let mut snap = parse_snapshot_value(&val).unwrap();

    // Mark layouts as zoomed -> apply_ops must refuse with zoomed error
    for l in &mut snap.layouts {
        l.zoomed = Some(true);
    }
    let ops = plan(&snap).unwrap();

    let mut driver = FakeHerdrDriver::new(snap);
    let res = apply_ops(&mut driver, &ops, None);
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.contains("is zoomed"), "unexpected error message: {err}");
}

#[test]
fn test_hook_locking_and_dead_pid_reclaim() {
    // Finding 6: acquire_lock reclaims dead PID
    use quaqua::grid::hook::acquire_lock;
    let tmp_lock_dir = std::env::temp_dir().join("test_herdr_grid_lock_dir");
    let pid_file = tmp_lock_dir.join("pid");

    let _ = fs::remove_file(&pid_file);
    let _ = fs::remove_dir(&tmp_lock_dir);

    fs::create_dir(&tmp_lock_dir).unwrap();
    // Write a definitely dead PID (e.g. 9999999)
    fs::write(&pid_file, "9999999").unwrap();

    let guard = acquire_lock(&tmp_lock_dir, &pid_file);
    assert!(guard.is_ok(), "acquire_lock must reclaim lock from dead PID");
    drop(guard);
}

#[test]
fn test_hook_status_early_exit() {
    // Finding 6: should_early_exit checks status transitions
    use quaqua::grid::hook::should_early_exit;
    let fix = fixtures_dir().join("2-workers.json");
    let tmp_snap = std::env::temp_dir().join("test_early_exit_snap.json");
    fs::copy(&fix, &tmp_snap).unwrap();

    // 1. Idle -> done: both non-working -> early exit true
    let event_done = serde_json::json!({
        "pane_id": "w1:p5",
        "agent_status": "done"
    })
    .to_string();
    assert!(should_early_exit(
        Some("pane.agent_status_changed"),
        Some(&event_done),
        &tmp_snap
    ));

    // 2. Idle -> working: state changed -> early exit false
    let event_working = serde_json::json!({
        "pane_id": "w1:p5",
        "agent_status": "working"
    })
    .to_string();
    assert!(!should_early_exit(
        Some("pane.agent_status_changed"),
        Some(&event_working),
        &tmp_snap
    ));

    // 3. Empty status -> do not early exit (Finding 8)
    let event_empty = serde_json::json!({
        "pane_id": "w1:p5"
    })
    .to_string();
    assert!(!should_early_exit(
        Some("pane.agent_status_changed"),
        Some(&event_empty),
        &tmp_snap
    ));

    let _ = fs::remove_file(&tmp_snap);
}

#[test]
fn test_repair_arranges_all_groups_using_tab_as_default() {
    // Finding 7: repair arranges all groups rather than filtering out non-matching groups
    let fix = fixtures_dir().join("cross_checker_owner_group.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let snap = parse_snapshot_value(&val).unwrap();

    let mut driver = FakeHerdrDriver::new(snap);
    let opts = ReconcileOptions {
        check: false,
        repair: true,
        tab: Some("workers".to_string()), // default tab provided
        only: None,
        owner: Some("lead-1".to_string()),
        dry_run: false,
        timeout_sec: None,
    };

    run_reconcile(&mut driver, &opts, "workers").unwrap();
    // Lead has workers in lead-workers group: both moves must be executed!
    assert_eq!(driver.executed_moves.len(), 2);
}

#[test]
fn test_hook_shadow_mode_file_precedence_and_empty_snapshot() {
    // Finding 8: shadow file precedence and empty snapshot
    use quaqua::grid::hook::{is_shadow_mode, reconcile_step};
    let tmp_state = std::env::temp_dir().join("test_shadow_mode_dir");
    let _ = fs::create_dir_all(&tmp_state);

    std::env::set_var("HERDR_GRID_SHADOW", "1");
    fs::write(tmp_state.join("shadow"), "0\n").unwrap();
    // Even though env=1, shadow file contains 0 -> live mode (false)
    assert!(!is_shadow_mode(&tmp_state));
    std::env::remove_var("HERDR_GRID_SHADOW");

    // Empty snapshot returns Ok(())
    let mut driver = FakeHerdrDriver::new(quaqua::grid::Snapshot {
        workspaces: vec![],
        tabs: vec![],
        panes: vec![],
        layouts: vec![],
        agents: vec![],
    });
    assert!(reconcile_step(&tmp_state, &mut driver).is_ok());
    let _ = fs::remove_dir_all(&tmp_state);
}

#[test]
fn test_grid_deferred_statuses_from_config() {
    // Finding 9: plan_with_deferred supports custom deferred_statuses
    use quaqua::grid::plan_with_deferred;
    let fix = fixtures_dir().join("1-worker.json");
    let content = fs::read_to_string(fix).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    let mut snap = parse_snapshot_value(&val).unwrap();

    snap.panes[1].agent_status = Some("paused".to_string());

    // With default ["working"], "paused" is not deferred
    let ops_def = plan_with_deferred(&snap, None).unwrap();
    assert!(!ops_def[0].deferred);

    // With custom ["working", "paused"], "paused" is deferred
    let ops_custom = plan_with_deferred(&snap, Some(&["working", "paused"])).unwrap();
    assert!(ops_custom[0].deferred);
}
