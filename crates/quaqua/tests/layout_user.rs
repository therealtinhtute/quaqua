use quaqua::config::Config;
use quaqua::identity::{Caller, CallerDepth, IdentitySource, Role};
use quaqua::layout::builtin;
use quaqua::layout::gate::{
    check, check_with_runs_dir, read_layout_role, record_layout_role, resolve_caller_role,
    resolve_target_role, validate_layout, validate_layout_with_config, validate_user_layout,
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

    // 3. Past-depth: role reachable only past layout.depth
    let past_depth_toml = r#"
name = "bad-past-depth"
depth = 1

[roles.cos]
class = "cos"
can_spawn = ["lead"]
can_prompt = ["lead"]

[roles.lead]
class = "lead"
can_spawn = ["peer"]
reports_to = "cos"

[roles.peer]
class = "peer"
can_spawn = []
reports_to = "owner"
"#;
    let bad_past: Layout = toml::from_str(past_depth_toml).expect("parse past depth layout");
    let err3 = validate_layout(&bad_past).expect_err("past-depth role must be rejected");
    assert!(
        err3.contains("reachable only at depth 2 which exceeds layout depth 1"),
        "error must mention past depth exceeding limit, got: {err3}"
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

    // Case 1: Unknown purpose
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
    let layout1: Layout = toml::from_str(invalid_purpose_toml).expect("parse layout");
    assert!(validate_layout(&layout1).is_ok());

    let err1 =
        validate_layout_with_config(&layout1, &cfg).expect_err("unknown purpose must be rejected");
    assert!(
        err1.contains("nonexistent_unicorn_purpose"),
        "error must mention unknown purpose: {err1}"
    );

    // Case 2: Unknown kind
    let invalid_kind_toml = r#"
name = "bad-kind"
depth = 2

[roles.cos]
class = "cos"
can_spawn = ["taskforce"]
can_prompt = ["taskforce"]

[roles.taskforce]
class = "taskforce"
reports_to = "owner"
members = [
    { kind = "ghost_kind_never_defined" }
]
"#;
    let layout2: Layout = toml::from_str(invalid_kind_toml).expect("parse layout");
    assert!(validate_layout(&layout2).is_ok());

    let err2 =
        validate_layout_with_config(&layout2, &cfg).expect_err("unknown kind must be rejected");
    assert!(
        err2.contains("ghost_kind_never_defined"),
        "error must mention unknown kind: {err2}"
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

    // Case 1: In test mode (QUAQUA_TEST=1) -> allowed with trace detail
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
fn layout_user_upward_roles_denied() {
    // 1. Peer spawns Lead (peer rank 2 cannot spawn lead rank 1)
    let peer_spawns_lead_toml = r#"
name = "bad-upward-spawn"
depth = 2

[roles.cos]
class = "cos"
can_spawn = ["px"]
can_prompt = ["px"]

[roles.px]
class = "peer"
can_spawn = ["lx"]
reports_to = "cos"

[roles.lx]
class = "lead"
can_spawn = []
reports_to = "cos"
"#;
    let l_up1: Layout = toml::from_str(peer_spawns_lead_toml).unwrap();
    let err1 = validate_layout(&l_up1).expect_err("peer spawning lead must be rejected as upward");
    assert!(
        err1.contains("layout.upward"),
        "error must mention layout.upward, got: {err1}"
    );

    // 2. Lead reports to Peer (lead rank 1 cannot report to lower-ranking peer rank 2)
    let lead_reports_peer_toml = r#"
name = "bad-upward-rep"
depth = 2

[roles.cos]
class = "cos"
can_spawn = ["px"]
can_prompt = ["px"]

[roles.px]
class = "peer"
can_spawn = []
reports_to = "cos"

[roles.lx]
class = "lead"
can_spawn = []
reports_to = "px"
"#;
    let l_up2: Layout = toml::from_str(lead_reports_peer_toml).unwrap();
    let err2 = validate_layout(&l_up2).expect_err("lead reporting to peer must be rejected");
    assert!(
        err2.contains("layout.upward"),
        "error must mention layout.upward, got: {err2}"
    );
}

#[test]
fn layout_user_unreachable_roles_denied() {
    // Disconnected role with no can_spawn incoming and not cos
    let unreachable_toml = r#"
name = "bad-unreachable"
depth = 2

[roles.cos]
class = "cos"
can_spawn = ["lead"]
can_prompt = ["lead"]

[roles.lead]
class = "lead"
can_spawn = []
reports_to = "cos"

[roles.orphan]
class = "peer"
can_spawn = []
"#;
    let l_unreach: Layout = toml::from_str(unreachable_toml).unwrap();
    let err = validate_layout(&l_unreach).expect_err("unreachable role must be rejected");
    assert!(
        err.contains("is unreachable from root 'cos'"),
        "error must mention unreachable from root, got: {err}"
    );
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
fn layout_user_target_class_ambiguous_denied() {
    let ambig_target_toml = r#"
name = "ambig-target"
depth = 2

[roles.cos]
class = "cos"
can_spawn = ["tf-a", "tf-b"]
can_prompt = ["tf-a", "tf-b"]

[roles.tf-a]
class = "taskforce"
reports_to = "owner"

[roles.tf-b]
class = "taskforce"
reports_to = "owner"
"#;
    let layout: Layout = toml::from_str(ambig_target_toml).unwrap();
    let res = resolve_target_role(&layout, Some("cos"), "taskforce");
    assert_eq!(res, Err("role.ambiguous".to_string()));
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

    // 3. Cos role with reports_to
    let cos_rep_toml = r#"
name = "bad-cos-rep"
depth = 1
[roles.cos]
class = "cos"
can_spawn = []
reports_to = "owner"
"#;
    let l_crep: Layout = toml::from_str(cos_rep_toml).unwrap();
    assert!(validate_layout(&l_crep)
        .unwrap_err()
        .contains("cos role 'cos' must not have reports_to"));

    // 4. User layout shadowing builtin
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

    // 5. Council members > 5
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

    // 6. Quorum > N
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

    // 7. Quorum = 0
    let quorum_zero_toml = r#"
name = "bad-quorum-zero"
depth = 2
[roles.cos]
class = "cos"
can_spawn = ["tf"]
[roles.tf]
class = "taskforce"
reports_to = "owner"
members = [{ purpose = "review" }]
quorum = 0
"#;
    let l_qzero: Layout = toml::from_str(quorum_zero_toml).unwrap();
    assert!(validate_layout(&l_qzero)
        .unwrap_err()
        .contains("quorum 0 must be in 1..=1"));

    // 8. Member with both purpose and kind
    let both_member_toml = r#"
name = "bad-both-member"
depth = 2
[roles.cos]
class = "cos"
can_spawn = ["tf"]
[roles.tf]
class = "taskforce"
reports_to = "owner"
members = [{ purpose = "review", kind = "suhuynh" }]
"#;
    let l_both: Layout = toml::from_str(both_member_toml).unwrap();
    assert!(validate_layout(&l_both)
        .unwrap_err()
        .contains("purpose and kind cannot both be set"));

    // 9. Member with neither purpose nor kind
    let neither_member_toml = r#"
name = "bad-neither-member"
depth = 2
[roles.cos]
class = "cos"
can_spawn = ["tf"]
[roles.tf]
class = "taskforce"
reports_to = "owner"
members = [{}]
"#;
    let l_neither: Layout = toml::from_str(neither_member_toml).unwrap();
    assert!(validate_layout(&l_neither)
        .unwrap_err()
        .contains("must define purpose or kind"));

    // 10. Members on non-taskforce class
    let members_on_peer_toml = r#"
name = "bad-peer-members"
depth = 2
[roles.cos]
class = "cos"
can_spawn = ["px"]
[roles.px]
class = "peer"
reports_to = "owner"
members = [{ purpose = "review" }]
"#;
    let l_pmem: Layout = toml::from_str(members_on_peer_toml).unwrap();
    assert!(validate_layout(&l_pmem)
        .unwrap_err()
        .contains("members allowed only on taskforce class"));

    // 11. max_children > CAP_CEILING (8)
    let cap_over_toml = r#"
name = "bad-cap-over"
depth = 2
[roles.cos]
class = "cos"
can_spawn = ["lead"]
max_children = 9
[roles.lead]
class = "lead"
reports_to = "cos"
"#;
    let l_cap: Layout = toml::from_str(cap_over_toml).unwrap();
    assert!(validate_layout(&l_cap)
        .unwrap_err()
        .contains("max_children 9 must be in 1..=8"));

    // 12. Naming slug-lead on non-lead class
    let slug_on_peer_toml = r#"
name = "bad-slug-peer"
depth = 2
[roles.cos]
class = "cos"
can_spawn = ["px"]
[roles.px]
class = "peer"
reports_to = "owner"
naming = "slug-lead"
"#;
    let l_slug: Layout = toml::from_str(slug_on_peer_toml).unwrap();
    assert!(validate_layout(&l_slug)
        .unwrap_err()
        .contains("naming 'slug-lead' only valid on lead class"));

    // 13. Invalid role-name regex (uppercase)
    let upper_role_toml = r#"
name = "bad-upper"
depth = 1
[roles.cos]
class = "cos"
can_spawn = ["Lead"]
[roles.Lead]
class = "lead"
reports_to = "cos"
"#;
    let l_upper: Layout = toml::from_str(upper_role_toml).unwrap();
    assert!(validate_layout(&l_upper)
        .unwrap_err()
        .contains("name is invalid"));
}

#[test]
fn layout_effective_quorum_majority_calculation() {
    let make_role = |n: usize, q: Option<usize>| LayoutRole {
        class: RoleClass::Taskforce,
        can_spawn: Vec::new(),
        can_prompt: Vec::new(),
        reports_to: Some("owner".to_string()),
        max_children: None,
        naming: Naming::KindTask,
        members: (0..n)
            .map(|_| Member {
                purpose: Some("review".to_string()),
                kind: None,
            })
            .collect(),
        quorum: q,
    };

    // No members -> None
    assert_eq!(make_role(0, None).effective_quorum(), None);

    // Majority floor(N/2)+1 defaults
    assert_eq!(make_role(1, None).effective_quorum(), Some(1));
    assert_eq!(make_role(2, None).effective_quorum(), Some(2));
    assert_eq!(make_role(3, None).effective_quorum(), Some(2));
    assert_eq!(make_role(4, None).effective_quorum(), Some(3));
    assert_eq!(make_role(5, None).effective_quorum(), Some(3));

    // Custom override
    assert_eq!(make_role(5, Some(4)).effective_quorum(), Some(4));
    assert_eq!(make_role(3, Some(1)).effective_quorum(), Some(1));
}

#[test]
fn layout_migrate_carries_taskforce_members() {
    let v2_with_tf = r#"
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

#[test]
fn layout_migrate_fails_when_members_exceed_five() {
    let v2_excessive_members = r#"
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

[taskforce.big]
members = ["m1", "m2", "m3", "m4", "m5", "m6"]
read_only = true

[strategy]
default_kind = "m1"
default_reviewer = ["m2"]
builder_reviewer_differ = "none"

[kinds.m1]
harness = "omp"
model = "m"
effort = "high"
can_review = true
[kinds.m2]
harness = "omp"
model = "m"
effort = "high"
can_review = true
[kinds.m3]
harness = "omp"
model = "m"
effort = "high"
can_review = true
[kinds.m4]
harness = "omp"
model = "m"
effort = "high"
can_review = true
[kinds.m5]
harness = "omp"
model = "m"
effort = "high"
can_review = true
[kinds.m6]
harness = "omp"
model = "m"
effort = "high"
can_review = true
"#;

    let err = migrate_team_toml(v2_excessive_members)
        .expect_err("migration must fail when carried members exceed 5");
    assert!(
        err.contains("carried member count 6 exceeds maximum allowed of 5"),
        "error must state exceeded member count: {err}"
    );
}

#[test]
fn layout_show_prints_members_and_quorum() {
    let _lock = TEST_ENV_LOCK.lock().unwrap();

    let temp = hermetic_temp_dir("show-members");
    let team_path = temp.join("team.toml");
    let custom_v3 = r#"
schema_version = 3
[layout]
use = "council"

[merge]
integ_branch = "quaqua/{slug}-integ"

[strategy]
default_kind = "sude"
default_reviewer = ["suhuynh"]
builder_reviewer_differ = "none"

[purposes]
build = "sude"
review = "suhuynh"
design = "truonglao"

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
"#;
    std::fs::write(&team_path, custom_v3).unwrap();

    let prev_team = std::env::var("QUAQUA_TEAM_TOML").ok();
    std::env::set_var("QUAQUA_TEAM_TOML", &team_path);

    let show_output = quaqua::layout::run_show(None, false).expect("run_show succeeds");

    match prev_team {
        Some(v) => std::env::set_var("QUAQUA_TEAM_TOML", v),
        None => std::env::remove_var("QUAQUA_TEAM_TOML"),
    }

    assert!(
        show_output.contains("members:      [purpose=review, purpose=review, purpose=design]"),
        "output must list members: {show_output}"
    );
    assert!(
        show_output.contains("quorum:       2"),
        "output must show quorum 2: {show_output}"
    );
}
