use quaqua::config::{self, Config};
use quaqua::identity::{Caller, CallerDepth, IdentitySource, Role};
use quaqua::layout::{
    builtin, check, check_with_count, find_lead_in_owner_chain, layout_sha256,
    read_layout_snapshot, resolve_caller_by_name, resolve_effective_layout, resolve_layout,
    run_refreeze, run_show, run_why, why, write_layout_snapshot, Layout, LayoutOrigin, Naming,
    RoleClass, Verb,
};
use quaqua::layout::migrate::{format_diff, migrate_team_toml, run_migrate, MigrateResult};
use std::path::PathBuf;

static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn hermetic_temp_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("quaqua-layout-test-{prefix}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create hermetic temp dir");
    dir
}

#[test]
fn layout_evidence_1_from_legacy_edge_set_equals_slp_builtin() {
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let cfg: Config = toml::from_str(template_toml).expect("parse template team.toml");
    let legacy = Layout::from_legacy(&cfg);
    let slp = builtin::get("slp").expect("slp built-in");
    assert_eq!(legacy.edge_set(), slp.edge_set());
}

#[test]
fn layout_evidence_2_v3_use_slp_equals_legacy() {
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let v3_toml = template_toml
        .replace("schema_version = 2", "schema_version = 3\n[layout]\nuse = \"slp\"")
        .replace("max_workers_per_orchestrator = 8", "max_workers_per_orchestrator = 3")
        .replace("max_peers = 4", "max_peers = 2");
    let cfg: Config = toml::from_str(&v3_toml).expect("parse v3 config");
    let v3_layout = resolve_layout(&cfg).expect("resolve v3 layout");
    let legacy_layout = Layout::from_legacy(&cfg);

    assert_eq!(v3_layout.roles, legacy_layout.roles);
    assert_eq!(v3_layout.roles["cos"].max_children, Some(3));
    assert_eq!(v3_layout.roles["lead"].max_children, Some(2));
    assert_eq!(v3_layout.edge_set(), legacy_layout.edge_set());
    assert_eq!(v3_layout.depth, legacy_layout.depth);
}

#[test]
fn layout_evidence_3_schema_version_4_refused_with_migrate_hint() {
    let toml_v4 = r#"
schema_version = 4
"#;
    let cfg: Config = toml::from_str(toml_v4).expect("parse v4 config");
    let err = config::validate(&cfg).expect_err("schema_version 4 must be refused");
    assert!(
        err.contains("migrate"),
        "error '{err}' must contain migrate hint"
    );
}

#[test]
fn layout_evidence_4_why_rule_id_equals_check_rule_id() {
    let slp = builtin::get("slp").expect("slp built-in");
    let runs_dir = hermetic_temp_dir("ev4");
    std::fs::write(runs_dir.join("mu-lead.owner"), "cos\n").expect("write owner");
    let caller = resolve_caller_by_name("mu-lead", &runs_dir);

    // Case A: allowed edge
    let dec = check(&slp, &caller, Verb::Spawn, "peer");
    let (why_rule_id, _trace) = why(&slp, &caller, Verb::Spawn, "peer");
    assert_eq!(why_rule_id, dec.rule_id);
    assert_eq!(dec.rule_id, "slp.lead.can_spawn.peer");
    assert!(dec.allow);

    // Case B: denied edge
    let dec_deny = check(&slp, &caller, Verb::Spawn, "lead");
    let (why_deny_rule_id, _trace) = why(&slp, &caller, Verb::Spawn, "lead");
    assert_eq!(why_deny_rule_id, dec_deny.rule_id);
    assert_eq!(dec_deny.rule_id, "edge.none");
    assert!(!dec_deny.allow);

    let _ = std::fs::remove_dir_all(&runs_dir);
}

#[test]
fn layout_evidence_live_team_toml_loads() {
    if let Ok(home) = std::env::var("HOME") {
        let live_path = std::path::PathBuf::from(home).join("quaqua/team.toml");
        if live_path.exists() {
            let content = std::fs::read_to_string(&live_path).expect("read live team.toml");
            let cfg: Config = toml::from_str(&content).expect("parse live team.toml");
            config::validate(&cfg).expect("validate live team.toml");
        }
    }
}

#[test]
fn layout_all_builtins_load() {
    for name in &["slp", "lead-peer", "advisor", "dual-lens", "council"] {
        let layout = builtin::get(name).unwrap_or_else(|| panic!("failed to load built-in '{name}'"));
        assert_eq!(&layout.name, *name);
        assert!(layout.depth > 0);
        assert!(!layout.roles.is_empty());
    }
}

#[test]
fn layout_council_members_fit_without_touching_taskforce() {
    let council = builtin::get("council").expect("council layout");
    let council_role = council.roles.get("council").expect("council role");
    assert_eq!(council_role.members.len(), 3);
    assert_eq!(council_role.members[0].purpose.as_deref(), Some("review"));
    assert_eq!(council_role.members[1].purpose.as_deref(), Some("review"));
    assert_eq!(council_role.members[2].purpose.as_deref(), Some("advise"));
}

#[test]
fn layout_deny_unknown_fields_interaction_with_v3() {
    // 1. Valid v3 with [layout] use = "slp"
    let valid_v3 = r#"
schema_version = 3
[layout]
use = "slp"
"#;
    let cfg: Config = toml::from_str(valid_v3).expect("v3 [layout] use must parse");
    assert_eq!(cfg.schema_version, 3);
    assert_eq!(cfg.layout.as_ref().map(|l| l.use_layout.as_str()), Some("slp"));

    // 2. Valid v3 with [layouts.review-council]
    let valid_custom_v3 = r#"
schema_version = 3
[layouts.review-council]
depth = 2
[layouts.review-council.roles.cos]
class = "cos"
can_spawn = ["chair"]
can_prompt = ["chair"]
[layouts.review-council.roles.chair]
class = "lead"
can_spawn = ["council"]
reports_to = "cos"
max_children = 3
[layouts.review-council.roles.council]
class = "taskforce"
members = [{purpose="review"},{purpose="review"},{purpose="advise"}]
"#;
    let cfg2: Config = toml::from_str(valid_custom_v3).expect("v3 [layouts.*] must parse");
    assert!(cfg2.layouts.contains_key("review-council"));
    let rc = &cfg2.layouts["review-council"];
    assert_eq!(rc.depth, 2);
    assert_eq!(rc.roles["chair"].max_children, Some(3));
    assert_eq!(rc.roles["council"].members.len(), 3);

    // 3. Unknown field on Config must be rejected by deny_unknown_fields
    let bad_config = r#"
schema_version = 3
[unknown_section]
foo = "bar"
"#;
    let err = toml::from_str::<Config>(bad_config).expect_err("unknown field on Config must fail");
    assert!(err.to_string().contains("unknown field"), "err: {err}");

    // 4. Unknown field in [layout] must be rejected
    let bad_layout = r#"
schema_version = 3
[layout]
use = "slp"
bogus_key = true
"#;
    let err = toml::from_str::<Config>(bad_layout).expect_err("unknown field in [layout] must fail");
    assert!(err.to_string().contains("unknown field"), "err: {err}");
}

#[test]
fn layout_check_all_layers() {
    let slp = builtin::get("slp").expect("slp built-in");

    // 1. Identity layer failure
    let unknown_caller = Caller {
        name: "unknown".to_string(),
        role: Role::Unknown,
        depth: CallerDepth::Unknown,
        source: IdentitySource::Unknown,
    };
    let d1 = check(&slp, &unknown_caller, Verb::Spawn, "peer");
    assert!(!d1.allow);
    assert_eq!(d1.rule_id, "identity.unknown");

    // 2. Role layer failure: caller role defined in caller struct but absent from layout
    // (Note: we test with a caller whose role as_str is not in layout.roles)
    let unknown_role_caller = Caller {
        name: "custom-agent".to_string(),
        role: Role::Unknown,
        depth: CallerDepth::Resolved(1),
        source: IdentitySource::PaneTokens,
    };
    let d2 = check(&slp, &unknown_role_caller, Verb::Spawn, "peer");
    assert!(!d2.allow);
    assert_eq!(d2.rule_id, "identity.unknown");

    // 3. Edge layer failure: peer cannot spawn
    let peer_caller = Caller {
        name: "worker-1".to_string(),
        role: Role::Peer,
        depth: CallerDepth::Resolved(2),
        source: IdentitySource::PaneTokens,
    };
    let d3 = check(&slp, &peer_caller, Verb::Spawn, "peer");
    assert!(!d3.allow);
    assert_eq!(d3.rule_id, "edge.none");

    // 4. Depth layer failure: caller depth 2 spawning would exceed depth limit 2
    let lead_caller_depth_2 = Caller {
        name: "deep-lead".to_string(),
        role: Role::Lead,
        depth: CallerDepth::Resolved(2),
        source: IdentitySource::PaneTokens,
    };
    let d4 = check(&slp, &lead_caller_depth_2, Verb::Spawn, "peer");
    assert!(!d4.allow);
    assert_eq!(d4.rule_id, "depth.exceeded");

    // 5. Cap layer: child count at or over max_children (lead has max_children = 4)
    let lead_caller = Caller {
        name: "my-lead".to_string(),
        role: Role::Lead,
        depth: CallerDepth::Resolved(1),
        source: IdentitySource::PaneTokens,
    };
    let d5_denied = check_with_count(&slp, &lead_caller, Verb::Spawn, "peer", Some(4));
    assert!(!d5_denied.allow);
    assert_eq!(d5_denied.rule_id, "cap.exceeded");

    let d5_allowed = check_with_count(&slp, &lead_caller, Verb::Spawn, "peer", Some(3));
    assert!(d5_allowed.allow);
    assert_eq!(d5_allowed.rule_id, "slp.lead.can_spawn.peer");
}

#[test]
fn layout_check_mail_rules() {
    let slp = builtin::get("slp").expect("slp built-in");

    let lead_caller = Caller {
        name: "my-lead".to_string(),
        role: Role::Lead,
        depth: CallerDepth::Resolved(1),
        source: IdentitySource::PaneTokens,
    };
    // lead reports_to cos -> mail allowed
    let d_lead_cos = check(&slp, &lead_caller, Verb::Mail, "cos");
    assert!(d_lead_cos.allow);
    assert_eq!(d_lead_cos.rule_id, "slp.lead.reports_to.cos");

    let cos_caller = Caller {
        name: "cos".to_string(),
        role: Role::Cos,
        depth: CallerDepth::Resolved(0),
        source: IdentitySource::PaneTokens,
    };
    // cos can_prompt lead -> mail allowed
    let d_cos_lead = check(&slp, &cos_caller, Verb::Mail, "lead");
    assert!(d_cos_lead.allow);
    assert_eq!(d_cos_lead.rule_id, "slp.cos.can_mail.lead");

    let peer_caller = Caller {
        name: "worker-1".to_string(),
        role: Role::Peer,
        depth: CallerDepth::Resolved(2),
        source: IdentitySource::PaneTokens,
    };
    // peer cannot mail lead directly (reports_to = owner)
    let d_peer_lead = check(&slp, &peer_caller, Verb::Mail, "lead");
    assert!(!d_peer_lead.allow);
    assert_eq!(d_peer_lead.rule_id, "edge.none");
}

#[test]
fn layout_cli_show_and_why() {
    let _guard = TEST_ENV_LOCK.lock().unwrap();
    let sandbox = hermetic_temp_dir("cli");
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let cfg_path = sandbox.join("team.toml");
    std::fs::write(&cfg_path, template_toml).expect("write team.toml");

    let prev_team = std::env::var("QUAQUA_TEAM_TOML").ok();
    let prev_runs = std::env::var("QUAQUA_RUNS_DIR").ok();
    std::env::set_var("QUAQUA_TEAM_TOML", &cfg_path);
    std::env::set_var("QUAQUA_RUNS_DIR", &sandbox);

    std::fs::write(sandbox.join("mu-lead.owner"), "cos\n").expect("write mu-lead owner");

    // Show text format
    let show_text = run_show(None, false).expect("run_show text");
    assert!(show_text.contains("Layout: slp"));
    assert!(show_text.contains("Depth:  2"));
    assert!(show_text.contains("cos (class: cos)"));

    // Show JSON format
    let show_json = run_show(None, true).expect("run_show json");
    let json_val: serde_json::Value = serde_json::from_str(&show_json).expect("parse json");
    assert_eq!(json_val["name"], "slp");
    assert_eq!(json_val["depth"], 2);

    // Why text allowed
    let why_allow = run_why("mu-lead", "spawn", "peer", None, false).expect("run_why allow");
    assert!(why_allow.contains("[✓] identity"));
    assert!(why_allow.contains("[✓] role"));
    assert!(why_allow.contains("[✓] edge"));
    assert!(why_allow.contains("Decision: ALLOWED (rule_id: slp.lead.can_spawn.peer)"));

    // Why text denied
    let why_deny = run_why("mu-lead", "spawn", "lead", None, false).expect("run_why deny");
    assert!(why_deny.contains("[✗] edge"));
    assert!(why_deny.contains("Decision: DENIED (rule_id: edge.none)"));

    // Why JSON
    let why_json = run_why("mu-lead", "spawn", "peer", None, true).expect("run_why json");
    let json_why: serde_json::Value = serde_json::from_str(&why_json).expect("parse why json");
    assert_eq!(json_why["code"], "allowed");
    assert_eq!(json_why["rule_id"], "slp.lead.can_spawn.peer");
    assert_eq!(json_why["caller"], "mu-lead");
    assert_eq!(json_why["target"], "peer");

    match prev_team {
        Some(t) => std::env::set_var("QUAQUA_TEAM_TOML", t),
        None => std::env::remove_var("QUAQUA_TEAM_TOML"),
    }
    match prev_runs {
        Some(r) => std::env::set_var("QUAQUA_RUNS_DIR", r),
        None => std::env::remove_var("QUAQUA_RUNS_DIR"),
    }
    let _ = std::fs::remove_dir_all(&sandbox);
}

// ---------------------------------------------------------------------------
// Regression tests for review findings F1 - F7
// ---------------------------------------------------------------------------

#[test]
fn layout_repro_f1_unknown_caller_resolves_to_identity_unknown() {
    let sandbox = hermetic_temp_dir("f1");
    let caller = resolve_caller_by_name("nobody-xyz", &sandbox);
    assert_eq!(caller.role, Role::Unknown);
    let slp = builtin::get("slp").expect("slp built-in");
    let dec = check(&slp, &caller, Verb::Spawn, "peer");
    assert!(!dec.allow);
    assert_eq!(dec.rule_id, "identity.unknown");
    let _ = std::fs::remove_dir_all(&sandbox);
}

#[test]
fn layout_repro_f2_depth_cycle_error_reaches_gate() {
    let sandbox = hermetic_temp_dir("f2");
    std::fs::write(sandbox.join("cyc-lead.owner"), "cyc-lead\n").expect("write cycle owner");
    let caller = resolve_caller_by_name("cyc-lead", &sandbox);
    assert!(matches!(caller.depth, CallerDepth::Cycle(_)));
    let slp = builtin::get("slp").expect("slp built-in");
    let dec = check(&slp, &caller, Verb::Spawn, "peer");
    assert!(!dec.allow);
    assert_eq!(dec.rule_id, "depth.exceeded");
    let _ = std::fs::remove_dir_all(&sandbox);
}

#[test]
fn layout_repro_f3_v3_use_slp_retains_live_caps_and_depth() {
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let v3_toml = template_toml
        .replace("schema_version = 2", "schema_version = 3\n[layout]\nuse = \"slp\"")
        .replace("max_workers_per_orchestrator = 8", "max_workers_per_orchestrator = 3")
        .replace("max_peers = 4", "max_peers = 2");
    let cfg: Config = toml::from_str(&v3_toml).expect("parse v3 config");
    let resolved = resolve_layout(&cfg).expect("resolve v3 slp");
    let legacy = Layout::from_legacy(&cfg);
    assert_eq!(resolved.roles["cos"].max_children, Some(3));
    assert_eq!(resolved.roles["lead"].max_children, Some(2));
    assert_eq!(resolved.roles, legacy.roles);
    assert_eq!(resolved.edge_set(), legacy.edge_set());
    assert_eq!(resolved.depth, legacy.depth);
}

#[test]
fn layout_repro_f4_v2_with_layout_refused() {
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let v2_with_layout = format!("{template_toml}\n[layout]\nuse = \"nope\"\n");
    let cfg: Config = toml::from_str(&v2_with_layout).expect("parse v2 with layout");
    let err = config::validate(&cfg).expect_err("v2 with [layout] must be refused");
    assert!(err.contains("schema_version") || err.contains("[layout]"));
}

#[test]
fn layout_repro_f5_unknown_target_role_denied() {
    let slp = builtin::get("slp").expect("slp built-in");
    let cos = Caller {
        name: "cos".to_string(),
        role: Role::Cos,
        depth: CallerDepth::Resolved(0),
        source: IdentitySource::PaneTokens,
    };
    let dec = check(&slp, &cos, Verb::Spawn, "bogus");
    assert!(!dec.allow);
    assert_ne!(dec.rule_id, "slp.cos.can_spawn.peer");
}

#[test]
fn layout_repro_f6_user_layout_colliding_with_builtin_refused() {
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let v3_collision = template_toml
        .replace("schema_version = 2", "schema_version = 3\n[layout]\nuse = \"slp\"\n[layouts.slp]\ndepth = 1\n[layouts.slp.roles.cos]\nclass = \"cos\"");
    let cfg: Config = toml::from_str(&v3_collision).expect("parse collision config");
    let err = config::validate(&cfg).expect_err("collision with built-in must be refused");
    assert!(err.contains("collides"));
}

#[test]
fn layout_repro_f7_v3_layout_invalid_role_refs_refused() {
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let v3_bad_role = template_toml
        .replace("schema_version = 2", "schema_version = 3\n[layout]\nuse = \"x\"\n[layouts.x]\ndepth = 2\n[layouts.x.roles.cos]\nclass = \"cos\"\ncan_spawn = [\"chiar\"]");
    let cfg: Config = toml::from_str(&v3_bad_role).expect("parse invalid role config");
    let err = config::validate(&cfg).expect_err("invalid role ref must be refused");
    assert!(err.contains("unknown role") || err.contains("chiar"));
}

// ---------------------------------------------------------------------------
// P7 Proving Tests: Snapshots, Refreeze, Drift Warning, and Migration
// ---------------------------------------------------------------------------

#[test]
fn layout_evidence_a_team_toml_edit_does_not_change_gate_decisions() {
    let dir = hermetic_temp_dir("p7-a");
    let runs_dir = dir.join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();

    // Binding rule: real lead name '<slug>-lead'
    std::fs::write(runs_dir.join("frz-lead.owner"), "cos\n").unwrap();
    std::fs::write(runs_dir.join("frz-peer.owner"), "frz-lead\n").unwrap();

    // Base template config where peer cannot prompt peer
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let cfg: Config = toml::from_str(template_toml).expect("parse template team.toml");

    // Write frozen layout snapshot for project "frz"
    let snap_path = write_layout_snapshot(&runs_dir, "frz", &cfg).unwrap();
    assert!(snap_path.is_file());

    // Initial gate check: frz-peer cannot prompt peer
    let caller = resolve_caller_by_name("frz-peer", &runs_dir);
    let (layout_before, origin_before) =
        resolve_effective_layout("frz-peer", &runs_dir, &cfg).unwrap();
    assert!(origin_before.is_snapshot());
    let dec_before = check(&layout_before, &caller, Verb::Prompt, "peer");
    assert!(!dec_before.allow);
    assert_eq!(dec_before.rule_id, "edge.none");

    // Edit team.toml config to allow peer prompt
    let mut edited_cfg = cfg.clone();
    edited_cfg.roles.get_mut("peer").unwrap().can_prompt = vec!["peer".to_string()];

    // Gate decision for frz-peer does NOT change because it resolves from frozen snapshot
    let (layout_after, origin_after) =
        resolve_effective_layout("frz-peer", &runs_dir, &edited_cfg).unwrap();
    assert!(origin_after.is_snapshot());
    let dec_after = check(&layout_after, &caller, Verb::Prompt, "peer");
    assert!(!dec_after.allow);
    assert_eq!(dec_after.rule_id, "edge.none");
    assert_eq!(layout_after.roles["peer"].can_prompt, Vec::<String>::new());

    // Verify CoS-owned work uses live file, not snapshot
    std::fs::write(runs_dir.join("cos-worker.owner"), "cos\n").unwrap();
    let cos_caller = resolve_caller_by_name("cos-worker", &runs_dir);
    let (cos_layout, cos_origin) =
        resolve_effective_layout("cos-worker", &runs_dir, &edited_cfg).unwrap();
    assert!(!cos_origin.is_snapshot());
    let cos_dec = check(&cos_layout, &cos_caller, Verb::Prompt, "peer");
    assert!(cos_dec.allow, "CoS-owned work must use live config immediately");
}

#[test]
fn layout_evidence_b_refreeze_applies_team_toml_edit() {
    let dir = hermetic_temp_dir("p7-b");
    let runs_dir = dir.join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();

    // Real lead name 'frz-lead'
    std::fs::write(runs_dir.join("frz-lead.owner"), "cos\n").unwrap();
    std::fs::write(runs_dir.join("frz-peer.owner"), "frz-lead\n").unwrap();

    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let cfg: Config = toml::from_str(template_toml).expect("parse template team.toml");
    write_layout_snapshot(&runs_dir, "frz", &cfg).unwrap();

    // Edit live config
    let mut edited_cfg = cfg.clone();
    edited_cfg.roles.get_mut("peer").unwrap().can_prompt = vec!["peer".to_string()];

    // Refreeze snapshot with edited config
    let _guard = TEST_ENV_LOCK.lock().unwrap();
    let edited_toml = template_toml.replace(
        "can_prompt = []",
        "can_prompt = [\"peer\"]",
    );
    let team_path = dir.join("team.toml");
    std::fs::write(&team_path, &edited_toml).unwrap();
    let prev_runs = std::env::var("QUAQUA_RUNS_DIR").ok();
    let prev_team = std::env::var("QUAQUA_TEAM_TOML").ok();
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_TEAM_TOML", &team_path);
    let refreeze_msg = run_refreeze("frz").expect("run_refreeze should succeed");
    assert!(refreeze_msg.contains("refroze layout for 'frz'"));
    match prev_runs {
        Some(v) => std::env::set_var("QUAQUA_RUNS_DIR", v),
        None => std::env::remove_var("QUAQUA_RUNS_DIR"),
    }
    match prev_team {
        Some(v) => std::env::set_var("QUAQUA_TEAM_TOML", v),
        None => std::env::remove_var("QUAQUA_TEAM_TOML"),
    }

    let snap = read_layout_snapshot(&runs_dir, "frz").unwrap().expect("snapshot exists");
    assert_eq!(snap.layout.roles["peer"].can_prompt, vec!["peer".to_string()]);

    // Gate decision for frz-peer now ALLOWS prompt because snapshot was refrozen
    let caller = resolve_caller_by_name("frz-peer", &runs_dir);
    let (layout_refrozen, origin_refrozen): (Layout, LayoutOrigin) =
        resolve_effective_layout("frz-peer", &runs_dir, &edited_cfg).unwrap();
    assert!(origin_refrozen.is_snapshot());
    assert_eq!(origin_refrozen.source_desc(), "snapshot frz.layout.toml");
    let sha = layout_sha256(&layout_refrozen).unwrap();
    assert_eq!(sha, snap.sha256);
    let dec_refrozen = check(&layout_refrozen, &caller, Verb::Prompt, "peer");
    assert!(dec_refrozen.allow);
    assert_eq!(layout_refrozen.roles["peer"].can_prompt, vec!["peer".to_string()]);
}

#[test]
fn layout_evidence_c_migrate_templates_workspace_team_toml_byte_identical_and_valid() {
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let res = migrate_team_toml(template_toml).expect("migrate templates/workspace/team.toml");
    let migrated = match res {
        MigrateResult::Migrated(s) => s,
        MigrateResult::AlreadyV3 => panic!("expected Migrated, got AlreadyV3"),
    };

    // Byte-identical except version line and [layout]
    let expected = template_toml.replace(
        "schema_version = 2",
        "schema_version = 3\n\n[layout]\nuse = \"slp\"",
    );
    assert_eq!(
        migrated, expected,
        "migrated template must be byte-identical except version line and [layout]"
    );

    // Result must pass validate()
    let parsed: Config = toml::from_str(&migrated).expect("parse migrated TOML as Config");
    config::validate(&parsed).expect("validate migrated Config");
}

#[test]
fn layout_evidence_d_v4_refused_with_hint() {
    let v4_toml = "# Comments\nschema_version = 4\n[limits]\ndepth = 2\n";
    let err = migrate_team_toml(v4_toml).unwrap_err();
    assert!(err.contains("schema_version 4"), "error was: {err}");
    assert!(err.contains("hint: upgrade quaqua"), "error was: {err}");
}

#[test]
fn layout_evidence_e_drift_warning_shown_on_mismatch() {
    let _guard = TEST_ENV_LOCK.lock().unwrap();
    let dir = hermetic_temp_dir("p7-e");
    let runs_dir = dir.join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();

    std::fs::write(runs_dir.join("frzdrift-lead.owner"), "cos\n").unwrap();

    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let cfg: Config = toml::from_str(template_toml).expect("parse template team.toml");
    let snap_path = write_layout_snapshot(&runs_dir, "frzdrift", &cfg).unwrap();
    assert!(snap_path.is_file());

    // Mutate live team.toml role topology
    let mutated_toml = template_toml.replace(
        "can_spawn  = [\"lead\", \"peer\", \"taskforce\"]",
        "can_spawn  = [\"lead\"]",
    );
    let team_path = dir.join("team.toml");
    std::fs::write(&team_path, &mutated_toml).unwrap();

    let prev_runs = std::env::var("QUAQUA_RUNS_DIR").ok();
    let prev_team = std::env::var("QUAQUA_TEAM_TOML").ok();
    std::env::set_var("QUAQUA_RUNS_DIR", &runs_dir);
    std::env::set_var("QUAQUA_TEAM_TOML", &team_path);

    let res = run_show(Some("frzdrift"), false);

    match prev_runs {
        Some(v) => std::env::set_var("QUAQUA_RUNS_DIR", v),
        None => std::env::remove_var("QUAQUA_RUNS_DIR"),
    }
    match prev_team {
        Some(v) => std::env::set_var("QUAQUA_TEAM_TOML", v),
        None => std::env::remove_var("QUAQUA_TEAM_TOML"),
    }

    let out = res.expect("run_show should succeed");
    assert!(
        out.contains("warning: frozen sha ≠ live sha"),
        "expected drift warning in output: {out}"
    );
    assert!(
        out.contains("Layout: slp (source: snapshot frzdrift.layout.toml)"),
        "expected snapshot source in output: {out}"
    );
}

#[test]
fn layout_p7_migrate_already_v3_is_noop_exit_0() {
    let v3_toml = "schema_version = 3\n\n[layout]\nuse = \"slp\"\n\n[limits]\ndepth = 2\nmax_workers_per_orchestrator = 4\n";
    let res = migrate_team_toml(v3_toml).expect("migrate already-v3 succeeds");
    assert_eq!(res, MigrateResult::AlreadyV3);
}

#[test]
fn layout_p7_migrate_custom_roles_moves_to_layouts_custom() {
    let v2_custom = r#"# custom v2
schema_version = 2

[limits]
depth = 2
max_workers_per_orchestrator = 4

[merge]
integ_branch = "quaqua/{slug}-integ"
[roles.cos]
can_spawn = ["lead", "taskforce"]
can_prompt = ["lead"]

[roles.lead]
can_spawn = ["peer", "taskforce"]
can_prompt = ["peer"]
reports_to = "cos"
max_peers = 3

[roles.peer]
can_spawn = []
can_prompt = []
reports_to = "owner"

[strategy]
default_kind = "sude"
default_reviewer = ["suhuynh"]
builder_reviewer_differ = "none"

[kinds.sude]
harness = "omp"
model = "google-antigravity/gemini-3.8-flash"
effort = "high"
can_review = true

[kinds.suhuynh]
harness = "claude"
model = "opus-5.5"
effort = "medium"
can_review = true
"#;
    let res = migrate_team_toml(v2_custom).expect("migrate custom roles");
    let migrated = match res {
        MigrateResult::Migrated(s) => s,
        _ => panic!("expected Migrated"),
    };
    assert!(migrated.contains("[layout]\nuse = \"custom\""));
    assert!(migrated.contains("[layouts.custom]"));
    assert!(migrated.contains("depth = 2"));
    assert!(migrated.contains("[layouts.custom.roles.cos]"));
    assert!(migrated.contains("[layouts.custom.roles.lead]"));
    assert!(migrated.contains("[layouts.custom.roles.peer]"));
    assert!(migrated.contains("[layouts.custom.roles.taskforce]"));
    assert!(migrated.contains("max_children = 3"));
    assert!(migrated.contains("[roles.lead]\nmax_peers = 3"));

    let v2_cfg: Config = toml::from_str(v2_custom).expect("parse v2 custom TOML");
    let v2_legacy = Layout::from_legacy(&v2_cfg);

    let migrated_cfg: Config = toml::from_str(&migrated).expect("parse migrated custom TOML");
    config::validate(&migrated_cfg).expect("validate migrated custom Config");

    let custom_layout = resolve_layout(&migrated_cfg).expect("resolve migrated layout");
    assert_eq!(
        custom_layout.edge_set(),
        v2_legacy.edge_set(),
        "migrated custom layout must preserve exact edge_set from legacy synthesis"
    );
    assert_eq!(
        custom_layout.roles["lead"].max_children,
        v2_legacy.roles["lead"].max_children
    );
    assert_eq!(
        custom_layout.roles["cos"].max_children,
        v2_legacy.roles["cos"].max_children
    );
    assert_eq!(custom_layout.roles["lead"].naming, Naming::SlugLead);
    assert_eq!(
        custom_layout.roles["taskforce"].class,
        RoleClass::Taskforce
    );
    assert_eq!(
        quaqua::cap::owner_cap(&migrated_cfg, "x-lead"),
        quaqua::cap::owner_cap(&v2_cfg, "x-lead"),
        "migrated config must preserve live Lead Peer cap"
    );
}

#[test]
fn layout_p7_migrate_dry_run_unified_diff() {
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let res = migrate_team_toml(template_toml).expect("migrate succeeds");
    let migrated = match res {
        MigrateResult::Migrated(s) => s,
        _ => panic!("expected Migrated"),
    };

    let diff = format_diff(template_toml, &migrated, "team.toml");
    assert!(diff.starts_with("--- team.toml\n+++ team.toml\n@@ -4,8 +4,11 @@\n"));
    assert!(diff.contains("-schema_version = 2\n"));
    assert!(diff.contains("+schema_version = 3\n"));
    assert!(diff.contains("+[layout]\n+use = \"slp\"\n"));
    // Crucial check from Finding 1: verify no false churn after the insertion
    assert!(!diff.contains("-[limits]"));
    assert!(!diff.contains("+[limits]"));
    assert!(!diff.contains("-[timers]"));
    assert!(!diff.contains("+[timers]"));
}

#[test]
fn layout_p7_run_migrate_write_and_bak() {
    let dir = hermetic_temp_dir("p7-bak");
    let team_path = dir.join("team.toml");
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    std::fs::write(&team_path, template_toml).unwrap();

    // Dry run first
    let dry_out = run_migrate(Some(&team_path), false).expect("dry run succeeds");
    assert!(dry_out.contains("-schema_version = 2\n"));
    assert!(dry_out.contains("+schema_version = 3\n"));
    assert!(!dir.join("team.toml.bak").exists());

    // Write run
    let write_out = run_migrate(Some(&team_path), true).expect("write run succeeds");
    assert!(write_out.contains("backup at"));

    // Verify backup created with original content
    let bak_path = dir.join("team.toml.bak");
    assert!(bak_path.is_file(), "backup file must exist");
    let bak_content = std::fs::read_to_string(&bak_path).unwrap();
    assert_eq!(bak_content, template_toml);

    // Verify team.toml updated
    let new_content = std::fs::read_to_string(&team_path).unwrap();
    assert!(new_content.contains("schema_version = 3"));
    assert!(new_content.contains("[layout]\nuse = \"slp\""));

    // Subsequent run on already-v3 file is a no-op
    let noop_out = run_migrate(Some(&team_path), false).expect("noop succeeds");
    assert_eq!(noop_out, "already at schema_version 3\n");
}

#[test]
fn layout_p7_find_lead_in_owner_chain_cycle_safe() {
    let dir = hermetic_temp_dir("cycle");
    let runs_dir = dir.join("runs");
    std::fs::create_dir_all(&runs_dir).unwrap();

    // Cycle: worker-a -> worker-b -> worker-a
    std::fs::write(runs_dir.join("worker-a.owner"), "worker-b\n").unwrap();
    std::fs::write(runs_dir.join("worker-b.owner"), "worker-a\n").unwrap();

    let found = find_lead_in_owner_chain(&runs_dir, "worker-a");
    assert_eq!(found, None, "cycles must terminate cleanly with None");
}
