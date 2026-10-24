use crate::identity::{Caller, CallerDepth, Role};
use crate::layout::model::{Decision, Layout, TraceStep, Verb};

/// Resolve the role name for a target string.
///
/// If target matches a defined role in the layout or a known role class, use it directly.
/// If target is an agent name ending with `-lead`, role is `lead`.
/// If target is `cos`, role is `cos`.
/// Otherwise, defaults to `peer`.
pub fn resolve_target_role<'a>(layout: &'a Layout, target: &'a str) -> &'a str {
    let target_trim = target.trim();
    if layout.roles.contains_key(target_trim) {
        return target_trim;
    }
    if target_trim == "cos" {
        return "cos";
    }
    if target_trim.ends_with("-lead") {
        return "lead";
    }
    if target_trim == "taskforce" {
        return "taskforce";
    }
    if crate::config::ROLE_CLASSES.contains(&target_trim) {
        return target_trim;
    }
    "peer"
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

    // 2. Role layer
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

    // 3. Edge layer
    let target_role = resolve_target_role(layout, target);
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
