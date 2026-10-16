use super::types::{Agent, Op, Pane, Rect, Snapshot, Target};
use std::collections::{BTreeSet, HashSet};

const TOL: i64 = 2;

fn near(a: i64, b: i64) -> bool {
    (a - b).abs() <= TOL
}

fn rect_near(a: &Rect, b: &Rect) -> bool {
    near(a.x, b.x) && near(a.y, b.y) && near(a.width, b.width) && near(a.height, b.height)
}

pub fn pane_seq(pane_id: &str) -> u64 {
    let idx = match pane_id.rfind(":p") {
        Some(i) => i + 2,
        None => return 0,
    };
    let s = &pane_id[idx..];
    if s.is_empty() {
        return 0;
    }
    let alpha = "123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ0";
    let mut seq = 0u64;
    for c in s.chars() {
        let pos = match alpha.find(c) {
            Some(p) => (p as u64) + 1,
            None => 0,
        };
        seq = seq * 36 + pos;
    }
    seq
}

pub fn is_candidate_worker(pane: &Pane) -> bool {
    let tokens = match &pane.tokens {
        Some(t) => t,
        None => return false,
    };
    let ready = match &tokens.grid_ready {
        Some(serde_json::Value::String(s)) => s == "1",
        Some(serde_json::Value::Number(n)) => n.as_i64() == Some(1),
        _ => false,
    };
    if !ready {
        return false;
    }
    tokens.grid_role.as_deref() == Some("worker")
        && tokens.grid_owner.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
        && tokens.grid_group.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
}

fn is_status_deferred(status: Option<&str>, deferred_list: &[&str]) -> bool {
    status.map(|s| deferred_list.contains(&s)).unwrap_or(false)
}

pub fn is_group_tab(group: &str, label: Option<&str>) -> bool {
    let l = match label {
        Some(s) if !s.is_empty() => s,
        _ => return false,
    };
    l == group
        || l == format!("{group}-workers")
        || l == group.strip_suffix("-workers").unwrap_or(group)
        || l == "workers"
}

fn cells(n: usize, a: &Rect) -> Vec<Rect> {
    let x = a.x;
    let y = a.y;
    let w = if a.width != 0 { a.width } else { 200 };
    let h = if a.height != 0 { a.height } else { 50 };
    let hw = w / 2;
    let hh = h / 2;
    let cw = (w * 2) / 5;
    match n {
        1 => vec![
            Rect { x, y, width: cw, height: h },
            Rect { x: x + cw, y, width: w - cw, height: h },
        ],
        2 => vec![
            Rect { x, y, width: hw, height: h },
            Rect { x: x + hw, y, width: w - hw, height: h },
        ],
        3 => vec![
            Rect { x, y, width: hw, height: hh },
            Rect { x: x + hw, y, width: w - hw, height: hh },
            Rect { x, y: y + hh, width: w, height: h - hh },
        ],
        4 => vec![
            Rect { x, y, width: hw, height: hh },
            Rect { x: x + hw, y, width: w - hw, height: hh },
            Rect { x, y: y + hh, width: hw, height: h - hh },
            Rect { x: x + hw, y: y + hh, width: w - hw, height: h - hh },
        ],
        _ => vec![],
    }
}

fn build_order(n: usize) -> Vec<usize> {
    match n {
        1 => vec![0],
        2 => vec![0, 1],
        3 => vec![0, 2, 1],
        4 => vec![0, 2, 1, 3],
        _ => vec![],
    }
}

fn split_target(n: usize, i: usize, gws: &[&Pane], tab_ref: &str) -> Target {
    match n {
        2 => Target::Split {
            tab: tab_ref.to_string(),
            anchor: gws[0].pane_id.clone(),
            direction: "right".to_string(),
            ratio: 0.5,
        },
        3 => {
            if i == 2 {
                Target::Split {
                    tab: tab_ref.to_string(),
                    anchor: gws[0].pane_id.clone(),
                    direction: "down".to_string(),
                    ratio: 0.5,
                }
            } else {
                Target::Split {
                    tab: tab_ref.to_string(),
                    anchor: gws[0].pane_id.clone(),
                    direction: "right".to_string(),
                    ratio: 0.5,
                }
            }
        }
        4 => {
            if i == 2 {
                Target::Split {
                    tab: tab_ref.to_string(),
                    anchor: gws[0].pane_id.clone(),
                    direction: "down".to_string(),
                    ratio: 0.5,
                }
            } else if i == 1 {
                Target::Split {
                    tab: tab_ref.to_string(),
                    anchor: gws[0].pane_id.clone(),
                    direction: "right".to_string(),
                    ratio: 0.5,
                }
            } else {
                Target::Split {
                    tab: tab_ref.to_string(),
                    anchor: gws[2].pane_id.clone(),
                    direction: "right".to_string(),
                    ratio: 0.5,
                }
            }
        }
        _ => unreachable!(),
    }
}

pub fn find_owner_pane(owner: &str, panes: &[Pane], agents: &[Agent], ws: &str) -> Option<String> {
    // 1. Pane tagged with matching grid_owner and non-worker role
    for p in panes {
        if let Some(t) = &p.tokens {
            if t.grid_owner.as_deref() == Some(owner) && t.grid_role.as_deref() != Some("worker") {
                return Some(p.pane_id.clone());
            }
        }
    }
    // 2. Registered agent whose name == owner
    for a in agents {
        if a.name.as_deref() == Some(owner) {
            if let Some(pid) = &a.pane_id {
                return Some(pid.clone());
            }
        }
    }
    // 3. Pane whose label == owner
    for p in panes {
        if p.label.as_deref() == Some(owner) {
            return Some(p.pane_id.clone());
        }
    }
    // 4. Pane whose agent == owner
    for p in panes {
        if p.agent.as_deref() == Some(owner) {
            return Some(p.pane_id.clone());
        }
    }
    // 5. For cos only: pane tagged orchestrator/lead/cos or first non-worker pane
    if owner == "cos" {
        for p in panes {
            if p.workspace_id.as_deref() == Some(ws) {
                if let Some(t) = &p.tokens {
                    let role = t.grid_role.as_deref().unwrap_or("");
                    if role == "orchestrator" || role == "cos" || role == "lead" {
                        return Some(p.pane_id.clone());
                    }
                }
            }
        }
        for p in panes {
            if p.workspace_id.as_deref() == Some(ws) && !is_candidate_worker(p) {
                return Some(p.pane_id.clone());
            }
        }
    }
    None
}

/// A split whose anchor is deferred cannot run, and a split into `@workers`
/// cannot run while the group's new tab is deferred (there is nowhere to land).
/// The apply path and the hook both propagate that through this same rule;
/// the planner must too, or the check counts an unrunnable op as pending and
/// times out while its anchor is still working.
fn propagate_deferrals(ops: &mut [Op]) {
    let mut deferred_panes: HashSet<String> = HashSet::new();
    let mut workers_tab_deferred = false;
    for op in ops.iter_mut() {
        if op.deferred {
            deferred_panes.insert(op.pane.clone());
            if matches!(op.target, Target::NewTab { .. }) {
                workers_tab_deferred = true;
            }
            continue;
        }
        if let Target::Split { tab, anchor, .. } = &op.target {
            if deferred_panes.contains(anchor.as_str())
                || (tab == "@workers" && workers_tab_deferred)
            {
                op.deferred = true;
                deferred_panes.insert(op.pane.clone());
            }
        }
    }
}

/// Pure layout planner: turns a snapshot into an ordered list of layout ops.
pub fn plan(snapshot: &Snapshot) -> Result<Vec<Op>, String> {
    plan_with_deferred(snapshot, None)
}

/// Layout planner with custom deferred statuses from config.
pub fn plan_with_deferred(
    snapshot: &Snapshot,
    deferred_statuses: Option<&[&str]>,
) -> Result<Vec<Op>, String> {
    let def_list = deferred_statuses.unwrap_or(&["working"]);
    let candidate_workers: Vec<&Pane> = snapshot
        .panes
        .iter()
        .filter(|p| is_candidate_worker(p))
        .collect();

    if candidate_workers.is_empty() {
        return Ok(vec![]);
    }
    // Distinct owners in sorted order (jq: `[ $candidate_workers[].tokens.grid_owner ] | unique`)
    let mut owners: BTreeSet<&str> = BTreeSet::new();
    for p in &candidate_workers {
        if let Some(t) = &p.tokens {
            if let Some(o) = &t.grid_owner {
                owners.insert(o.as_str());
            }
        }
    }

    let mut ops = Vec::new();

    for owner in owners {
        let mut owner_workers: Vec<&Pane> = candidate_workers
            .iter()
            .copied()
            .filter(|p| p.tokens.as_ref().and_then(|t| t.grid_owner.as_deref()) == Some(owner))
            .collect();

        owner_workers.sort_by(|a, b| {
            let seq_a = pane_seq(&a.pane_id);
            let seq_b = pane_seq(&b.pane_id);
            seq_a.cmp(&seq_b).then_with(|| a.pane_id.cmp(&b.pane_id))
        });

        let total = owner_workers.len();
        if total == 0 {
            continue;
        }

        let owner_ws = owner_workers[0]
            .workspace_id
            .as_deref()
            .unwrap_or("");
        let anchor_pane_id = find_owner_pane(owner, &snapshot.panes, &snapshot.agents, owner_ws);
        let anchor_pane = anchor_pane_id
            .as_ref()
            .and_then(|id| snapshot.panes.iter().find(|p| &p.pane_id == id));
        let anchor_tab = anchor_pane.and_then(|p| p.tab_id.as_deref());

        if total == 1 && anchor_pane.is_some() {
            let w = owner_workers[0];
            // "Beside the anchor" is acceptance, not a target ratio: the pane is
            // arranged as soon as it shares the anchor's tab. The repair path
            // (reconcile.rs `beside`) uses exactly this test, so the check and the
            // repair must not disagree — a herdr `pane split` lands 50/50, never
            // the old 2/5|3/5, and the old rect_near test made `--check` time out
            // forever on an already-arranged pane.
            let atab = anchor_tab.unwrap_or_default();
            let in_place = anchor_tab.is_some() && w.tab_id.as_deref() == Some(atab);

            if !in_place {
                ops.push(Op {
                    op: "move".to_string(),
                    pane: w.pane_id.clone(),
                    deferred: is_status_deferred(w.agent_status.as_deref(), def_list),
                    target: Target::Split {
                        tab: atab.to_string(),
                        anchor: anchor_pane.unwrap().pane_id.clone(),
                        direction: "right".to_string(),
                        ratio: 0.4,
                    },
                });
            }
        } else {
            // >1 workers total for this owner -> one tab per group
            // Distinct groups in sorted order (jq: `[ $owner_workers[].tokens.grid_group ] | unique`)
            let mut groups: BTreeSet<&str> = BTreeSet::new();
            for w in &owner_workers {
                if let Some(t) = &w.tokens {
                    if let Some(g) = &t.grid_group {
                        groups.insert(g.as_str());
                    }
                }
            }

            for group in groups {
                let mut gws: Vec<&Pane> = owner_workers
                    .iter()
                    .copied()
                    .filter(|w| w.tokens.as_ref().and_then(|t| t.grid_group.as_deref()) == Some(group))
                    .collect();

                gws.sort_by(|a, b| {
                    let seq_a = pane_seq(&a.pane_id);
                    let seq_b = pane_seq(&b.pane_id);
                    seq_a.cmp(&seq_b).then_with(|| a.pane_id.cmp(&b.pane_id))
                });

                let n = gws.len();
                if n > 4 {
                    return Err(format!("too many workers in group {group}: {n} (max 4)"));
                }

                // Search for an existing valid workers tab for this group
                // jq:
                // [ $tabs[]
                //   | select((.workspace_id == $owner_ws or $owner_ws == "")
                //            and ($anchor_tab == null or .tab_id != $anchor_tab)
                //            and is_group_tab($group; .label))
                //   | .tab_id as $tid
                //   | [ $panes[] | select(.tab_id == $tid) ] as $occ
                //   | select(($occ | length > 0)
                //            and ($occ | all(. as $p | any($gws[]; .pane_id == $p.pane_id))))
                // ]
                // | sort_by(if (.label == $group or .label == "\($group)-workers" or .label == ($group | rtrimstr("-workers"))) then 0 else 1 end)
                // | first // null
                let mut matching_tabs: Vec<&super::types::Tab> = snapshot
                    .tabs
                    .iter()
                    .filter(|tab| {
                        let ws_ok = tab.workspace_id.as_deref() == Some(owner_ws) || owner_ws.is_empty();
                        let anchor_ok = anchor_tab.map(|at| tab.tab_id != at).unwrap_or(true);
                        let group_ok = is_group_tab(group, tab.label.as_deref());
                        if !(ws_ok && anchor_ok && group_ok) {
                            return false;
                        }
                        let occ: Vec<&Pane> = snapshot
                            .panes
                            .iter()
                            .filter(|p| p.tab_id.as_deref() == Some(&tab.tab_id))
                            .collect();
                        if occ.is_empty() {
                            return false;
                        }
                        occ.iter().all(|p| gws.iter().any(|gw| gw.pane_id == p.pane_id))
                    })
                    .collect();

                matching_tabs.sort_by_key(|tab| {
                    let l = tab.label.as_deref().unwrap_or("");
                    let exact = l == group
                        || l == format!("{group}-workers")
                        || l == group.strip_suffix("-workers").unwrap_or(group);
                    if exact {
                        0
                    } else {
                        1
                    }
                });

                let valid_tab = matching_tabs.first().copied();

                if let Some(vt) = valid_tab {
                    let wt_id = &vt.tab_id;
                    let wt_layout = snapshot.layouts.iter().find(|l| &l.tab_id == wt_id);
                    let wt_area = wt_layout
                        .and_then(|l| l.area)
                        .unwrap_or(Rect { x: 0, y: 0, width: 200, height: 50 });
                    let occ_count = snapshot
                        .panes
                        .iter()
                        .filter(|p| p.tab_id.as_deref() == Some(wt_id.as_str()))
                        .count();
                    let all_present = gws.iter().all(|w| w.tab_id.as_deref() == Some(wt_id.as_str()))
                        && occ_count == n;

                    let in_place = if all_present {
                        if n == 1 {
                            let r0 = wt_layout.and_then(|l| {
                                l.panes
                                    .iter()
                                    .find(|p| p.pane_id == gws[0].pane_id)
                                    .and_then(|p| p.rect)
                            });
                            r0.map(|r| rect_near(&r, &wt_area)).unwrap_or(false)
                        } else {
                            let req_cells = cells(n, &wt_area);
                            (0..n).all(|i| {
                                let ri = wt_layout.and_then(|l| {
                                    l.panes
                                        .iter()
                                        .find(|p| p.pane_id == gws[i].pane_id)
                                        .and_then(|p| p.rect)
                                });
                                ri.map(|r| rect_near(&r, &req_cells[i])).unwrap_or(false)
                            })
                        }
                    } else {
                        false
                    };

                    if !in_place {
                        let in_tab_count = gws
                            .iter()
                            .filter(|w| w.tab_id.as_deref() == Some(wt_id.as_str()))
                            .count();
                        if in_tab_count > 0 && gws[0].tab_id.as_deref() == Some(wt_id.as_str()) {
                            // Worker 0 is already in tab; place remaining workers
                            for i in build_order(n) {
                                if i > 0 && gws[i].tab_id.as_deref() != Some(wt_id.as_str()) {
                                    ops.push(Op {
                                        op: "move".to_string(),
                                        pane: gws[i].pane_id.clone(),
                                        deferred: is_status_deferred(gws[i].agent_status.as_deref(), def_list),
                                        target: split_target(n, i, &gws, wt_id),
                                    });
                                }
                            }
                        } else {
                            // Need full rebuild in workers tab
                            for i in build_order(n) {
                                if i == 0 {
                                    ops.push(Op {
                                        op: "move".to_string(),
                                        pane: gws[0].pane_id.clone(),
                                        deferred: is_status_deferred(gws[0].agent_status.as_deref(), def_list),
                                        target: Target::NewTab {
                                            label: group.to_string(),
                                        },
                                    });
                                } else {
                                    ops.push(Op {
                                        op: "move".to_string(),
                                        pane: gws[i].pane_id.clone(),
                                        deferred: is_status_deferred(gws[i].agent_status.as_deref(), def_list),
                                        target: split_target(n, i, &gws, "@workers"),
                                    });
                                }
                            }
                        }
                    }
                } else {
                    // No valid workers tab exists -> create new tab and split
                    for i in build_order(n) {
                        if i == 0 {
                            ops.push(Op {
                                op: "move".to_string(),
                                pane: gws[0].pane_id.clone(),
                                deferred: is_status_deferred(gws[0].agent_status.as_deref(), def_list),
                                target: Target::NewTab {
                                    label: group.to_string(),
                                },
                            });
                        } else {
                            ops.push(Op {
                                op: "move".to_string(),
                                pane: gws[i].pane_id.clone(),
                                deferred: is_status_deferred(gws[i].agent_status.as_deref(), def_list),
                                target: split_target(n, i, &gws, "@workers"),
                            });
                        }
                    }
                }
            }
        }
    }

    propagate_deferrals(&mut ops);
    Ok(ops)
}
