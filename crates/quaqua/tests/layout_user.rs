use quaqua::config::Config;
use quaqua::identity::{Caller, CallerDepth, IdentitySource, Role};
use quaqua::layout::builtin;
use quaqua::layout::gate::{
    check, check_with_origin, check_with_runs_dir, read_layout_role, record_layout_role,
    resolve_caller_role, resolve_target_role, validate_layout, validate_layout_with_config,
    validate_user_layout,
};
use quaqua::layout::migrate::{migrate_team_toml, MigrateResult};
use quaqua::layout::model::{Layout, LayoutRole, Member, Naming, RoleClass, Verb};
use std::path::PathBuf;

static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn hermetic_temp_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "quaqua-layout-user-test-{prefix}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create hermetic temp dir");
    dir
}

fn sample_user_review_council_toml() -> &'static str {
    r#"
name = "review-council"
depth = 2

[roles.cos]
class = "cos"
can_spawn = ["chair"]
can_prompt = ["chair"]

[roles.chair]
class = "lead"
can_spawn = ["council"]
can_prompt = ["council"]
reports_to = "cos"
max_children = 4
naming = "slug-lead"

[roles.council]
class = "taskforce"
reports_to = "owner"
members = [
    { purpose = "review" },
    { purpose = "review" },
    { purpose = "design" },
]
quorum = 2
"#
}

#[test]
fn layout_user_role_by_name_chair_spawns_council() {
    let toml_str = sample_user_review_council_toml();
    let layout: Layout = toml::from_str(toml_str).expect("parse user review-council layout");
    assert!(validate_layout(&layout).is_ok());

    let runs_dir = hermetic_temp_dir("chair-spawns");

    // Case 1: Caller identified as Lead (class match to 'chair')
    let caller = Caller {
        name: "test-chair".to_string(),
        role: Role::Lead,
        depth: CallerDepth::Resolved(1),
        source: IdentitySource::PaneTokens,
    };

    // Target by concrete role name "council"
    let d1 = check_with_runs_dir(
        &layout,
        &caller,
        Verb::Spawn,
        "council",
        None,
        false,
        Some(&runs_dir),
    );
    assert!(
        d1.allow,
        "chair should be permitted to spawn council by name, got deny: {:?}",
        d1.trace
    );
    assert_eq!(d1.rule_id, "review-council.chair.can_spawn.council");

    // Target by class name "taskforce" (mapped by gate to unique candidate "council" in chair.can_spawn)
    let d2 = check_with_runs_dir(
        &layout,
        &caller,
        Verb::Spawn,
        "taskforce",
        None,
        false,
        Some(&runs_dir),
    );
    assert!(
        d2.allow,
        "chair should be permitted to spawn taskforce (mapped to council), got deny: {:?}",
        d2.trace
    );
    assert_eq!(d2.rule_id, "review-council.chair.can_spawn.council");

    // Case 2: Ambiguous caller class disambiguated via runs/<name>.layout_role
    record_layout_role(&runs_dir, "test-chair", "chair").expect("record layout role");
    assert_eq!(
        read_layout_role(&runs_dir, "test-chair"),
        Some("chair".to_string())
    );

    let d3 = check_with_runs_dir(
        &layout,
        &caller,
        Verb::Spawn,
        "council",
        None,
        false,
        Some(&runs_dir),
    );
    assert!(d3.allow);
}

#[test]
fn layout_user_cycle_past_depth_denied() {
    // 1. Self cycle: lead -> lead
    let self_cycle_toml = r#"
name = "bad-self"
depth = 2

[roles.cos]
class = "cos"
can_spawn = ["lead"]
can_prompt = ["lead"]

[roles.lead]
class = "lead"
can_spawn = ["lead"]
reports_to = "cos"
"#;
    let bad_self: Layout = toml::from_str(self_cycle_toml).expect("parse bad self cycle");
    let err1 = validate_layout(&bad_self).expect_err("self cycle must be rejected");
    assert!(
        err1.contains("layout.cycle"),
        "error must mention layout.cycle, got: {err1}"
    );

    // 2. Multi-hop cycle: a -> b -> a
    let loop_cycle_toml = r#"
name = "bad-loop"
depth = 3

[roles.cos]
class = "cos"
can_spawn = ["lead-a"]
can_prompt = ["lead-a"]

[roles.lead-a]
class = "lead"
can_spawn = ["lead-b"]
reports_to = "cos"

[roles.lead-b]
class = "lead"
can_spawn = ["lead-a"]
reports_to = "cos"
"#;
    let bad_loop: Layout = toml::from_str(loop_cycle_toml).expect("parse bad loop cycle");
    let err2 = validate_layout(&bad_loop).expect_err("loop cycle must be rejected");
    assert!(
        err2.contains("layout.cycle"),
        "error must mention layout.cycle, got: {err2}"
    );
}

#[test]
fn layout_council_builtin_lead_row_reachable() {
    let council = builtin::get("council").expect("council builtin must load");
    assert_eq!(council.depth, 2, "council depth must be 2");

    let cos_role = council.roles.get("cos").expect("cos role");
    assert!(
        cos_role.can_spawn.contains(&"lead".to_string()),
        "cos must be able to spawn lead"
    );
    assert!(
        cos_role.can_spawn.contains(&"council".to_string()),
        "cos must be able to spawn council"
    );

    let cos_caller = Caller {
        name: "cos".to_string(),
        role: Role::Cos,
        depth: CallerDepth::Resolved(0),
        source: IdentitySource::PaneTokens,
    };

    // Cos spawns lead -> target depth 1 <= depth 2
    let d_lead = check(&council, &cos_caller, Verb::Spawn, "lead");
    assert!(
        d_lead.allow,
        "cos spawning lead must be allowed, trace: {:?}",
        d_lead.trace
    );

    let lead_caller = Caller {
        name: "my-lead".to_string(),
        role: Role::Lead,
        depth: CallerDepth::Resolved(1),
        source: IdentitySource::PaneTokens,
    };

    // Lead spawns council -> target depth 2 <= depth 2
    let d_council = check(&council, &lead_caller, Verb::Spawn, "council");
    assert!(
        d_council.allow,
        "lead spawning council must be allowed at depth 2, trace: {:?}",
        d_council.trace
    );
}

#[test]
fn layout_member_unknown_purpose_rejected() {
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let cfg: Config = toml::from_str(template_toml).expect("parse workspace team.toml");

    let invalid_purpose_toml = r#"
name = "bad-purpose"
depth = 2

[roles.cos]
class = "cos"
can_spawn = ["taskforce"]
can_prompt = ["taskforce"]

[roles.taskforce]
class = "taskforce"
reports_to = "owner"
members = [
    { purpose = "nonexistent_unicorn_purpose" }
]
"#;
    let layout: Layout = toml::from_str(invalid_purpose_toml).expect("parse layout");
    // Pure structural validation passes because format is valid
    assert!(validate_layout(&layout).is_ok());

    // Config validation fails because purpose is not in [purposes]
    let err =
        validate_layout_with_config(&layout, &cfg).expect_err("unknown purpose must be rejected");
    assert!(
        err.contains("nonexistent_unicorn_purpose"),
        "error must mention unknown purpose: {err}"
    );
}

#[test]
fn layout_unknown_depth_denied_in_enforce() {
    let _lock = TEST_ENV_LOCK.lock().unwrap();

    let slp = builtin::get("slp").expect("slp builtin");
    let caller = Caller {
        name: "my-lead".to_string(),
        role: Role::Lead,
        depth: CallerDepth::Unknown,
        source: IdentitySource::PaneTokens,
    };

    // Case 1: In test mode (QUAQUA_TEST=1) or shadow mode -> allowed with trace detail
    let prev_shadow = std::env::var("QUAQUA_LAYOUT_SHADOW").ok();
    let prev_test = std::env::var("QUAQUA_TEST").ok();

    std::env::set_var("QUAQUA_TEST", "1");
    std::env::remove_var("QUAQUA_LAYOUT_SHADOW");
    let d_test = check(&slp, &caller, Verb::Spawn, "peer");
    assert!(
        d_test.allow,
        "CallerDepth::Unknown must pass when QUAQUA_TEST=1"
    );

    // Case 2: In shadow mode (QUAQUA_LAYOUT_SHADOW=1) -> allowed
    std::env::remove_var("QUAQUA_TEST");
    std::env::set_var("QUAQUA_LAYOUT_SHADOW", "1");
    let d_shadow = check(&slp, &caller, Verb::Spawn, "peer");
    assert!(
        d_shadow.allow,
        "CallerDepth::Unknown must pass in shadow mode"
    );

    // Case 3: Enforce mode (no shadow, QUAQUA_TEST unset) -> denied with depth.unknown
    std::env::remove_var("QUAQUA_TEST");
    std::env::remove_var("QUAQUA_LAYOUT_SHADOW");
    let d_enforce = check(&slp, &caller, Verb::Spawn, "peer");
    assert!(
        !d_enforce.allow,
        "CallerDepth::Unknown must be denied in enforce mode"
    );
    assert_eq!(d_enforce.rule_id, "depth.unknown");

    // Restore env
    match prev_shadow {
        Some(v) => std::env::set_var("QUAQUA_LAYOUT_SHADOW", v),
        None => std::env::remove_var("QUAQUA_LAYOUT_SHADOW"),
    }
    match prev_test {
        Some(v) => std::env::set_var("QUAQUA_TEST", v),
        None => std::env::remove_var("QUAQUA_TEST"),
    }
}

#[test]
fn layout_all_builtins_pass_validate_layout_with_config() {
    let template_toml = include_str!("../../../templates/workspace/team.toml");
    let cfg: Config = toml::from_str(template_toml).expect("parse workspace team.toml");

    for name in &builtin::BUILTIN_NAMES {
        let layout = builtin::load(name).unwrap_or_else(|e| panic!("failed to load '{name}': {e}"));
        validate_layout_with_config(&layout, &cfg)
            .unwrap_or_else(|e| panic!("builtin '{name}' failed validate_layout_with_config: {e}"));
    }
}

#[test]
fn layout_user_ambiguous_role_denied_without_marker() {
    let ambiguous_toml = r#"
name = "ambiguous-leads"
depth = 2

[roles.cos]
class = "cos"
can_spawn = ["lead-a", "lead-b"]
can_prompt = ["lead-a", "lead-b"]

[roles.lead-a]
class = "lead"
can_spawn = []
reports_to = "cos"

[roles.lead-b]
class = "lead"
can_spawn = []
reports_to = "cos"
"#;
    let layout: Layout = toml::from_str(ambiguous_toml).expect("parse layout");
    let runs_dir = hermetic_temp_dir("ambig-role");

    let caller = Caller {
        name: "some-agent".to_string(),
        role: Role::Lead,
        depth: CallerDepth::Resolved(1),
        source: IdentitySource::PaneTokens,
    };

    // Without runs/<name>.layout_role, Lead class matches both lead-a and lead-b -> role.ambiguous
    let res = resolve_caller_role(&layout, &caller, Some(&runs_dir));
    assert_eq!(res, Err("role.ambiguous".to_string()));

    // With runs/<name>.layout_role recorded -> resolves to lead-a
    record_layout_role(&runs_dir, "some-agent", "lead-a").unwrap();
    let res_ok = resolve_caller_role(&layout, &caller, Some(&runs_dir));
    assert_eq!(res_ok, Ok("lead-a"));
}

#[test]
fn layout_user_validation_spec_constraints() {
    // 1. Reserved name 'owner'
    let owner_role_toml = r#"
name = "bad-owner"
depth = 1
[roles.cos]
class = "cos"
can_spawn = ["owner"]
[roles.owner]
class = "peer"
"#;
    let l_owner: Layout = toml::from_str(owner_role_toml).unwrap();
    assert!(validate_layout(&l_owner)
        .unwrap_err()
        .contains("reserved role name 'owner'"));

    // 2. Multiple cos roles
    let multi_cos_toml = r#"
name = "bad-cos"
depth = 1
[roles.cos]
class = "cos"
can_spawn = []
[roles.cos2]
class = "cos"
can_spawn = []
"#;
    let l_cos: Layout = toml::from_str(multi_cos_toml).unwrap();
    assert!(validate_layout(&l_cos)
        .unwrap_err()
        .contains("multiple cos roles"));

    // 3. User layout shadowing builtin
    let shadow_toml = r#"
name = "council"
depth = 1
[roles.cos]
class = "cos"
can_spawn = []
"#;
    let l_shadow: Layout = toml::from_str(shadow_toml).unwrap();
    let err_shadow = validate_user_layout(&l_shadow).unwrap_err();
    assert!(err_shadow.contains("layout.shadows_builtin"));

    // 4. Council members > 5
    let council_6_toml = r#"
name = "council-6"
depth = 2
[roles.cos]
class = "cos"
can_spawn = ["tf"]
[roles.tf]
class = "taskforce"
reports_to = "owner"
members = [
    { purpose = "review" },
    { purpose = "review" },
    { purpose = "review" },
    { purpose = "review" },
    { purpose = "review" },
    { purpose = "review" },
]
"#;
    let l_c6: Layout = toml::from_str(council_6_toml).unwrap();
    assert!(validate_layout(&l_c6)
        .unwrap_err()
        .contains("member count 6 must be in 1..=5"));

    // 5. Quorum > N
    let quorum_bad_toml = r#"
name = "bad-quorum"
depth = 2
[roles.cos]
class = "cos"
can_spawn = ["tf"]
[roles.tf]
class = "taskforce"
reports_to = "owner"
members = [{ purpose = "review" }, { purpose = "design" }]
quorum = 3
"#;
    let l_qbad: Layout = toml::from_str(quorum_bad_toml).unwrap();
    assert!(validate_layout(&l_qbad)
        .unwrap_err()
        .contains("quorum 3 must be in 1..=2"));
}

#[test]
fn layout_migrate_carries_taskforce_members() {
    let v2_with_tf = r#"
schema_version = 2

[limits]
depth = 2
max_workers_per_orchestrator = 4

[roles.cos]
can_spawn = ["lead"]
can_prompt = ["lead"]

[roles.lead]
can_spawn = ["peer"]
can_prompt = ["peer"]
reports_to = "cos"
max_peers = 3

[roles.peer]
can_spawn = []
can_prompt = []
reports_to = "owner"

[taskforce.advisor]
members = ["suhuynh"]
read_only = true

[taskforce.dual-lens]
members = ["truonglao", "suhuynh"]
read_only = true

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

[kinds.truonglao]
harness = "claude"
model = "opus-5.5"
effort = "high"
can_review = true

[merge]
integ_branch = "quaqua/{slug}-integ"
"#;

    let res = migrate_team_toml(v2_with_tf).expect("migration must succeed");
    let migrated_str = match res {
        MigrateResult::Migrated(s) => s,
        _ => panic!("expected Migrated result"),
    };

    assert!(migrated_str.contains("[layouts.custom.roles.taskforce]"));
    assert!(migrated_str.contains("members = ["));
    assert!(migrated_str.contains("kind = \"suhuynh\""));
}
