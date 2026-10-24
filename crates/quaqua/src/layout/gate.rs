use crate::identity::{Caller, CallerDepth, Role};
use crate::layout::model::{Decision, Layout, TraceStep, Verb};

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
pub fn check(
    layout: &Layout,
    caller: &Caller,
    verb: Verb,
    target: &str,
) -> Decision {
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
    let edge_allowed = match verb {
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
    trace.push(TraceStep {
        layer: "edge".to_string(),
        passed: true,
        detail: format!(
            "{} edge permits '{}' -> '{}'",
            verb, caller_role_name, target_role
        ),
    });

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
                return Decision::deny("depth.exceeded", trace);
            }
            CallerDepth::MissingOwner(owner) => {
                trace.push(TraceStep {
                    layer: "depth".to_string(),
                    passed: false,
                    detail: format!("missing owner in chain: {}", owner),
                });
                return Decision::deny("depth.exceeded", trace);
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
                    trace.push(TraceStep {
                        layer: "cap".to_string(),
                        passed: false,
                        detail: format!(
                            "role '{}' has reached child cap {} (current count {})",
                            caller_role_name, max, count
                        ),
                    });
                    return Decision::deny("cap.exceeded", trace);
                }
                trace.push(TraceStep {
                    layer: "cap".to_string(),
                    passed: true,
                    detail: format!(
                        "role '{}' count {} < cap {}",
                        caller_role_name, count, max
                    ),
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
            layout.name, caller_role_name, verb.as_str(), target_role
        )
    };

    Decision::allow(rule_id, trace)
}
