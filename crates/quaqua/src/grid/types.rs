use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub panes: Vec<Pane>,
    #[serde(default)]
    pub layouts: Vec<Layout>,
    #[serde(default)]
    pub agents: Vec<Agent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub workspace_id: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub tab_id: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub pane_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    #[serde(default)]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: Option<String>,
    #[serde(default)]
    pub tokens: Option<Tokens>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    #[serde(default)]
    pub grid_role: Option<String>,
    #[serde(default)]
    pub grid_owner: Option<String>,
    #[serde(default)]
    pub grid_group: Option<String>,
    #[serde(default)]
    pub grid_ready: Option<serde_json::Value>,
    #[serde(default)]
    pub grid_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub tab_id: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub area: Option<Rect>,
    #[serde(default)]
    pub panes: Vec<PaneLayout>,
    #[serde(default)]
    pub zoomed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneLayout {
    pub pane_id: String,
    #[serde(default)]
    pub rect: Option<Rect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub pane_id: Option<String>,
    #[serde(default)]
    pub agent_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Op {
    pub op: String,
    pub pane: String,
    pub deferred: bool,
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Target {
    #[serde(rename = "new_tab")]
    NewTab {
        label: String,
    },
    #[serde(rename = "split")]
    Split {
        tab: String,
        anchor: String,
        direction: String,
        ratio: f64,
    },
}

pub fn parse_snapshot_value(val: &serde_json::Value) -> Result<Snapshot, String> {
    if let Some(res) = val.get("result").and_then(|r| r.get("snapshot")) {
        serde_json::from_value(res.clone()).map_err(|e| format!("cannot parse snapshot: {e}"))
    } else if let Some(snap) = val.get("snapshot") {
        serde_json::from_value(snap.clone()).map_err(|e| format!("cannot parse snapshot: {e}"))
    } else {
        serde_json::from_value(val.clone()).map_err(|e| format!("cannot parse snapshot: {e}"))
    }
}
