use crate::config::Config;
use crate::herdr::HerdrOps;
use crate::identity::{Caller, CallerDepth, Role};
use crate::layout::model::{Decision, Layout, TraceStep, Verb};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// Origin of a worker spawn invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpawnOrigin {
    Cli,
    Pipeline,
    Activity,
    TaskforceMember,
}

impl fmt::Display for SpawnOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpawnOrigin::Cli => write!(f, "cli"),
            SpawnOrigin::Pipeline => write!(f, "pipeline"),
            SpawnOrigin::Activity => write!(f, "activity"),
            SpawnOrigin::TaskforceMember => write!(f, "taskforce-member"),
        }
    }
}

/// Resolve the role name for a target string within a layout.
///
/// If target matches a defined role in the layout, returns that role.
/// If target is `cos` and `cos` is defined in the layout, returns `Some("cos")`.
/// If target ends with `-lead` and `lead` is defined in the layout, returns `Some("lead")`.
/// Otherwise returns `None` (unknown target). Never falls back to a default role.
pub fn resolve_target_role<'a>(layout: &'a Layout, target: &'a str) -> Option<&'a str> {
    let target_trim = target.trim();
    if layout.roles.contains_key(target_trim) {
        return Some(target_trim);
    }
    if target_trim == "cos" && layout.roles.contains_key("cos") {
        return Some("cos");
    }
    if target_trim.ends_with("-lead") && layout.roles.contains_key("lead") {
        return Some("lead");
    }
    None
}

/// Validate a layout's internal consistency:
/// - depth >= 1
/// - at least one role defined
/// - all can_spawn references exist in layout.roles
/// - all can_prompt references exist in layout.roles
/// - all reports_to references exist in layout.roles or equal "owner"
/// - any max_children >= 1
pub fn validate_layout(layout: &Layout) -> Result<(), String> {
    if layout.depth < 1 {
        return Err(format!("layout '{}' depth must be at least 1", layout.name));
    }
    if layout.roles.is_empty() {
        return Err(format!(
            "layout '{}' must define at least one role",
            layout.name
        ));
    }

    for (role_name, role) in &layout.roles {
        for target in &role.can_spawn {
            if !layout.roles.contains_key(target) {
                return Err(format!(
                    "layout '{}' role '{role_name}' can_spawn references unknown role '{target}'",
                    layout.name
                ));
            }
        }
        for target in &role.can_prompt {
            if !layout.roles.contains_key(target) {
                return Err(format!(
                    "layout '{}' role '{role_name}' can_prompt references unknown role '{target}'",
                    layout.name
                ));
            }
        }
        if let Some(rep) = &role.reports_to {
            if rep != "owner" && !layout.roles.contains_key(rep) {
                return Err(format!(
                    "layout '{}' role '{role_name}' reports_to references unknown role '{rep}'",
                    layout.name
                ));
            }
        }
        if let Some(cap) = role.max_children {
            if cap < 1 {
                return Err(format!(
                    "layout '{}' role '{role_name}' max_children {cap} must be >= 1",
                    layout.name
                ));
            }
        }
    }

    Ok(())
}

/// Pure layout gate check: layers identity -> role -> edge -> depth -> cap.
pub fn check(layout: &Layout, caller: &Caller, verb: Verb, target: &str) -> Decision {
    check_with_count(layout, caller, verb, target, None)
}

/// Pure layout gate check with optional current child count passed in.
pub fn check_with_count(
    layout: &Layout,
    caller: &Caller,
    verb: Verb,
    target: &str,
    current_children: Option<i64>,
) -> Decision {
    check_with_origin(layout, caller, verb, target, current_children, false)
}

/// Pure layout gate check with optional child count and edge-skip flag.
pub fn check_with_origin(
    layout: &Layout,
    caller: &Caller,
    verb: Verb,
    target: &str,
    current_children: Option<i64>,
    skip_edge: bool,
) -> Decision {
    let mut trace = Vec::new();

    // 1. Identity layer
    if caller.role == Role::Unknown {
        trace.push(TraceStep {
            layer: "identity".to_string(),
            passed: false,
            detail: format!("caller '{}' identity is unknown", caller.name),
        });
        return Decision::deny("identity.unknown", trace);
    }
    trace.push(TraceStep {
        layer: "identity".to_string(),
        passed: true,
        detail: format!(
            "caller '{}' identified as {} (depth {})",
            caller.name, caller.role, caller.depth
        ),
    });

    // 2. Role layer (caller role and target role must exist in layout)
    let caller_role_name = caller.role.as_str();
    let role = match layout.roles.get(caller_role_name) {
        Some(r) => {
            trace.push(TraceStep {
                layer: "role".to_string(),
                passed: true,
                detail: format!(
                    "role '{}' is defined in layout '{}'",
                    caller_role_name, layout.name
                ),
            });
            r
        }
        None => {
            trace.push(TraceStep {
                layer: "role".to_string(),
                passed: false,
                detail: format!(
                    "role '{}' is not defined in layout '{}'",
                    caller_role_name, layout.name
                ),
            });
            return Decision::deny("role.unknown", trace);
        }
    };

    let target_role = match resolve_target_role(layout, target) {
        Some(r) => r,
        None => {
            trace.push(TraceStep {
                layer: "role".to_string(),
                passed: false,
                detail: format!(
                    "target role '{target}' is not defined in layout '{}'",
                    layout.name
                ),
            });
            return Decision::deny("role.unknown", trace);
        }
    };

    // 3. Edge layer
    let mut reports_to_edge = false;
    let edge_allowed = if skip_edge {
        trace.push(TraceStep {
            layer: "edge".to_string(),
            passed: true,
            detail: "edge check skipped".to_string(),
        });
        true
    } else {
        match verb {
            Verb::Spawn => role.can_spawn.iter().any(|r| r == target_role),
            Verb::Prompt => role.can_prompt.iter().any(|r| r == target_role),
            Verb::Mail => {
                if role.can_prompt.iter().any(|r| r == target_role) {
                    true
                } else if role.reports_to.as_deref() == Some(target_role) {
                    reports_to_edge = true;
                    true
                } else if role.reports_to.as_deref() == Some("owner") && target_role == "cos" {
                    reports_to_edge = true;
                    true
                } else {
                    false
                }
            }
        }
    };

    if !edge_allowed {
        let detail = match verb {
            Verb::Spawn => format!(
                "role '{}' cannot spawn role '{}' (can_spawn = {:?})",
                caller_role_name, target_role, role.can_spawn
            ),
            Verb::Prompt => format!(
                "role '{}' cannot prompt role '{}' (can_prompt = {:?})",
                caller_role_name, target_role, role.can_prompt
            ),
            Verb::Mail => format!(
                "role '{}' cannot send mail to role '{}'",
                caller_role_name, target_role
            ),
        };
        trace.push(TraceStep {
            layer: "edge".to_string(),
            passed: false,
            detail,
        });
        return Decision::deny("edge.none", trace);
    }
    if !skip_edge {
        trace.push(TraceStep {
            layer: "edge".to_string(),
            passed: true,
            detail: format!(
                "{} edge permits '{}' -> '{}'",
                verb, caller_role_name, target_role
            ),
        });
    }

    // 4. Depth layer
    if verb == Verb::Spawn {
        match &caller.depth {
            CallerDepth::Resolved(d) => {
                let target_depth = d + 1;
                if target_depth > layout.depth {
                    trace.push(TraceStep {
                        layer: "depth".to_string(),
                        passed: false,
                        detail: format!(
                            "spawn target depth {} would exceed layout limit {}",
                            target_depth, layout.depth
                        ),
                    });
                    return Decision::deny("depth.exceeded", trace);
                }
                trace.push(TraceStep {
                    layer: "depth".to_string(),
                    passed: true,
                    detail: format!(
                        "target depth {} <= layout limit {}",
                        target_depth, layout.depth
                    ),
                });
            }
            CallerDepth::Cycle(cycle) => {
                trace.push(TraceStep {
                    layer: "depth".to_string(),
                    passed: false,
                    detail: format!("cycle in owner chain: {:?}", cycle),
                });
                return Decision::deny("depth.cycle", trace);
            }
            CallerDepth::MissingOwner(owner) => {
                trace.push(TraceStep {
                    layer: "depth".to_string(),
                    passed: false,
                    detail: format!("missing owner in chain: {}", owner),
                });
                return Decision::deny("depth.owner_missing", trace);
            }
            CallerDepth::Unknown => {
                trace.push(TraceStep {
                    layer: "depth".to_string(),
                    passed: true,
                    detail: "caller depth unknown (allowed in shadow/test)".to_string(),
                });
            }
        }
    } else {
        trace.push(TraceStep {
            layer: "depth".to_string(),
            passed: true,
            detail: format!("verb '{}' does not create child roles", verb),
        });
    }

    // 5. Cap layer
    if let Some(count) = current_children {
        if verb == Verb::Spawn {
            if let Some(max) = role.max_children {
                if count >= max {
                    let source =
                        if caller_role_name == "lead" || crate::cap::is_lead_owner(&caller.name) {
                            "roles.lead.max_peers"
                        } else {
                            "limits.max_workers_per_orchestrator"
                        };
                    trace.push(TraceStep {
                        layer: "cap".to_string(),
                        passed: false,
                        detail: format!(
                            "worker cap exceeded ({count} live worker slots for owner '{}'; this spawn needs 1, {source} is {max})",
                            caller.name
                        ),
                    });
                    return Decision::deny("cap.exceeded", trace);
                }
                trace.push(TraceStep {
                    layer: "cap".to_string(),
                    passed: true,
                    detail: format!("role '{}' count {} < cap {}", caller_role_name, count, max),
                });
            } else {
                trace.push(TraceStep {
                    layer: "cap".to_string(),
                    passed: true,
                    detail: format!("role '{}' has no child cap", caller_role_name),
                });
            }
        } else {
            trace.push(TraceStep {
                layer: "cap".to_string(),
                passed: true,
                detail: format!("verb '{}' does not consume child slots", verb),
            });
        }
    } else {
        trace.push(TraceStep {
            layer: "cap".to_string(),
            passed: true,
            detail: "child count not passed (skipped)".to_string(),
        });
    }

    let rule_id = if reports_to_edge {
        format!(
            "{}.{}.reports_to.{}",
            layout.name, caller_role_name, target_role
        )
    } else {
        format!(
            "{}.{}.can_{}.{}",
            layout.name,
            caller_role_name,
            verb.as_str(),
            target_role
        )
    };

    Decision::allow(rule_id, trace)
}

/// Returns true if layout gate is running in shadow mode.
///
/// In P4, the default is enforce mode (false). Setting `QUAQUA_LAYOUT_SHADOW=1`
/// (or `"true"`) provides an escape hatch to restore shadow mode without blocking.
pub fn is_shadow_mode() -> bool {
    std::env::var("QUAQUA_LAYOUT_SHADOW")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Choke-point evaluation of layout rules in enforce mode (default) or shadow mode.
///
/// Under enforce mode: returns `Ok(decision)` if allowed; if denied, records a `denied`
/// FarmEvent in `runs_dir` and returns `Err((1, format!("error: [{}] {}\n", decision.rule_id, detail)))`.
/// Under shadow mode (`QUAQUA_LAYOUT_SHADOW=1`): prints `would_deny` to stderr and records
/// a `would_deny` FarmEvent, but returns `Ok(decision)` without blocking.
pub fn gate<H: HerdrOps>(
    caller: Option<&Caller>,
    herdr: Option<&H>,
    verb: Verb,
    target: &str,
    target_name: Option<&str>,
    origin: SpawnOrigin,
    owner: Option<&str>,
    project: Option<&str>,
    runs_dir: &Path,
    cfg: Option<&Config>,
) -> Result<Decision, (i32, String)> {
    gate_inner(
        caller,
        herdr,
        verb,
        target,
        target_name,
        origin,
        owner,
        project,
        runs_dir,
        cfg,
        false,
    )
}

/// Shadow mode evaluation of layout rules at a choke point.
///
/// Evaluates the decision, prints one `would_deny` stderr line and writes a
/// `farm_events` row if denied. Never blocks, never changes exit codes.
pub fn shadow<H: HerdrOps>(
    caller: Option<&Caller>,
    herdr: Option<&H>,
    verb: Verb,
    target: &str,
    target_name: Option<&str>,
    origin: SpawnOrigin,
    owner: Option<&str>,
    project: Option<&str>,
    runs_dir: &Path,
    cfg: Option<&Config>,
) -> Decision {
    gate_inner(
        caller,
        herdr,
        verb,
        target,
        target_name,
        origin,
        owner,
        project,
        runs_dir,
        cfg,
        true,
    )
    .unwrap_or_else(|_| Decision::deny("shadow.skipped.error", Vec::new()))
}

fn gate_inner<H: HerdrOps>(
    caller: Option<&Caller>,
    herdr: Option<&H>,
    verb: Verb,
    target: &str,
    target_name: Option<&str>,
    origin: SpawnOrigin,
    owner: Option<&str>,
    project: Option<&str>,
    runs_dir: &Path,
    cfg: Option<&Config>,
    force_shadow: bool,
) -> Result<Decision, (i32, String)> {
    let caller_resolved;
    let effective_caller = match caller {
        Some(c) => c,
        None => {
            caller_resolved = match herdr {
                Some(h) => {
                    let mut c = crate::identity::resolve_caller(h, runs_dir);
                    if c.role == crate::identity::Role::Unknown
                        && (std::env::var("QUAQUA_TEST").as_deref() == Ok("1") || cfg!(test))
                        && crate::identity::caller_pane_tokens(h).is_none()
                        && std::env::var("QUAQUA_OWNER").as_deref() != Ok("")
                    {
                        c = crate::identity::Caller {
                            name: owner.unwrap_or("cos").to_string(),
                            role: if owner.map(|o| crate::cap::is_lead_owner(o)).unwrap_or(false) {
                                crate::identity::Role::Lead
                            } else {
                                crate::identity::Role::Cos
                            },
                            depth: crate::identity::CallerDepth::Resolved(
                                if owner.map(|o| crate::cap::is_lead_owner(o)).unwrap_or(false) {
                                    1
                                } else {
                                    0
                                },
                            ),
                            source: crate::identity::IdentitySource::EnvFallback,
                        };
                    }
                    c
                }
                None => crate::identity::Caller {
                    name: "unknown".to_string(),
                    role: crate::identity::Role::Unknown,
                    depth: crate::identity::CallerDepth::Unknown,
                    source: crate::identity::IdentitySource::Unknown,
                },
            };
            &caller_resolved
        }
    };

    let loaded_cfg;
    let effective_cfg = match cfg {
        Some(c) => c,
        None => {
            match crate::config::load_team_config() {
                Ok(c) => {
                    loaded_cfg = c;
                    &loaded_cfg
                }
                Err((_, e)) => {
                    eprintln!("warning: [layout gate] cannot resolve layout: failed to load team config: {e}");
                    return Ok(Decision::allow("gate.skipped.layout_err", Vec::new()));
                }
            }
        }
    };

    let layout = match crate::layout::resolve_effective_layout(
        &effective_caller.name,
        runs_dir,
        effective_cfg,
    ) {
        Ok((l, _)) => l,
        Err(e) => {
            eprintln!("warning: [layout gate] effective layout failed, falling back to live: {e}");
            match crate::layout::resolve_layout(effective_cfg) {
                Ok(l) => l,
                Err(live_err) => {
                    eprintln!("warning: [layout gate] cannot resolve layout: {live_err}");
                    return Ok(Decision::allow("gate.skipped.layout_err", Vec::new()));
                }
            }
        }
    };

    let mut skip_edge = origin == SpawnOrigin::TaskforceMember;
    if !skip_edge && verb == Verb::Prompt {
        if let Some(tname) = target_name {
            let owner_file = runs_dir.join(format!("{tname}.owner"));
            if let Ok(target_owner) = std::fs::read_to_string(&owner_file) {
                if target_owner.trim() == effective_caller.name {
                    skip_edge = true;
                }
            }
        }
    }
    let current_children = if verb == Verb::Spawn
        && origin != SpawnOrigin::Pipeline
        && std::env::var("PIPELINE_TASK").is_err()
    {
        if let Some(h) = herdr {
            let owner_to_count = owner.unwrap_or(effective_caller.name.as_str());
            Some(crate::cap::count_owner_slots(
                runs_dir,
                owner_to_count,
                target_name,
                h,
                effective_cfg,
            ))
        } else {
            None
        }
    } else {
        None
    };

    let decision = check_with_origin(
        &layout,
        effective_caller,
        verb,
        target,
        current_children,
        skip_edge,
    );

    if !decision.allow {
        let failing_step = decision.trace.iter().find(|t| !t.passed);
        let layer = failing_step.map(|t| t.layer.as_str()).unwrap_or("unknown");
        let detail = failing_step.map(|t| t.detail.as_str()).unwrap_or("");

        let subj = target_name
            .map(String::from)
            .or_else(|| Some(target.to_string()));
        let mut payload = serde_json::json!({
            "rule_id": decision.rule_id,
            "layer": layer,
            "detail": detail,
            "layout": layout.name,
            "verb": verb.as_str(),
            "caller_role": effective_caller.role.as_str(),
            "target": target,
            "origin": origin.to_string(),
        });
        if let Some(target_role) = resolve_target_role(&layout, target) {
            payload["target_role"] = serde_json::Value::String(target_role.to_string());
        }

        let is_shadow = force_shadow || is_shadow_mode();
        if is_shadow {
            eprintln!(
                "would_deny: rule_id={} layer={} layout={}",
                decision.rule_id, layer, layout.name
            );

            let event = crate::bus::FarmEvent::new(
                "would_deny",
                Some(effective_caller.name.clone()),
                subj,
                owner.map(String::from),
                project.map(String::from),
            )
            .with_payload_json(payload.to_string());

            let _ = std::fs::create_dir_all(runs_dir);
            crate::bus::record_event_safely_in_runs(runs_dir, &event);
            return Ok(decision);
        }

        // Enforce mode
        let event = crate::bus::FarmEvent::new(
            "denied",
            Some(effective_caller.name.clone()),
            subj,
            owner.map(String::from),
            project.map(String::from),
        )
        .with_payload_json(payload.to_string());

        let _ = std::fs::create_dir_all(runs_dir);
        crate::bus::record_event_safely_in_runs(runs_dir, &event);

        let err_msg = if let Some(tname) = target_name {
            if verb == Verb::Spawn {
                format!(
                    "error: [{}] {} — cannot spawn {}\n",
                    decision.rule_id, detail, tname
                )
            } else {
                format!("error: [{}] {}\n", decision.rule_id, detail)
            }
        } else {
            format!("error: [{}] {}\n", decision.rule_id, detail)
        };

        return Err((1, err_msg));
    }

    Ok(decision)
}
