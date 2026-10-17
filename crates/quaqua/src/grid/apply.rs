use super::plan::is_candidate_worker;
use super::types::{Op, Snapshot, Target};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveResult {
    pub pane_id: String,
    pub tab_id: String,
    pub previous_pane_id: Option<String>,
}

pub trait HerdrDriver {
    fn snapshot(&mut self) -> Result<Snapshot, String>;
    fn move_pane(&mut self, pane: &str, target: &Target) -> Result<MoveResult, String>;
    fn rename_pane(&mut self, pane: &str, label: &str) -> Result<(), String>;
}

pub struct CliHerdrDriver {
    pub bin: String,
}

impl CliHerdrDriver {
    pub fn new() -> Self {
        let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".to_string());
        CliHerdrDriver { bin }
    }
}

impl Default for CliHerdrDriver {
    fn default() -> Self {
        Self::new()
    }
}

pub fn parse_move_response(val: &serde_json::Value, orig_pane: &str) -> Result<MoveResult, String> {
    let mr = val
        .get("result")
        .and_then(|r| r.get("move_result"))
        .or_else(|| val.get("move_result"))
        .unwrap_or(val);

    let changed = mr.get("changed").and_then(|v| v.as_bool()).unwrap_or(false);
    let reason = mr.get("reason").and_then(|v| v.as_str()).unwrap_or("unknown");

    if !changed {
        return Err(format!(
            "pane.move changed nothing for {orig_pane} (changed=false, reason={reason})"
        ));
    }

    let pane_obj = mr.get("pane");
    let new_pane = pane_obj
        .and_then(|p| p.get("pane_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing pane.pane_id in move result for {orig_pane}"))?
        .to_string();

    let new_tab = pane_obj
        .and_then(|p| p.get("tab_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let prev_pane = mr
        .get("previous_pane_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(MoveResult {
        pane_id: new_pane,
        tab_id: new_tab,
        previous_pane_id: prev_pane,
    })
}

impl HerdrDriver for CliHerdrDriver {
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        if let Ok(stub_path) = std::env::var("HERDR_STUB_SNAPSHOT") {
            let content = std::fs::read_to_string(&stub_path)
                .map_err(|e| format!("cannot read HERDR_STUB_SNAPSHOT {stub_path}: {e}"))?;
            let val: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| format!("invalid stub snapshot JSON: {e}"))?;
            return super::types::parse_snapshot_value(&val);
        }

        let out = std::process::Command::new(&self.bin)
            .args(["api", "snapshot"])
            .output()
            .map_err(|e| format!("cannot run {} api snapshot: {e}", self.bin))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("{} api snapshot failed: {err}", self.bin));
        }
        let val: serde_json::Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("invalid snapshot JSON from herdr: {e}"))?;
        super::types::parse_snapshot_value(&val)
    }

    fn move_pane(&mut self, pane: &str, target: &Target) -> Result<MoveResult, String> {
        if std::env::var("HERDR_STUB_SNAPSHOT").is_ok() {
            let tab_id = match target {
                Target::NewTab { .. } => "w1:t_stub_new".to_string(),
                Target::Split { tab, .. } => tab.clone(),
            };
            return Ok(MoveResult {
                pane_id: pane.to_string(),
                tab_id,
                previous_pane_id: None,
            });
        }

        let mut cmd = std::process::Command::new(&self.bin);
        cmd.args(["pane", "move", pane]);
        match target {
            Target::NewTab { label } => {
                cmd.args(["--new-tab", "--label", label, "--no-focus"]);
            }
            Target::Split {
                tab,
                anchor,
                direction,
                ratio,
            } => {
                let ratio_s = format!("{ratio:.1}");
                cmd.args([
                    "--tab",
                    tab,
                    "--split",
                    direction,
                    "--target-pane",
                    anchor,
                    "--ratio",
                    &ratio_s,
                    "--no-focus",
                ]);
            }
        }
        let out = cmd
            .output()
            .map_err(|e| format!("cannot run {} pane move: {e}", self.bin))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("pane move failed: {err}"));
        }
        let val: serde_json::Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("invalid move response JSON: {e}"))?;
        parse_move_response(&val, pane)
    }

    fn rename_pane(&mut self, pane: &str, label: &str) -> Result<(), String> {
        if std::env::var("HERDR_STUB_SNAPSHOT").is_ok() {
            return Ok(());
        }
        let out = std::process::Command::new(&self.bin)
            .args(["pane", "rename", pane, label])
            .output()
            .map_err(|e| format!("cannot run {} pane rename: {e}", self.bin))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("pane rename failed: {err}"));
        }
        Ok(())
    }
}

/// Fake driver for testing without calling real herdr.
/// Models reality:
/// - Assigns a new pane ID on move.
/// - Creates a tab/layout on NewTab.
/// - Updates current_snapshot with the new pane ID and tab ID.
/// - Updates agent pane_ids (N2).
pub struct FakeHerdrDriver {
    pub current_snapshot: Snapshot,
    pub executed_moves: Vec<(String, Target)>,
    pub executed_renames: Vec<(String, String)>,
    pub next_pane_counter: u64,
    pub next_tab_counter: u64,
}

impl FakeHerdrDriver {
    pub fn new(snapshot: Snapshot) -> Self {
        FakeHerdrDriver {
            current_snapshot: snapshot,
            executed_moves: Vec::new(),
            executed_renames: Vec::new(),
            next_pane_counter: 100,
            next_tab_counter: 100,
        }
    }
}

impl HerdrDriver for FakeHerdrDriver {
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        Ok(self.current_snapshot.clone())
    }

    fn move_pane(&mut self, pane: &str, target: &Target) -> Result<MoveResult, String> {
        self.executed_moves.push((pane.to_string(), target.clone()));
        self.next_pane_counter += 1;
        self.next_tab_counter += 1;
        let new_pane_id = format!("w1:p{}", self.next_pane_counter);

        let tab_id = match target {
            Target::NewTab { label } => {
                let tid = format!("w1:t_{}", self.next_tab_counter);
                self.current_snapshot.tabs.push(super::types::Tab {
                    tab_id: tid.clone(),
                    workspace_id: Some("w1".to_string()),
                    label: Some(label.clone()),
                    pane_count: Some(1),
                });
                self.current_snapshot.layouts.push(super::types::Layout {
                    tab_id: tid.clone(),
                    workspace_id: Some("w1".to_string()),
                    area: Some(super::types::Rect { x: 0, y: 0, width: 200, height: 50 }),
                    panes: vec![super::types::PaneLayout {
                        pane_id: new_pane_id.clone(),
                        rect: Some(super::types::Rect { x: 0, y: 0, width: 200, height: 50 }),
                    }],
                    zoomed: None,
                });
                tid
            }
            Target::Split { tab, .. } => tab.clone(),
        };

        // Update current_snapshot so subsequent fresh snapshots reflect the moved pane
        if let Some(p) = self.current_snapshot.panes.iter_mut().find(|p| p.pane_id == pane) {
            p.pane_id = new_pane_id.clone();
            p.tab_id = Some(tab_id.clone());
        }

        // N2: Update agent pane_id
        for a in &mut self.current_snapshot.agents {
            if a.pane_id.as_deref() == Some(pane) {
                a.pane_id = Some(new_pane_id.clone());
            }
        }

        Ok(MoveResult {
            pane_id: new_pane_id,
            tab_id,
            previous_pane_id: Some(pane.to_string()),
        })
    }

    fn rename_pane(&mut self, pane: &str, label: &str) -> Result<(), String> {
        self.executed_renames.push((pane.to_string(), label.to_string()));
        if let Some(p) = self.current_snapshot.panes.iter_mut().find(|p| p.pane_id == pane) {
            p.label = Some(label.to_string());
        }
        Ok(())
    }
}

/// Execute layout ops via a HerdrDriver, verifying fresh snapshots before each move,
/// propagating deferrals, parking panes if in-tab repositioning, and returning
/// the remapped ID dictionary.
pub fn apply_ops<D: HerdrDriver>(
    driver: &mut D,
    ops: &[Op],
    _initial_snapshot: Option<&Snapshot>,
) -> Result<HashMap<String, String>, String> {
    let mut pane_map: HashMap<String, String> = HashMap::new();
    let mut latest_tab: Option<String> = None;
    let mut latest_tab_2: Option<String> = None;
    let mut scratch_counter = 0;

    let mut deferred_panes: HashSet<String> = HashSet::new();
    let mut workers_tab_deferred = false;
    let mut workers_tab_2_deferred = false;

    for op in ops {
        // Finding 3: Propagate deferral to dependent ops
        if op.deferred {
            deferred_panes.insert(op.pane.clone());
            if let Target::NewTab { label } = &op.target {
                if label.ends_with("-2") {
                    workers_tab_2_deferred = true;
                } else {
                    workers_tab_deferred = true;
                }
            }
            continue;
        }

        if let Target::Split { tab, anchor, .. } = &op.target {
            let tab_def = if tab == "@workers" {
                workers_tab_deferred
            } else if tab == "@workers-2" {
                workers_tab_2_deferred
            } else {
                false
            };
            if deferred_panes.contains(anchor) || tab_def {
                deferred_panes.insert(op.pane.clone());
                continue;
            }
        }

        let actual_pane = pane_map.get(&op.pane).cloned().unwrap_or_else(|| op.pane.clone());

        // Finding 5: Fetch fresh snapshot before each move to verify pane state
        let fresh_snap = driver.snapshot()?;
        let pane_info = fresh_snap.panes.iter().find(|p| p.pane_id == actual_pane);
        let pane_obj = match pane_info {
            Some(p) => p,
            None => return Err(format!("apply: pane {actual_pane} missing from snapshot")),
        };

        if !is_candidate_worker(pane_obj) {
            return Err(format!("apply: pane {actual_pane} is not a candidate worker"));
        }

        // Finding 2: Recheck idle status before each move
        if pane_obj.agent_status.as_deref() == Some("working") {
            return Err(format!("pane {actual_pane} became busy; replan required"));
        }

        let current_tab = pane_obj.tab_id.clone();
        let worker_group = pane_obj
            .tokens
            .as_ref()
            .and_then(|t| t.grid_group.as_deref())
            .unwrap_or("workers");

        // Finding 5: Check if current tab is zoomed
        if let Some(ctab) = &current_tab {
            if let Some(layout) = fresh_snap.layouts.iter().find(|l| &l.tab_id == ctab) {
                if layout.zoomed == Some(true) {
                    return Err(format!("apply: tab {ctab} is zoomed"));
                }
            }
        }

        let remapped_target = match &op.target {
            Target::NewTab { label } => Target::NewTab {
                label: label.clone(),
            },
            Target::Split {
                tab,
                anchor,
                direction,
                ratio,
            } => {
                let actual_anchor = pane_map
                    .get(anchor)
                    .cloned()
                    .unwrap_or_else(|| anchor.clone());

                let anchor_pane = fresh_snap
                    .panes
                    .iter()
                    .find(|p| p.pane_id == actual_anchor)
                    .ok_or_else(|| format!("apply: anchor {actual_anchor} missing from snapshot"))?;

                // Finding 5 & N1: Resolve @workers to the workers tab, NEVER the orchestrator tab
                let actual_tab = if tab == "@workers" || tab.starts_with("@workers-") {
                    let mut resolved = None;
                    if tab == "@workers-2" {
                        if let Some(lt2) = &latest_tab_2 {
                            resolved = Some(lt2.clone());
                        }
                    } else if let Some(lt) = &latest_tab {
                        resolved = Some(lt.clone());
                    }

                    if let Some(r) = resolved {
                        r
                    } else {
                        // Look for an existing valid workers tab for this group,
                        // by the same acceptance predicate the plan and the
                        // repair use: the group tab, or the exact label the pane
                        // was spawned into (`grid_tab`).
                        let anchor_tid = anchor_pane.tab_id.as_deref().unwrap_or("");
                        let worker_gtab = pane_obj
                            .tokens
                            .as_ref()
                            .and_then(|t| t.grid_tab.as_deref());
                        let is_workers_tab = fresh_snap.tabs.iter().find(|t| t.tab_id == anchor_tid).map(|t| {
                            super::plan::accepts_workers_tab(worker_group, worker_gtab, t.label.as_deref())
                        }).unwrap_or(false);

                        if is_workers_tab {
                            anchor_tid.to_string()
                        } else {
                            return Err(format!("apply: cannot resolve {tab} tab for group {worker_group}"));
                        }
                    }
                } else {
                    tab.clone()
                };

                // Finding 5: Target tab zoom verification
                if let Some(layout) = fresh_snap.layouts.iter().find(|l| l.tab_id == actual_tab) {
                    if layout.zoomed == Some(true) {
                        return Err(format!("apply: target tab {actual_tab} is zoomed"));
                    }
                }

                // Finding 5 remainder: Anchor is in the target tab check (apply.sh:209-213)
                let anchor_tab = anchor_pane.tab_id.as_deref().unwrap_or("");
                if anchor_tab != actual_tab {
                    return Err(format!(
                        "apply: anchor {actual_anchor} is in tab {anchor_tab}, expected target tab {actual_tab}"
                    ));
                }

                if let Some(atokens) = &anchor_pane.tokens {
                    if atokens.grid_role.as_deref() == Some("worker") {
                        let anchor_owner = atokens.grid_owner.as_deref().unwrap_or("");
                        let pane_owner = pane_obj.tokens.as_ref().and_then(|t| t.grid_owner.as_deref()).unwrap_or("");
                        if anchor_owner != pane_owner {
                            return Err(format!("apply: anchor owner {anchor_owner} != pane owner {pane_owner}"));
                        }
                    }
                }

                Target::Split {
                    tab: actual_tab,
                    anchor: actual_anchor,
                    direction: direction.clone(),
                    ratio: *ratio,
                }
            }
        };

        // Findings 1, 5, 9: If current_tab == target_tab during split, park in scratch tab first
        let mut parked_pane: Option<String> = None;
        let effective_pane = if let Target::Split { tab: target_tab, .. } = &remapped_target {
            if current_tab.as_deref() == Some(target_tab.as_str()) {
                scratch_counter += 1;
                let scratch_label = format!("grid-scratch-{}-{}-{}", worker_group, std::process::id(), scratch_counter);
                let park_res = driver.move_pane(
                    &actual_pane,
                    &Target::NewTab {
                        label: scratch_label,
                    },
                )?;
                parked_pane = Some(park_res.pane_id.clone());
                park_res.pane_id
            } else {
                actual_pane.clone()
            }
        } else {
            actual_pane.clone()
        };

        let res = driver.move_pane(&effective_pane, &remapped_target)?;

        let new_id = res.pane_id.clone();
        if let Some(prev) = &res.previous_pane_id {
            pane_map.insert(prev.clone(), new_id.clone());
        }
        pane_map.insert(op.pane.clone(), new_id.clone());
        pane_map.insert(actual_pane.clone(), new_id.clone());
        pane_map.insert(effective_pane.clone(), new_id.clone());
        if let Some(parked) = parked_pane {
            pane_map.insert(parked, new_id.clone());
        }

        if !res.tab_id.is_empty() {
            if let Target::NewTab { label } = &op.target {
                if label.ends_with("-2") {
                    latest_tab_2 = Some(res.tab_id.clone());
                } else {
                    latest_tab = Some(res.tab_id.clone());
                }
            }
        }
    }

    Ok(pane_map)
}
