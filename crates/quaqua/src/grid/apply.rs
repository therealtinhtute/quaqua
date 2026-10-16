use super::types::{Op, Snapshot, Target};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct MoveResult {
    pub pane_id: String,
    pub tab_id: String,
}

pub trait HerdrDriver {
    fn snapshot(&mut self) -> Result<Snapshot, String>;
    fn move_pane(&mut self, pane: &str, target: &Target) -> Result<MoveResult, String>;
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
            });
        }
        let mut cmd = std::process::Command::new(&self.bin);
        cmd.args(["pane", "move", pane]);
        match target {
            Target::NewTab { label } => {
                cmd.args(["--target", "new_tab", "--label", label]);
            }
            Target::Split {
                tab,
                anchor,
                direction,
                ratio,
            } => {
                let ratio_s = format!("{ratio:.1}");
                cmd.args([
                    "--target",
                    "split",
                    "--tab",
                    tab,
                    "--anchor",
                    anchor,
                    "--direction",
                    direction,
                    "--ratio",
                    &ratio_s,
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
            .unwrap_or(serde_json::Value::Null);
        let new_pane = val
            .get("pane_id")
            .and_then(|v| v.as_str())
            .unwrap_or(pane)
            .to_string();
        let new_tab = val
            .get("tab_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(MoveResult {
            pane_id: new_pane,
            tab_id: new_tab,
        })
    }
}

/// Fake driver for testing without calling real herdr
pub struct FakeHerdrDriver {
    pub current_snapshot: Snapshot,
    pub executed_moves: Vec<(String, Target)>,
    pub next_pane_counter: u64,
    pub next_tab_counter: u64,
}

impl FakeHerdrDriver {
    pub fn new(snapshot: Snapshot) -> Self {
        FakeHerdrDriver {
            current_snapshot: snapshot,
            executed_moves: Vec::new(),
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
        })
    }
}

/// Execute layout ops via a HerdrDriver, remapping pane and tab IDs as needed.
pub fn apply_ops<D: HerdrDriver>(driver: &mut D, ops: &[Op]) -> Result<usize, String> {
    let mut pane_map: HashMap<String, String> = HashMap::new();
    let mut latest_tab: Option<String> = None;
    let mut applied_count = 0;

    for op in ops {
        if op.deferred {
            continue;
        }

        let actual_pane = pane_map.get(&op.pane).cloned().unwrap_or_else(|| op.pane.clone());
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

        let res = driver.move_pane(&actual_pane, &remapped_target)?;
        if res.pane_id != actual_pane {
            pane_map.insert(op.pane.clone(), res.pane_id.clone());
            pane_map.insert(actual_pane, res.pane_id.clone());
        }
        if matches!(op.target, Target::NewTab { .. }) && !res.tab_id.is_empty() {
            latest_tab = Some(res.tab_id);
        }
        applied_count += 1;
    }

    Ok(applied_count)
}
