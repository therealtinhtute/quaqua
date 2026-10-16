use super::apply::{apply_ops, HerdrDriver};
use super::plan::{is_candidate_worker, is_group_tab, plan_with_deferred};
use super::types::{Op, Snapshot, Target};
use std::collections::HashSet;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ReconcileOptions {
    pub check: bool,
    pub repair: bool,
    pub tab: Option<String>,
    pub only: Option<String>,
    pub owner: Option<String>,
    pub dry_run: bool,
    pub timeout_sec: Option<u64>,
}

impl Default for ReconcileOptions {
    fn default() -> Self {
        ReconcileOptions {
            check: false,
            repair: true,
            tab: None,
            only: None,
            owner: None,
            dry_run: false,
            timeout_sec: None,
        }
    }
}

/// Read timestamp via shared fake-clock hook (PROMPT_WORKER_CLOCK_FILE), else system time
pub fn now_sec() -> u64 {
    if let Ok(clock_file) = std::env::var("PROMPT_WORKER_CLOCK_FILE") {
        if let Ok(content) = std::fs::read_to_string(&clock_file) {
            if let Ok(sec) = content.trim().parse::<u64>() {
                return sec;
            }
        }
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn poll_interval() -> Duration {
    if let Ok(poll_s) = std::env::var("GRID_CHECK_POLL_SEC") {
        if let Ok(s) = poll_s.trim().parse::<f64>() {
            if s > 0.0 {
                return Duration::from_secs_f64(s);
            }
        }
    }
    Duration::from_millis(500)
}

pub fn filter_ops_for_check<'a>(
    ops: &'a [Op],
    snapshot: &Snapshot,
    owner_filter: Option<&str>,
    only_filter: Option<&str>,
    tab_filter: Option<&str>,
) -> Vec<&'a Op> {
    let only_set: Option<HashSet<&str>> = only_filter.map(|s| {
        s.split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .collect()
    });

    ops.iter()
        .filter(|op| {
            if op.deferred {
                return false;
            }

            let pane = snapshot.panes.iter().find(|p| p.pane_id == op.pane);
            let agent = snapshot.agents.iter().find(|a| a.pane_id.as_deref() == Some(&op.pane));

            let name = agent
                .and_then(|a| a.name.as_deref())
                .or_else(|| pane.and_then(|p| p.tokens.as_ref()).and_then(|t| t.grid_name.as_deref()))
                .or_else(|| pane.and_then(|p| p.label.as_deref()))
                .unwrap_or("");

            let powner = pane
                .and_then(|p| p.tokens.as_ref())
                .and_then(|t| t.grid_owner.as_deref())
                .unwrap_or("cos");
            let pgroup = pane
                .and_then(|p| p.tokens.as_ref())
                .and_then(|t| t.grid_group.as_deref())
                .unwrap_or("");

            if let Some(req_owner) = owner_filter {
                if !req_owner.is_empty() && powner != req_owner {
                    return false;
                }
            }

            if let Some(set) = &only_set {
                if !set.is_empty() && !set.contains(name) {
                    return false;
                }
            }

            // dec5a30 bugfix: --tab always scopes the check, with or without --only
            if let Some(tab) = tab_filter {
                if !tab.is_empty() && !is_group_tab(pgroup, Some(tab)) {
                    return false;
                }
            }

            true
        })
        .collect()
}

pub fn run_reconcile<D: HerdrDriver>(
    driver: &mut D,
    opts: &ReconcileOptions,
    default_tab: &str,
    deferred_statuses: Option<&[&str]>,
) -> Result<(), String> {
    let tab = opts.tab.as_deref().unwrap_or(default_tab);
    let owner_env = opts
        .owner
        .clone()
        .or_else(|| std::env::var("QUAQUA_OWNER").ok())
        .unwrap_or_else(|| "cos".to_string());

    if opts.check {
        let timeout_sec = opts.timeout_sec.unwrap_or_else(|| {
            std::env::var("GRID_CHECK_TIMEOUT_SEC")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(60)
        });

        let start_ts = now_sec();
        let poll = poll_interval();

        loop {
            // Finding 7: Retry snapshot failure until timeout
            let snap = match driver.snapshot() {
                Ok(s) => s,
                Err(e) => {
                    let elapsed = now_sec().saturating_sub(start_ts);
                    if elapsed >= timeout_sec {
                        return Err(format!("grid check snapshot failed after {timeout_sec}s: {e}"));
                    }
                    sleep(poll);
                    continue;
                }
            };

            let ops = plan_with_deferred(&snap, deferred_statuses)?;
            let rem_ops = filter_ops_for_check(
                &ops,
                &snap,
                Some(&owner_env),
                opts.only.as_deref(),
                Some(tab),
            );

            if opts.dry_run {
                let serialized = serde_json::to_string(&rem_ops).map_err(|e| e.to_string())?;
                println!("{serialized}");
                if rem_ops.is_empty() {
                    return Ok(());
                } else {
                    return Err(format!("grid check pending ops: {}", rem_ops.len()));
                }
            }

            if rem_ops.is_empty() {
                return Ok(());
            }

            let elapsed = now_sec().saturating_sub(start_ts);
            if elapsed >= timeout_sec {
                let serialized = serde_json::to_string(&rem_ops).unwrap_or_default();
                return Err(format!(
                    "grid check timed out after {timeout_sec}s; pending ops: {serialized}"
                ));
            }

            sleep(poll);
        }
    }

    // Repair / apply mode
    let snap = driver.snapshot()?;

    let only_set: Option<HashSet<&str>> = opts.only.as_deref().map(|s| {
        s.split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .collect()
    });

    let def_list = deferred_statuses.unwrap_or(&["working"]);
    let is_deferred = |st: Option<&str>| st.map(|s| def_list.contains(&s)).unwrap_or(false);

    let owner_workers: Vec<&super::types::Pane> = snap
        .panes
        .iter()
        .filter(|p| {
            if !is_candidate_worker(p) {
                return false;
            }
            let powner = p.tokens.as_ref().and_then(|t| t.grid_owner.as_deref()).unwrap_or("cos");
            owner_env.is_empty() || powner == owner_env
        })
        .collect();

    let total_workers = owner_workers.len();
    if total_workers == 0 {
        return Ok(());
    }

    let owner_ws = owner_workers[0].workspace_id.as_deref().unwrap_or("");
    let anchor_pane_id = super::plan::find_owner_pane(&owner_env, &snap.panes, &snap.agents, owner_ws);
    let anchor_pane = anchor_pane_id
        .as_ref()
        .and_then(|id| snap.panes.iter().find(|p| &p.pane_id == id));
    let anchor_tab = anchor_pane.and_then(|p| p.tab_id.as_deref());

    // Distinct groups in first-seen order
    let mut groups: Vec<String> = Vec::new();
    for w in &owner_workers {
        if let Some(t) = &w.tokens {
            if let Some(g) = &t.grid_group {
                if !groups.contains(g) {
                    groups.push(g.clone());
                }
            }
        }
    }
    if groups.is_empty() {
        groups.push(tab.to_string());
    }

    let mut repair_ops: Vec<Op> = Vec::new();
    let mut movable_workers: Vec<(&super::types::Pane, String)> = Vec::new();

    for group in &groups {
        let mut gws: Vec<&super::types::Pane> = owner_workers
            .iter()
            .copied()
            .filter(|w| w.tokens.as_ref().and_then(|t| t.grid_group.as_deref()) == Some(group.as_str()))
            .collect();

        gws.sort_by(|a, b| {
            let seq_a = super::plan::pane_seq(&a.pane_id);
            let seq_b = super::plan::pane_seq(&b.pane_id);
            seq_a.cmp(&seq_b).then_with(|| a.pane_id.cmp(&b.pane_id))
        });

        let n = gws.len();
        if n == 0 {
            continue;
        }
        if n > 4 {
            return Err(format!("too many workers in group {group}: {n} (max 4)"));
        }

        // Determine movable workers under --only for this group
        for w in &gws {
            let name = snap
                .agents
                .iter()
                .find(|a| a.pane_id.as_deref() == Some(&w.pane_id))
                .and_then(|a| a.name.as_deref())
                .or_else(|| w.tokens.as_ref().and_then(|t| t.grid_name.as_deref()))
                .or_else(|| w.label.as_deref())
                .unwrap_or("");
            if let Some(set) = &only_set {
                if !set.is_empty() && !set.contains(name) {
                    continue;
                }
            }
            movable_workers.push((w, name.to_string()));
        }

        let beside = total_workers == 1 && n == 1 && anchor_pane.is_some();
        if beside {
            let w = gws[0];
            let is_movable = movable_workers.iter().any(|(p, _)| p.pane_id == w.pane_id);
            if is_movable {
                let atab = anchor_tab.unwrap_or("");
                let in_place = w.tab_id.as_deref() == Some(atab);
                if !in_place {
                    repair_ops.push(Op {
                        op: "move".to_string(),
                        pane: w.pane_id.clone(),
                        deferred: is_deferred(w.agent_status.as_deref()),
                        target: Target::Split {
                            tab: atab.to_string(),
                            anchor: anchor_pane.unwrap().pane_id.clone(),
                            direction: "right".to_string(),
                            ratio: 0.4,
                        },
                    });
                }
            }
        } else {
            // !beside -> Dedicated workers tab (N1: never orchestrator tab!)
            let mut matching_tabs: Vec<&super::types::Tab> = snap
                .tabs
                .iter()
                .filter(|t| {
                    let ws_ok = t.workspace_id.as_deref() == Some(owner_ws) || owner_ws.is_empty();
                    let anchor_ok = anchor_tab.map(|at| t.tab_id != at).unwrap_or(true);
                    let group_ok = super::plan::is_group_tab(group, t.label.as_deref());
                    if !(ws_ok && anchor_ok && group_ok) {
                        return false;
                    }
                    let occ: Vec<&super::types::Pane> = snap
                        .panes
                        .iter()
                        .filter(|p| p.tab_id.as_deref() == Some(&t.tab_id))
                        .collect();
                    if occ.is_empty() {
                        return false;
                    }
                    occ.iter().all(|p| gws.iter().any(|gw| gw.pane_id == p.pane_id))
                })
                .collect();

            matching_tabs.sort_by_key(|t| {
                let l = t.label.as_deref().unwrap_or("");
                let exact = l == group
                    || l == format!("{group}-workers")
                    || l == group.strip_suffix("-workers").unwrap_or(group);
                if exact { 0 } else { 1 }
            });

            let valid_tab = matching_tabs.first().copied();

            let mut present: Vec<String> = if let Some(vt) = valid_tab {
                snap.panes
                    .iter()
                    .filter(|p| p.tab_id.as_deref() == Some(&vt.tab_id))
                    .map(|p| p.pane_id.clone())
                    .collect()
            } else {
                Vec::new()
            };

            let build_orders = match n {
                1 => vec![0],
                2 => vec![0, 1],
                3 => vec![0, 2, 1],
                4 => vec![0, 2, 1, 3],
                _ => vec![],
            };

            let target_tab_ref = valid_tab
                .map(|t| t.tab_id.clone())
                .unwrap_or_else(|| "@workers".to_string());

            for idx in build_orders {
                let w = gws[idx];
                let is_movable = movable_workers.iter().any(|(p, _)| p.pane_id == w.pane_id);

                if !is_movable {
                    // Unlisted worker: if it is already in valid_tab, track in present
                    if valid_tab.map(|vt| w.tab_id.as_deref() == Some(&vt.tab_id)).unwrap_or(false) {
                        if !present.contains(&w.pane_id) {
                            present.push(w.pane_id.clone());
                        }
                    }
                    continue;
                }

                let in_place = valid_tab.map(|vt| w.tab_id.as_deref() == Some(&vt.tab_id)).unwrap_or(false)
                    && (present.contains(&w.pane_id) || present.len() == n);

                if in_place {
                    if !present.contains(&w.pane_id) {
                        present.push(w.pane_id.clone());
                    }
                    continue;
                }

                // N1: Fallback logic mirroring grid-workers.sh:441-461
                let (mode, anchor_idx, dir, ratio) = if idx == 0 {
                    ("new-tab", 0, "right", 0.5)
                } else if n == 2 {
                    ("split", 0, "right", 0.5)
                } else if n == 3 {
                    if idx == 2 { ("split", 0, "down", 0.5) } else { ("split", 0, "right", 0.5) }
                } else {
                    if idx == 2 { ("split", 0, "down", 0.5) }
                    else if idx == 1 { ("split", 0, "right", 0.5) }
                    else { ("split", 2, "right", 0.5) }
                };

                if mode == "new-tab" {
                    if present.is_empty() && valid_tab.is_none() {
                        repair_ops.push(Op {
                            op: "move".to_string(),
                            pane: w.pane_id.clone(),
                            deferred: is_deferred(w.agent_status.as_deref()),
                            target: Target::NewTab { label: group.clone() },
                        });
                    } else {
                        let anchor_target = if !present.is_empty() { present[0].clone() } else { w.pane_id.clone() };
                        repair_ops.push(Op {
                            op: "move".to_string(),
                            pane: w.pane_id.clone(),
                            deferred: is_deferred(w.agent_status.as_deref()),
                            target: Target::Split {
                                tab: target_tab_ref.clone(),
                                anchor: anchor_target,
                                direction: "right".to_string(),
                                ratio: 0.5,
                            },
                        });
                    }
                } else {
                    let want_anchor = &gws[anchor_idx].pane_id;
                    if present.contains(want_anchor) {
                        repair_ops.push(Op {
                            op: "move".to_string(),
                            pane: w.pane_id.clone(),
                            deferred: is_deferred(w.agent_status.as_deref()),
                            target: Target::Split {
                                tab: target_tab_ref.clone(),
                                anchor: want_anchor.clone(),
                                direction: dir.to_string(),
                                ratio,
                            },
                        });
                    } else if !present.is_empty() {
                        // Fallback to present[0] in workers tab (grid-workers.sh:454-456)
                        repair_ops.push(Op {
                            op: "move".to_string(),
                            pane: w.pane_id.clone(),
                            deferred: is_deferred(w.agent_status.as_deref()),
                            target: Target::Split {
                                tab: target_tab_ref.clone(),
                                anchor: present[0].clone(),
                                direction: dir.to_string(),
                                ratio,
                            },
                        });
                    } else {
                        // Nobody in workers tab yet! Create new tab! (grid-workers.sh:457-460)
                        repair_ops.push(Op {
                            op: "move".to_string(),
                            pane: w.pane_id.clone(),
                            deferred: is_deferred(w.agent_status.as_deref()),
                            target: Target::NewTab { label: group.clone() },
                        });
                    }
                }
                present.push(w.pane_id.clone());
            }
        }
    }

    if opts.dry_run {
        let serialized = serde_json::to_string(&repair_ops).map_err(|e| e.to_string())?;
        println!("{serialized}");
        return Ok(());
    }

    let pane_map = apply_ops(driver, &repair_ops, Some(&snap))?;

    // Finding 1 remainder: Rename failure is NOT swallowed; matches bash set -e!
    for (w, target_name) in movable_workers {
        let cur_pane = pane_map.get(&w.pane_id).cloned().unwrap_or_else(|| w.pane_id.clone());
        driver.rename_pane(&cur_pane, &target_name)?;
    }

    Ok(())
}
