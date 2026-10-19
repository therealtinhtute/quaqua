//! Pipeline execution state persistence (`PipelineState`).

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct PipelineState {
    pub repo: String,
    pub brief: String,
    pub builder: String,
    pub reviewer: String,
    pub review: String,
    pub rounds: u32,
    pub base: String,
    pub wt: String,
    pub base_sha: String,
    pub head_sha: String,
    pub planfile: String,
    pub commit: String,
    pub phase: String,
    pub round: u32,
    pub prompted: u32,
    pub prompt_ts: u64,
    pub rounds_run: u32,
    pub review_run: String,
    pub review_changed: bool,
    pub last_findings: u32,
    pub finished: u32,
    pub start_elapsed: u64,
    pub builder_spawned: u32,
    pub reviewer_spawned: u32,
    pub lead: String,
    pub lead_pane: String,
    pub pipeline_pid: u32,
}

impl PipelineState {
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut s = PipelineState::default();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(idx) = line.find('=') {
                let key = &line[..idx];
                let mut val = &line[idx + 1..];
                // strip quotes
                if (val.starts_with('\'') && val.ends_with('\''))
                    || (val.starts_with('"') && val.ends_with('"'))
                {
                    val = &val[1..val.len() - 1];
                }

                match key {
                    "repo" => s.repo = val.to_string(),
                    "brief" => s.brief = val.to_string(),
                    "builder" => s.builder = val.to_string(),
                    "reviewer" => s.reviewer = val.to_string(),
                    "review" => s.review = val.to_string(),
                    "rounds" => s.rounds = val.parse().unwrap_or(2),
                    "base" => s.base = val.to_string(),
                    "wt" => s.wt = val.to_string(),
                    "base_sha" => s.base_sha = val.to_string(),
                    "head_sha" => s.head_sha = val.to_string(),
                    "planfile" => s.planfile = val.to_string(),
                    "commit" => s.commit = val.to_string(),
                    "phase" => s.phase = val.to_string(),
                    "round" => s.round = val.parse().unwrap_or(0),
                    "prompted" => s.prompted = val.parse().unwrap_or(0),
                    "prompt_ts" => s.prompt_ts = val.parse().unwrap_or(0),
                    "rounds_run" => s.rounds_run = val.parse().unwrap_or(0),
                    "review_run" => s.review_run = val.to_string(),
                    "review_changed" => s.review_changed = val == "true" || val == "1",
                    "last_findings" => s.last_findings = val.parse().unwrap_or(0),
                    "finished" => s.finished = val.parse().unwrap_or(0),
                    "start_elapsed" => s.start_elapsed = val.parse().unwrap_or(0),
                    "builder_spawned" => s.builder_spawned = val.parse().unwrap_or(0),
                    "reviewer_spawned" => s.reviewer_spawned = val.parse().unwrap_or(0),
                    "lead" => s.lead = val.to_string(),
                    "lead_pane" => s.lead_pane = val.to_string(),
                    "pipeline_pid" => s.pipeline_pid = val.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        Ok(s)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut out = String::new();
        out.push_str(&format!("repo={}\n", self.repo));
        out.push_str(&format!("brief={}\n", self.brief));
        out.push_str(&format!("builder={}\n", self.builder));
        out.push_str(&format!("reviewer={}\n", self.reviewer));
        out.push_str(&format!("review={}\n", self.review));
        out.push_str(&format!("rounds={}\n", self.rounds));
        out.push_str(&format!("base={}\n", self.base));
        out.push_str(&format!("wt={}\n", self.wt));
        out.push_str(&format!("base_sha={}\n", self.base_sha));
        out.push_str(&format!("head_sha={}\n", self.head_sha));
        out.push_str(&format!("planfile={}\n", self.planfile));
        out.push_str(&format!("commit={}\n", self.commit));
        out.push_str(&format!("phase={}\n", self.phase));
        out.push_str(&format!("round={}\n", self.round));
        out.push_str(&format!("prompted={}\n", self.prompted));
        out.push_str(&format!("prompt_ts={}\n", self.prompt_ts));
        out.push_str(&format!("rounds_run={}\n", self.rounds_run));
        out.push_str(&format!("review_run={}\n", self.review_run));
        out.push_str(&format!(
            "review_changed={}\n",
            if self.review_changed { "true" } else { "false" }
        ));
        out.push_str(&format!("last_findings={}\n", self.last_findings));
        out.push_str(&format!("finished={}\n", self.finished));
        out.push_str(&format!("start_elapsed={}\n", self.start_elapsed));
        out.push_str(&format!("builder_spawned={}\n", self.builder_spawned));
        out.push_str(&format!("reviewer_spawned={}\n", self.reviewer_spawned));
        out.push_str(&format!("lead={}\n", self.lead));
        out.push_str(&format!("lead_pane={}\n", self.lead_pane));
        out.push_str(&format!("pipeline_pid={}\n", self.pipeline_pid));
        std::fs::write(path, out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelinePhase {
    Plan,
    PlanReadyGate,
    Build,
    Review,
    Fix,
    Complete,
    Failed,
    Disputed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStrategy {
    Auto,
    Always,
    Skip,
}

fn default_review_strategy() -> ReviewStrategy {
    ReviewStrategy::Auto
}
fn default_max_rounds() -> u32 {
    2
}
fn default_plan_budget() -> u64 {
    900
}
fn default_build_budget() -> u64 {
    2700
}
fn default_review_budget() -> u64 {
    900
}
fn default_fix_budget() -> u64 {
    1200
}
fn default_base() -> String {
    "main".to_string()
}
fn default_brief_path() -> String {
    "brief.md".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineInput {
    pub task: String,
    pub repo: String,
    pub builder: String,
    pub reviewer: String,
    pub brief: String,
    #[serde(default = "default_brief_path")]
    pub brief_path: String,
    #[serde(default = "default_base")]
    pub base: String,
    #[serde(default = "default_review_strategy")]
    pub review_strategy: ReviewStrategy,
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
    #[serde(default = "default_plan_budget")]
    pub plan_budget_sec: u64,
    #[serde(default = "default_build_budget")]
    pub build_budget_sec: u64,
    #[serde(default = "default_review_budget")]
    pub review_budget_sec: u64,
    #[serde(default = "default_fix_budget")]
    pub fix_budget_sec: u64,
    #[serde(default)]
    pub base_sha: Option<String>,
}

impl PipelineInput {
    pub fn new(
        task: impl Into<String>,
        repo: impl Into<String>,
        builder: impl Into<String>,
        reviewer: impl Into<String>,
        brief: impl Into<String>,
    ) -> Self {
        Self {
            task: task.into(),
            repo: repo.into(),
            builder: builder.into(),
            reviewer: reviewer.into(),
            brief: brief.into(),
            brief_path: "brief.md".to_string(),
            base: "main".to_string(),
            review_strategy: ReviewStrategy::Auto,
            max_rounds: 2,
            plan_budget_sec: 900,
            build_budget_sec: 2700,
            review_budget_sec: 900,
            fix_budget_sec: 1200,
            base_sha: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineWorkflowState {
    pub input: PipelineInput,
    pub phase: PipelinePhase,
    pub round: u32,
    pub rounds_run: u32,
    pub last_findings: u32,
    pub review_changed: bool,
    pub planfile: Option<String>,
    pub head_sha: Option<String>,
    pub active_child_id: Option<String>,
    pub pending_report_read: Option<String>,
    pub last_review_report: Option<String>,
    #[serde(default)]
    pub review_skip: bool,
}

impl PipelineWorkflowState {
    pub fn new(input: PipelineInput) -> Self {
        Self {
            input,
            phase: PipelinePhase::Plan,
            round: 0,
            rounds_run: 0,
            last_findings: 0,
            review_changed: false,
            planfile: None,
            head_sha: None,
            active_child_id: None,
            pending_report_read: None,
            last_review_report: None,
            review_skip: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineOutput {
    pub status: String,
    pub rounds: u32,
    pub review_changed: bool,
    pub commit: Option<String>,
}
