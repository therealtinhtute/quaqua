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

/// Fake driver for testing without calling real herdr
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
        let tab_id = match target {
            Target::NewTab { .. } => format!("w1:t_{}", self.next_tab_counter),
            Target::Split { tab, .. } => tab.clone(),
        };
        Ok(MoveResult {
            pane_id: pane.to_string(),
            tab_id,
            previous_pane_id: None,
        })
    }

    fn rename_pane(&mut self, pane: &str, label: &str) -> Result<(), String> {
        self.executed_renames.push((pane.to_string(), label.to_string()));
        Ok(())
    }
}

/// Execute layout ops via a HerdrDriver, propagating deferrals, parking panes
/// if in-tab repositioning, and remapping pane/tab IDs as needed.
pub fn apply_ops<D: HerdrDriver>(
    driver: &mut D,
    ops: &[Op],
    snapshot: Option<&Snapshot>,
) -> Result<usize, String> {
    let mut pane_map: HashMap<String, String> = HashMap::new();
    let mut pane_tabs: HashMap<String, String> = HashMap::new();
    let mut latest_tab: Option<String> = None;
    let mut applied_count = 0;
    let mut scratch_counter = 0;

    let mut deferred_panes: HashSet<String> = HashSet::new();
    let mut workers_tab_deferred = false;

    if let Some(snap) = snapshot {
        for p in &snap.panes {
            if let Some(tid) = &p.tab_id {
                pane_tabs.insert(p.pane_id.clone(), tid.clone());
            }
        }
    }

    for op in ops {
        // Finding 3: Propagate deferral to dependent ops
        if op.deferred {
            deferred_panes.insert(op.pane.clone());
            if matches!(op.target, Target::NewTab { .. }) {
                workers_tab_deferred = true;
            }
            continue;
        }

        if let Target::Split { tab, anchor, .. } = &op.target {
            if deferred_panes.contains(anchor) || (tab == "@workers" && workers_tab_deferred) {
                deferred_panes.insert(op.pane.clone());
                continue;
            }
        }

        let actual_pane = pane_map.get(&op.pane).cloned().unwrap_or_else(|| op.pane.clone());
        let current_tab = pane_tabs.get(&actual_pane).cloned();

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
                let actual_tab = if tab == "@workers" {
                    latest_tab.clone().unwrap_or_else(|| tab.clone())
                } else {
                    tab.clone()
                };
                let actual_anchor = pane_map
                    .get(anchor)
                    .cloned()
                    .unwrap_or_else(|| anchor.clone());
                Target::Split {
                    tab: actual_tab,
                    anchor: actual_anchor,
                    direction: direction.clone(),
                    ratio: *ratio,
                }
            }
        };

        // Findings 1 & 5: If current_tab == target_tab during split, park in scratch tab first
        let mut parked_pane: Option<String> = None;
        let effective_pane = if let Target::Split { tab: ref target_tab, .. } = &remapped_target {
            if current_tab.as_ref() == Some(target_tab) {
                scratch_counter += 1;
                let scratch_label = format!("grid-scratch-quaqua-{}", scratch_counter);
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
            pane_tabs.insert(new_id.clone(), res.tab_id.clone());
            if matches!(op.target, Target::NewTab { .. }) {
                latest_tab = Some(res.tab_id);
            }
        }

        applied_count += 1;
    }

    Ok(applied_count)
}
