use quaqua::cap::count_owner_slots;
use quaqua::config::{self, Config, ProjectOverride};
use quaqua::herdr::HerdrOps;
use quaqua::layout::model::{Layout, LayoutRole, Member, RoleClass};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Child;
use std::process::Output;

struct MockHerdr {
    agent_list_output: String,
}

impl MockHerdr {
    fn new(agent_list_json: &str) -> Self {
        Self {
            agent_list_output: agent_list_json.to_string(),
        }
    }
}

impl HerdrOps for MockHerdr {
    fn run(&self, _args: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_list(&self) -> std::io::Result<String> {
        Ok(self.agent_list_output.clone())
    }
    fn agent_get(&self, _name: &str) -> std::io::Result<(bool, String)> {
        unimplemented!()
    }
    fn agent_read(&self, _name: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        unimplemented!()
    }
    fn agent_start(
        &self,
        _name: &str,
        _kind: &str,
        _pane: &str,
        _timeout_ms: u32,
        _extra_args: &[&str],
    ) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_split(
        &self,
        _cwd: &str,
        _owner: &str,
        _envs: &[(&str, &str)],
    ) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_close(&self, _pane: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_report_metadata(&self, _pane: &str, _tokens: &[&str]) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn pane_wait_output(&self, _pane: &str, _pattern: &str, _timeout_ms: u32) -> bool {
        false
    }
    fn pane_read(&self, _pane: &str, _source: &str, _lines: u32) -> std::io::Result<String> {
        unimplemented!()
    }
    fn pane_send_keys(&self, _pane: &str, _key: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_send_keys(&self, _name: &str, _key: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_wait_idle(&self, _name: &str, _timeout_ms: u32) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_prompt_nowait(&self, _name: &str, _prompt: &str) -> std::io::Result<Output> {
        unimplemented!()
    }
    fn agent_prompt_wait_background(
        &self,
        _name: &str,
        _prompt: &str,
        _timeout_ms: u32,
        _log_file: std::fs::File,
    ) -> std::io::Result<Child> {
        unimplemented!()
    }
}

fn temp_runs_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "quaqua-test-{prefix}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn valid_base_config() -> Config {
    config::embedded_team_config().expect("embedded team.toml")
}

#[test]
fn config_validate_rejects_bad_layout() {
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;

    // Bad layout with self-cycle and invalid upward edge
    let mut bad_roles = BTreeMap::new();
    bad_roles.insert(
        "cos".to_string(),
        LayoutRole {
            class: RoleClass::Cos,
            can_spawn: vec!["lead".to_string()],
            can_prompt: vec![],
            reports_to: None,
            max_children: Some(4),
            naming: Default::default(),
            members: vec![],
            quorum: None,
        },
    );
    bad_roles.insert(
        "lead".to_string(),
        LayoutRole {
            class: RoleClass::Lead,
            can_spawn: vec!["lead".to_string()], // Self-cycle
            can_prompt: vec![],
            reports_to: Some("cos".to_string()),
            max_children: Some(4),
            naming: Default::default(),
            members: vec![],
            quorum: None,
        },
    );

    let bad_layout = Layout {
        name: "bad".to_string(),
        depth: 2,
        roles: bad_roles,
    };

    cfg.layouts.insert("bad".to_string(), bad_layout);

    let err = config::validate(&cfg).unwrap_err();
    assert!(
        err.contains("layout.cycle"),
        "expected cycle error, got: {err}"
    );
}

#[test]
fn config_project_layout_override_resolves() {
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;
    cfg.layout = Some(config::LayoutRefConfig {
        use_layout: "slp".to_string(),
    });

    // Project A overrides layout to "council"
    cfg.projects.insert(
        "proj-a".to_string(),
        ProjectOverride {
            max_workers_per_orchestrator: None,
            layout: Some("council".to_string()),
        },
    );

    // Project B has no layout override
    cfg.projects.insert(
        "proj-b".to_string(),
        ProjectOverride {
            max_workers_per_orchestrator: Some(3),
            layout: None,
        },
    );

    // Resolution project -> global -> legacy
    let layout_a = config::resolve_project_layout(&cfg, Some("proj-a")).expect("proj-a layout");
    assert_eq!(layout_a.name, "council");

    let layout_b = config::resolve_project_layout(&cfg, Some("proj-b")).expect("proj-b layout");
    assert_eq!(layout_b.name, "slp");

    let layout_global = config::resolve_project_layout(&cfg, None).expect("global layout");
    assert_eq!(layout_global.name, "slp");

    let layout_unregistered =
        config::resolve_project_layout(&cfg, Some("proj-unknown")).expect("fallback to global");
    assert_eq!(layout_unregistered.name, "slp");

    // config::resolve must also set cfg.layout per A3
    let resolved_a = config::resolve(cfg.clone(), Some("proj-a")).expect("resolved config A");
    assert_eq!(
        resolved_a.layout.as_ref().map(|l| l.use_layout.as_str()),
        Some("council")
    );
}

#[test]
fn cap_council_counts_n_slots() {
    let tmp = temp_runs_dir("council-cap");
    let cfg = valid_base_config();

    // Active council state file
    let state_file = tmp.join("council-mycouncil.state");
    std::fs::write(
        &state_file,
        format!(
            "task=mycouncil\nkind=council\nowner=cos\nfinished=0\ntaskforce_pid={}\nn=3\n",
            std::process::id()
        ),
    )
    .unwrap();

    let mock_no_live = MockHerdr::new(r#"{"result":{"agents":[]}}"#);
    // Council with n=3 reserves 3 slots even with 0 live workers
    assert_eq!(
        count_owner_slots(&tmp, "cos", None, &mock_no_live, &cfg),
        3,
        "active council must reserve n=3 slots before workers are live"
    );

    // Create 3 council member workers with .taskforce markers
    let members = ["sude-mycouncilc1", "sude-mycouncilc2", "sude-mycouncilc3"];
    for name in &members {
        std::fs::write(tmp.join(format!("{name}.owner")), "cos\n").unwrap();
        quaqua::state::WorkerMetadata::save_taskforce(&tmp, name, "mycouncil").unwrap();
    }

    // 1 live worker: still accounts for 3 slots (max(1, weight=3))
    let mock_1_live = MockHerdr::new(
        r#"{"result":{"agents":[{"name":"sude-mycouncilc1","agent_status":"working"}]}}"#,
    );
    assert_eq!(
        count_owner_slots(&tmp, "cos", None, &mock_1_live, &cfg),
        3,
        "1 live member of council must still account for 3 slots"
    );

    // All 3 live workers: accounts for 3 slots (max(3, weight=3)), never 6
    let agents_json = format!(
        r#"{{"result":{{"agents":[
            {{"name":"{}","agent_status":"working"}},
            {{"name":"{}","agent_status":"working"}},
            {{"name":"{}","agent_status":"working"}}
        ]}}}}"#,
        members[0], members[1], members[2]
    );
    let mock_all_live = MockHerdr::new(&agents_json);
    assert_eq!(
        count_owner_slots(&tmp, "cos", None, &mock_all_live, &cfg),
        3,
        "3 live members of council must account for exactly 3 slots"
    );

    // Finished council reserves 0 slots
    std::fs::write(
        &state_file,
        format!(
            "task=mycouncil\nkind=council\nowner=cos\nfinished=1\ntaskforce_pid={}\nn=3\n",
            std::process::id()
        ),
    )
    .unwrap();
    assert_eq!(
        count_owner_slots(&tmp, "cos", None, &mock_no_live, &cfg),
        0,
        "finished council reserves 0 slots"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn cap_council5_rejected_under_default_lead() {
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;

    // Define a 5-member council layout spawnable by lead
    let mut roles = BTreeMap::new();
    roles.insert(
        "cos".to_string(),
        LayoutRole {
            class: RoleClass::Cos,
            can_spawn: vec!["lead".to_string()],
            can_prompt: vec![],
            reports_to: None,
            max_children: Some(8),
            naming: Default::default(),
            members: vec![],
            quorum: None,
        },
    );
    roles.insert(
        "lead".to_string(),
        LayoutRole {
            class: RoleClass::Lead,
            can_spawn: vec!["council".to_string()],
            can_prompt: vec![],
            reports_to: Some("cos".to_string()),
            max_children: Some(4),
            naming: Default::default(),
            members: vec![],
            quorum: None,
        },
    );
    roles.insert(
        "council".to_string(),
        LayoutRole {
            class: RoleClass::Taskforce,
            can_spawn: vec![],
            can_prompt: vec![],
            reports_to: Some("lead".to_string()),
            max_children: None,
            naming: Default::default(),
            members: vec![
                Member {
                    purpose: Some("review".to_string()),
                    kind: None,
                },
                Member {
                    purpose: Some("review".to_string()),
                    kind: None,
                },
                Member {
                    purpose: Some("review".to_string()),
                    kind: None,
                },
                Member {
                    purpose: Some("review".to_string()),
                    kind: None,
                },
                Member {
                    purpose: Some("design".to_string()),
                    kind: None,
                },
            ],
            quorum: Some(3),
        },
    );

    let council5_layout = Layout {
        name: "council5".to_string(),
        depth: 2,
        roles,
    };
    cfg.layouts.insert("council5".to_string(), council5_layout);

    // Default Lead max_peers is 4 -> N=5 exceeds 4!
    let err = config::validate(&cfg).unwrap_err();
    assert!(
        err.contains("council5")
            && err.contains("council")
            && err.contains("N=5")
            && err.contains("lead")
            && err.contains("4")
            && err.contains("[roles.lead] max_peers"),
        "error must name layout, role, N=5, spawner lead, cap 4, and knob: {err}"
    );

    // Raising lead max_peers to 5 (with global cap 8) allows it to pass
    cfg.limits.max_workers_per_orchestrator = 8;
    cfg.roles.get_mut("lead").unwrap().max_peers = Some(5);
    if let Err(e) = config::validate(&cfg) {
        panic!("unexpected validate error: {e}");
    }
}

#[test]
fn config_validate_rejects_project_unknown_layout() {
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;

    cfg.projects.insert(
        "bad-proj".to_string(),
        ProjectOverride {
            max_workers_per_orchestrator: None,
            layout: Some("nonexistent_layout".to_string()),
        },
    );

    let err = config::validate(&cfg).unwrap_err();
    assert!(
        err.contains("bad-proj") && err.contains("nonexistent_layout"),
        "error must name project and unknown layout: {err}"
    );
}
#[test]
fn f1_schema2_project_resolve_preserves_schema2_and_does_not_invent_layout() {
    let mut cfg = valid_base_config();
    assert_eq!(cfg.schema_version, 2);
    assert!(cfg.layout.is_none());

    cfg.limits.max_workers_per_orchestrator = 5;
    cfg.roles.get_mut("lead").unwrap().max_peers = Some(4);
    cfg.projects.insert(
        "zimage".to_string(),
        ProjectOverride {
            max_workers_per_orchestrator: Some(5),
            layout: None,
        },
    );

    // resolve must NOT invent cfg.layout
    let resolved = config::resolve(cfg.clone(), Some("zimage")).expect("resolve project");
    assert!(
        resolved.layout.is_none(),
        "resolve must never invent cfg.layout for schema 2"
    );
    assert_eq!(resolved.limits.max_workers_per_orchestrator, 5);

    // validate on resolved config must pass without triggering schema 2 [layout] error
    assert!(config::validate(&resolved).is_ok());

    // resolve_project_layout must succeed and return synthesized legacy layout
    let layout = config::resolve_project_layout(&cfg, Some("zimage")).expect("project layout");
    assert_eq!(layout.roles["cos"].max_children, Some(5));
}

#[test]
fn f2_v3_legacy_resolve_preserves_edges_and_does_not_widen_can_spawn() {
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;
    cfg.layout = None; // v3 legacy config: no [layout] table

    // Restrict cos can_spawn to only ["lead"]
    cfg.roles.get_mut("cos").unwrap().can_spawn = vec!["lead".to_string()];

    cfg.projects.insert(
        "proj9".to_string(),
        ProjectOverride {
            max_workers_per_orchestrator: Some(8),
            layout: None,
        },
    );

    // resolve must NOT invent cfg.layout = Some("slp")
    let resolved = config::resolve(cfg.clone(), Some("proj9")).expect("resolve proj9");
    assert!(
        resolved.layout.is_none(),
        "resolve must never invent cfg.layout for v3 legacy configs"
    );

    // Both without and with project must return the synthesized layout with restricted edges
    let layout_no_proj = config::resolve_project_layout(&cfg, None).expect("layout without proj");
    assert_eq!(
        layout_no_proj.roles["cos"].can_spawn,
        vec!["lead".to_string()]
    );

    let layout_proj =
        config::resolve_project_layout(&cfg, Some("proj9")).expect("layout with proj");
    assert_eq!(
        layout_proj.roles["cos"].can_spawn,
        vec!["lead".to_string()],
        "can_spawn edges must not be widened to builtin slp"
    );
}

#[test]
fn f3_config_validate_rejects_weight_override_exceeding_spawner_cap() {
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;
    cfg.layout = Some(config::LayoutRefConfig {
        use_layout: "council".to_string(),
    });
    // Council has 3 members in built-in layout, but weight override is 5
    cfg.limits.weights.insert("council".to_string(), 5);

    // Lead spawner cap is 4, but weight is 5 -> static check must catch it!
    let err = config::validate(&cfg).unwrap_err();
    assert!(
        err.contains("N=5")
            && err.contains("council")
            && err.contains("lead")
            && err.contains("4")
            && err.contains("[roles.lead] max_peers"),
        "error must reflect weight override N=5 vs lead cap 4: {err}"
    );
}

#[test]
fn f4_config_validate_rejects_project_cap_only_override_exceeding_effective_layout() {
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;
    cfg.layout = Some(config::LayoutRefConfig {
        use_layout: "council".to_string(),
    });

    // Project small only overrides cap to 2 (effective council weight is 3)
    cfg.projects.insert(
        "small".to_string(),
        ProjectOverride {
            max_workers_per_orchestrator: Some(2),
            layout: None,
        },
    );

    let err = config::validate(&cfg).unwrap_err();
    assert!(
        err.contains("council")
            && err.contains("N=3")
            && err.contains("2")
            && err.contains("[limits] max_workers_per_orchestrator"),
        "error must reject project cap-only override 2 against inherited council weight 3: {err}"
    );
}

#[test]
fn f5_cap_council_counts_n_slots_with_state_members_without_markers() {
    let tmp = temp_runs_dir("council-state-members");
    let cfg = valid_base_config();

    // Active council state with members= list and no .taskforce marker files written
    let state_file = tmp.join("council-mycouncil.state");
    std::fs::write(
        &state_file,
        format!(
            "task=mycouncil\nkind=council\nowner=cos\nfinished=0\ntaskforce_pid={}\nmembers=sude-c1,sude-c2,sude-c3\n",
            std::process::id()
        ),
    )
    .unwrap();

    let members = ["sude-c1", "sude-c2", "sude-c3"];
    for name in &members {
        std::fs::write(tmp.join(format!("{name}.owner")), "cos\n").unwrap();
        // Explicitly NOT writing .taskforce marker files
    }

    let agents_json = format!(
        r#"{{"result":{{"agents":[
            {{"name":"{}","agent_status":"working"}},
            {{"name":"{}","agent_status":"working"}},
            {{"name":"{}","agent_status":"working"}}
        ]}}}}"#,
        members[0], members[1], members[2]
    );
    let mock_all_live = MockHerdr::new(&agents_json);

    // Must count N=3 slots via state.members attribution, not 2N=6 slots
    assert_eq!(
        count_owner_slots(&tmp, "cos", None, &mock_all_live, &cfg),
        3,
        "council with state members and no markers must count N=3 slots, never 2N"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn f6_config_validate_allows_peer_spawner_without_config_knob() {
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;

    let mut roles = BTreeMap::new();
    roles.insert(
        "cos".to_string(),
        LayoutRole {
            class: RoleClass::Cos,
            can_spawn: vec!["lead".to_string()],
            can_prompt: vec![],
            reports_to: None,
            max_children: Some(8),
            naming: Default::default(),
            members: vec![],
            quorum: None,
        },
    );
    roles.insert(
        "lead".to_string(),
        LayoutRole {
            class: RoleClass::Lead,
            can_spawn: vec!["peer".to_string()],
            can_prompt: vec![],
            reports_to: Some("cos".to_string()),
            max_children: Some(4),
            naming: Default::default(),
            members: vec![],
            quorum: None,
        },
    );
    roles.insert(
        "peer".to_string(),
        LayoutRole {
            class: RoleClass::Peer,
            can_spawn: vec!["taskforce".to_string()],
            can_prompt: vec![],
            reports_to: Some("lead".to_string()),
            max_children: None,
            naming: Default::default(),
            members: vec![],
            quorum: None,
        },
    );
    roles.insert(
        "taskforce".to_string(),
        LayoutRole {
            class: RoleClass::Taskforce,
            can_spawn: vec![],
            can_prompt: vec![],
            reports_to: Some("peer".to_string()),
            max_children: None,
            naming: Default::default(),
            members: vec![
                Member {
                    purpose: Some("review".to_string()),
                    kind: None,
                },
                Member {
                    purpose: Some("review".to_string()),
                    kind: None,
                },
            ],
            quorum: Some(2),
        },
    );

    let peerfork_layout = Layout {
        name: "peerfork".to_string(),
        depth: 3,
        roles,
    };
    cfg.layouts.insert("peerfork".to_string(), peerfork_layout);

    // Legal peer->taskforce spawn must be allowed without knob "unsupported" false reject
    assert!(config::validate(&cfg).is_ok());

    // If peer max_children is set below member count (1 < 2), it fails naming max_children
    cfg.layouts
        .get_mut("peerfork")
        .unwrap()
        .roles
        .get_mut("peer")
        .unwrap()
        .max_children = Some(1);
    let err = config::validate(&cfg).unwrap_err();
    assert!(
        err.contains("effective cap 1") && err.contains("layout role max_children"),
        "error must name layout max_children: {err}"
    );
}

#[test]
fn a4_taskforce_weight_precedence_all_tiers() {
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;

    let mut roles = BTreeMap::new();
    roles.insert(
        "council".to_string(),
        LayoutRole {
            class: RoleClass::Taskforce,
            can_spawn: vec![],
            can_prompt: vec![],
            reports_to: None,
            max_children: None,
            naming: Default::default(),
            members: vec![
                Member {
                    purpose: Some("review".to_string()),
                    kind: None,
                },
                Member {
                    purpose: Some("review".to_string()),
                    kind: None,
                },
                Member {
                    purpose: Some("review".to_string()),
                    kind: None,
                },
                Member {
                    purpose: Some("design".to_string()),
                    kind: None,
                },
            ],
            quorum: Some(3),
        },
    );
    let test_layout = Layout {
        name: "testlayout".to_string(),
        depth: 1,
        roles,
    };

    let empty_layout = Layout {
        name: "empty".to_string(),
        depth: 1,
        roles: BTreeMap::new(),
    };

    // Tier 5: Legacy default (dual-lens: 2, council: 3, other: 1)
    let s_dual = quaqua::state::TaskforceExecutionState {
        kind: "dual-lens".to_string(),
        ..Default::default()
    };
    assert_eq!(
        quaqua::cap::taskforce_weight(&cfg, &s_dual, &empty_layout),
        2
    );

    let s_council = quaqua::state::TaskforceExecutionState {
        kind: "council".to_string(),
        ..Default::default()
    };
    assert_eq!(
        quaqua::cap::taskforce_weight(&cfg, &s_council, &empty_layout),
        3
    );

    let s_other = quaqua::state::TaskforceExecutionState {
        kind: "advisor".to_string(),
        ..Default::default()
    };
    assert_eq!(
        quaqua::cap::taskforce_weight(&cfg, &s_other, &empty_layout),
        1
    );

    // Tier 4: Layout role members count (test_layout has 4 members for "council")
    assert_eq!(
        quaqua::cap::taskforce_weight(&cfg, &s_council, &test_layout),
        4
    );

    // Tier 3: State members count overrides layout members count
    let s_members = quaqua::state::TaskforceExecutionState {
        kind: "council".to_string(),
        members: vec!["m1".to_string(), "m2".to_string()],
        ..Default::default()
    };
    assert_eq!(
        quaqua::cap::taskforce_weight(&cfg, &s_members, &test_layout),
        2
    );

    // Tier 2: State n overrides state members count and layout members
    let s_n = quaqua::state::TaskforceExecutionState {
        kind: "council".to_string(),
        members: vec!["m1".to_string(), "m2".to_string()],
        n: Some(5),
        ..Default::default()
    };
    assert_eq!(quaqua::cap::taskforce_weight(&cfg, &s_n, &test_layout), 5);

    // Tier 1: Config weights.<role> overrides all state and layout fields
    cfg.limits.weights.insert("council".to_string(), 7);
    assert_eq!(quaqua::cap::taskforce_weight(&cfg, &s_n, &test_layout), 7);
}
#[test]
fn u3_1_project_write_back_stores_key_not_name() {
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;

    let mut roles = BTreeMap::new();
    roles.insert(
        "cos".to_string(),
        LayoutRole {
            class: RoleClass::Cos,
            can_spawn: vec!["lead".to_string()],
            can_prompt: vec![],
            reports_to: None,
            max_children: Some(8),
            naming: Default::default(),
            members: vec![],
            quorum: None,
        },
    );
    roles.insert(
        "lead".to_string(),
        LayoutRole {
            class: RoleClass::Lead,
            can_spawn: vec!["peer".to_string()],
            can_prompt: vec![],
            reports_to: Some("cos".to_string()),
            max_children: Some(4),
            naming: Default::default(),
            members: vec![],
            quorum: None,
        },
    );
    roles.insert(
        "peer".to_string(),
        LayoutRole {
            class: RoleClass::Peer,
            can_spawn: vec![],
            can_prompt: vec![],
            reports_to: Some("owner".to_string()),
            max_children: None,
            naming: Default::default(),
            members: vec![],
            quorum: None,
        },
    );

    // Layout key is "mine", but inner name is "renamed"
    let custom_layout = Layout {
        name: "renamed".to_string(),
        depth: 2,
        roles,
    };
    cfg.layouts.insert("mine".to_string(), custom_layout);

    cfg.projects.insert(
        "projM".to_string(),
        ProjectOverride {
            max_workers_per_orchestrator: None,
            layout: Some("mine".to_string()),
        },
    );

    // validate accepts it
    assert!(config::validate(&cfg).is_ok());

    // resolve must write back key "mine", NOT inner name "renamed"
    let resolved = config::resolve(cfg.clone(), Some("projM")).expect("resolve projM");
    assert_eq!(
        resolved.layout.as_ref().map(|l| l.use_layout.as_str()),
        Some("mine"),
        "write-back must store the layout key 'mine', not inner name 'renamed'"
    );

    // And subsequent resolution on resolved config succeeds finding "mine" in cfg.layouts
    let layout = config::resolve_project_layout(&resolved, None).expect("resolved layout");
    assert_eq!(layout.name, "renamed");
}

#[test]
fn u3_2_count_owner_slots_warns_on_unresolvable_layout() {
    let tmp = temp_runs_dir("unresolvable-layout");
    let mut cfg = valid_base_config();
    cfg.schema_version = 3;
    // Set layout to non-existent layout to trigger resolve_project_layout failure
    cfg.layout = Some(config::LayoutRefConfig {
        use_layout: "ghost_layout".to_string(),
    });

    let mock = MockHerdr::new(r#"{"result":{"agents":[]}}"#);
    // count_owner_slots should warn and fallback to legacy without panicking
    let slots = count_owner_slots(&tmp, "cos", None, &mock, &cfg);
    assert_eq!(slots, 0);

    let _ = std::fs::remove_dir_all(&tmp);
}
