//! Worker cost tracking and budget resolution via opencode API and sqlite database.

use crate::engine::activity::{ActivityResult, CostResult, ReportFileResult};
use crate::state::WorkerMetadata;
use crate::worker::cleanup::default_runs_dir;
use crate::worker::prompt::observer::Observation;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

pub fn brief_cost_budget(text: &str) -> Option<f64> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        if let Some(idx) = lower.find("cost budget:") {
            let rest = line[idx + 12..].trim();
            let num_str = rest.strip_prefix('$').unwrap_or(rest).trim();
            let num_part: String = num_str
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(val) = num_part.parse::<f64>() {
                if val > 0.0 {
                    return Some(val);
                }
            }
        }
    }
    None
}

pub fn resolve_cost_budget(task: &str) -> Option<f64> {
    if let Ok(env_val) = std::env::var("PROMPT_WORKER_COST_BUDGET") {
        let trimmed = env_val.trim();
        let num_str = trimmed.strip_prefix('$').unwrap_or(trimmed).trim();
        if let Ok(val) = num_str.parse::<f64>() {
            if val > 0.0 {
                return Some(val);
            }
        }
    }
    brief_cost_budget(task)
}

pub fn read_worker_cost(name: &str, session_id: &str, cwd: &str, harness: &str) -> Option<f64> {
    if let Ok(cost_file) = std::env::var("PROMPT_WORKER_COST_FILE") {
        if let Ok(content) = std::fs::read_to_string(&cost_file) {
            if let Ok(val) = content.trim().parse::<f64>() {
                return Some(val);
            }
        }
        return None;
    }

    if harness != "opencode" && !name.starts_with("tieude-") {
        return None;
    }

    if !session_id.is_empty() {
        let mut cmd = Command::new("opencode");
        cmd.args(["api", "GET", &format!("/api/session/{session_id}")]);

        let state_dir = default_runs_dir().join(".opencode-state").join(name);
        if state_dir.is_dir() {
            cmd.env("XDG_STATE_HOME", &state_dir);
        }
        if let Ok(out) = cmd.output() {
            if out.status.success() {
                if let Ok(json_str) = std::str::from_utf8(&out.stdout) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if let Some(c) = v
                            .get("data")
                            .and_then(|d| d.get("cost"))
                            .and_then(|c| c.as_f64())
                        {
                            return Some(c);
                        }
                    }
                }
            }
        }
    }

    let mut db_candidates = Vec::new();
    let state_db = default_runs_dir()
        .join(".opencode-state")
        .join(name)
        .join("opencode")
        .join("opencode.db");
    if state_db.is_file() {
        db_candidates.push(state_db);
    }
    if let Ok(home) = std::env::var("HOME") {
        let home_db = std::path::PathBuf::from(home).join(".local/share/opencode/opencode.db");
        if home_db.is_file() {
            db_candidates.push(home_db);
        }
    }

    for db_path in db_candidates {
        let uri = format!("file:{}?mode=ro", db_path.display());
        let query = if !cwd.is_empty() {
            format!("SELECT cost FROM session_v2 WHERE directory = '{cwd}' ORDER BY time_updated DESC LIMIT 1;")
        } else if !session_id.is_empty() {
            format!("SELECT cost FROM session_v2 WHERE id = '{session_id}' LIMIT 1;")
        } else {
            continue;
        };
        if let Ok(out) = Command::new("sqlite3").arg(&uri).arg(&query).output() {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                if let Ok(c) = text.trim().parse::<f64>() {
                    return Some(c);
                }
            }
        }
    }

    None
}

/// Tracks cost budget warnings (80% and 100%) during watchdog execution.
pub struct CostTracker {
    pub budget: Option<f64>,
    pub cost80_warned: bool,
    pub cost100_warned: bool,
}

impl CostTracker {
    pub fn new(runs_dir: &Path, name: &str, budget: Option<f64>) -> Self {
        Self {
            budget,
            cost80_warned: WorkerMetadata::has_cost80(runs_dir, name),
            cost100_warned: WorkerMetadata::has_cost100(runs_dir, name),
        }
    }

    pub fn check(&mut self, runs_dir: &Path, name: &str, obs: &Observation) {
        if let Some(budget_val) = self.budget {
            if !self.cost100_warned || !self.cost80_warned {
                if let Some(current_cost) =
                    read_worker_cost(name, &obs.session_id, &obs.cwd, &obs.harness)
                {
                    if !self.cost80_warned && current_cost >= budget_val * 0.8 {
                        let _ = WorkerMetadata::record_cost80(runs_dir, name, current_cost);
                        self.cost80_warned = true;
                        eprintln!("WATCHDOG cost 80% | name={name} cost=${current_cost:.4} budget=${budget_val:.4}");
                    }
                    if !self.cost100_warned && current_cost >= budget_val {
                        let _ = WorkerMetadata::record_cost100(runs_dir, name, current_cost);
                        self.cost100_warned = true;
                        eprintln!("WATCHDOG cost 100% | name={name} cost=${current_cost:.4} budget=${budget_val:.4}");
                    }
                }
            }
        }
    }
}

/// Budget warnings emitted during worker execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostWarning {
    /// Worker spend has reached or exceeded 80% of budget.
    Cost80,
    /// Worker spend has reached or exceeded 100% of budget.
    Cost100,
}

/// Tracks budget warning emissions for deterministic workflow state machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CostState {
    pub warned_80: bool,
    pub warned_100: bool,
}

impl CostState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Pure helper: checks spend against budget and updates cost state.
///
/// Returns `Some(CostWarning::Cost80)` when `spend >= budget * 0.8` (once).
/// Returns `Some(CostWarning::Cost100)` when `spend >= budget` (once).
///
/// If `spend >= budget` triggers first (e.g. fast jump from <80% straight past 100%),
/// it emits `Cost100` and marks both `warned_80` and `warned_100` to prevent redundant
/// late 80% warnings.
pub fn cost_warning(state: &mut CostState, spend: f64, budget: f64) -> Option<CostWarning> {
    if budget <= 0.0 || !budget.is_finite() || spend.is_nan() {
        return None;
    }
    const EPSILON: f64 = 1e-9;
    if !state.warned_100 && spend >= budget - EPSILON {
        state.warned_80 = true;
        state.warned_100 = true;
        return Some(CostWarning::Cost100);
    }
    if !state.warned_80 && spend >= (budget * 0.8) - EPSILON {
        state.warned_80 = true;
        return Some(CostWarning::Cost80);
    }
    None
}

/// Evaluates a frozen engine `CostResult` against budget.
pub fn cost_warning_from_result(
    state: &mut CostState,
    result: &CostResult,
    budget: f64,
) -> Option<CostWarning> {
    match result {
        CostResult::Spend(spend) => cost_warning(state, *spend, budget),
        CostResult::BudgetExceeded(exceeded) => {
            if *exceeded && !state.warned_100 {
                state.warned_80 = true;
                state.warned_100 = true;
                Some(CostWarning::Cost100)
            } else {
                None
            }
        }
    }
}

/// Evaluates a top-level engine `ActivityResult` against budget.
pub fn cost_warning_from_activity(
    state: &mut CostState,
    result: &ActivityResult,
    budget: f64,
) -> Option<CostWarning> {
    match result {
        ActivityResult::Cost(cost_res) => cost_warning_from_result(state, cost_res, budget),
        _ => None,
    }
}

/// Recorded sample of report file metadata for stability checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReportSample {
    pub mtime_ms: u64,
}

impl ReportSample {
    pub fn new(mtime_ms: u64) -> Self {
        Self { mtime_ms }
    }
}

/// Report file watchdog status decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportState {
    /// Report file does not exist, mtime is 0, mtime is unstable, or content is empty.
    Pending,
    /// Report file is stable and contains valid required headings.
    Ready,
    /// Report content exists and is stable, but is missing required headings.
    Malformed(String),
}

/// Checks whether builder report content contains a valid `## Lessons` section.
pub fn is_builder_report_valid(content: &str) -> bool {
    content
        .lines()
        .any(crate::worker::prompt::watchdog::is_lessons_heading)
}

/// Checks whether reviewer report content contains a valid `Verdict:` line.
pub fn is_reviewer_report_valid(content: &str) -> bool {
    content
        .lines()
        .any(|l| l.trim_start().starts_with("Verdict:"))
}

/// Pure helper: evaluates report state using builder validation (`## Lessons`).
pub fn report_state(
    prev_sample: Option<ReportSample>,
    next_result: &ReportFileResult,
    content: Option<&str>,
) -> ReportState {
    report_state_validated(prev_sample, next_result, content, false)
}

/// Configurable report state evaluation supporting builder and reviewer validation.
pub fn report_state_validated(
    prev_sample: Option<ReportSample>,
    next_result: &ReportFileResult,
    content: Option<&str>,
    expect_verdict: bool,
) -> ReportState {
    let validate_fn = if expect_verdict {
        is_reviewer_report_valid
    } else {
        is_builder_report_valid
    };
    let missing_desc = if expect_verdict {
        "missing 'Verdict:' line"
    } else {
        "missing '## Lessons' section"
    };

    match next_result {
        ReportFileResult::Exists(false) => ReportState::Pending,
        ReportFileResult::Exists(true) => {
            if let Some(c) = content {
                if c.trim().is_empty() {
                    ReportState::Pending
                } else if validate_fn(c) {
                    ReportState::Ready
                } else {
                    ReportState::Malformed(missing_desc.to_string())
                }
            } else {
                ReportState::Pending
            }
        }
        ReportFileResult::MtimeMs(mtime) => {
            if *mtime == 0 {
                return ReportState::Pending;
            }
            let is_stable = match prev_sample {
                Some(prev) => prev.mtime_ms > 0 && prev.mtime_ms == *mtime,
                None => false,
            };
            if !is_stable {
                return ReportState::Pending;
            }
            if let Some(c) = content {
                if c.trim().is_empty() {
                    ReportState::Pending
                } else if validate_fn(c) {
                    ReportState::Ready
                } else {
                    ReportState::Malformed(missing_desc.to_string())
                }
            } else {
                ReportState::Ready
            }
        }
        ReportFileResult::Content(c) => {
            if c.trim().is_empty() {
                ReportState::Pending
            } else if validate_fn(c) {
                ReportState::Ready
            } else {
                ReportState::Malformed(missing_desc.to_string())
            }
        }
        ReportFileResult::Written => ReportState::Pending,
    }
}

/// Evaluates report state from a top-level engine `ActivityResult`.
pub fn report_state_from_activity(
    prev_sample: Option<ReportSample>,
    activity_result: &ActivityResult,
    content: Option<&str>,
) -> Option<ReportState> {
    match activity_result {
        ActivityResult::ReportFile(rf) => Some(report_state(prev_sample, rf, content)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::activity::{ActivityResult, CostResult, ReportFileResult};

    #[test]
    fn test_cost_warning_below_threshold() {
        let mut state = CostState::new();
        assert_eq!(cost_warning(&mut state, 1.5, 2.0), None);
        assert!(!state.warned_80);
        assert!(!state.warned_100);
    }

    #[test]
    fn test_cost_warning_80_boundary_and_once() {
        let mut state = CostState::new();
        // 1.6 / 2.0 == 80%
        let res = cost_warning(&mut state, 1.6, 2.0);
        assert_eq!(res, Some(CostWarning::Cost80));
        assert!(state.warned_80);
        assert!(!state.warned_100);

        // Subsequent call between 80% and 100% returns None
        assert_eq!(cost_warning(&mut state, 1.8, 2.0), None);
        assert_eq!(cost_warning(&mut state, 1.99, 2.0), None);
    }

    #[test]
    fn test_cost_warning_100_boundary_and_once() {
        let mut state = CostState::new();
        state.warned_80 = true;
        // 2.0 / 2.0 == 100%
        let res = cost_warning(&mut state, 2.0, 2.0);
        assert_eq!(res, Some(CostWarning::Cost100));
        assert!(state.warned_100);

        // Subsequent call > 100% returns None
        assert_eq!(cost_warning(&mut state, 2.5, 2.0), None);
    }

    #[test]
    fn test_cost_warning_leapfrog_100() {
        let mut state = CostState::new();
        // Jump from <80% straight past 100%
        let res = cost_warning(&mut state, 2.5, 2.0);
        assert_eq!(res, Some(CostWarning::Cost100));
        assert!(state.warned_80);
        assert!(state.warned_100);

        // Ensure 80% is not fired later
        assert_eq!(cost_warning(&mut state, 2.6, 2.0), None);
    }

    #[test]
    fn test_cost_warning_invalid_budget() {
        let mut state = CostState::new();
        assert_eq!(cost_warning(&mut state, 1.0, 0.0), None);
        assert_eq!(cost_warning(&mut state, 1.0, -5.0), None);
        assert_eq!(cost_warning(&mut state, 1.0, f64::NAN), None);
        assert_eq!(cost_warning(&mut state, 1.0, f64::INFINITY), None);
        assert_eq!(cost_warning(&mut state, f64::NAN, 2.0), None);
        assert!(!state.warned_80);
        assert!(!state.warned_100);
    }

    #[test]
    fn test_cost_warning_from_activity_result() {
        let mut state = CostState::new();
        let act80 = ActivityResult::Cost(CostResult::Spend(2.4));
        let act100 = ActivityResult::Cost(CostResult::Spend(3.0));

        assert_eq!(
            cost_warning_from_activity(&mut state, &act80, 3.0),
            Some(CostWarning::Cost80)
        );
        assert_eq!(cost_warning_from_activity(&mut state, &act80, 3.0), None);
        assert_eq!(
            cost_warning_from_activity(&mut state, &act100, 3.0),
            Some(CostWarning::Cost100)
        );
        assert_eq!(cost_warning_from_activity(&mut state, &act100, 3.0), None);

        // Non-cost activity returns None
        let other = ActivityResult::ReportFile(ReportFileResult::Exists(true));
        assert_eq!(cost_warning_from_activity(&mut state, &other, 3.0), None);
    }

    #[test]
    fn test_report_state_mtime_stability() {
        // First sample, no prev
        let res1 = ReportFileResult::MtimeMs(1000);
        assert_eq!(report_state(None, &res1, None), ReportState::Pending);

        // Second sample, mtime changed (unstable)
        let prev1 = Some(ReportSample::new(1000));
        let res2 = ReportFileResult::MtimeMs(1200);
        assert_eq!(report_state(prev1, &res2, None), ReportState::Pending);

        // Third sample, mtime equal (stable)
        let prev2 = Some(ReportSample::new(1200));
        let res3 = ReportFileResult::MtimeMs(1200);
        assert_eq!(report_state(prev2, &res3, None), ReportState::Ready);

        // Mtime 0 is never ready
        let prev0 = Some(ReportSample::new(0));
        let res0 = ReportFileResult::MtimeMs(0);
        assert_eq!(report_state(prev0, &res0, None), ReportState::Pending);
    }

    #[test]
    fn test_report_state_content_lessons_valid() {
        let valid_builder = "# Run Summary\n\n## Outcome\nDone\n\n## Lessons\nnone\n";
        let res = ReportFileResult::Content(valid_builder.to_string());
        assert_eq!(report_state(None, &res, None), ReportState::Ready);

        let valid_numbered = "# Run Summary\n\n## 5. Lessons\n- Note\n";
        let res2 = ReportFileResult::Content(valid_numbered.to_string());
        assert_eq!(report_state(None, &res2, None), ReportState::Ready);
    }

    #[test]
    fn test_report_state_content_lessons_malformed() {
        let missing_lessons = "# Run Summary\n\n## Outcome\nDone\n\n## Changes\nNone\n";
        let res = ReportFileResult::Content(missing_lessons.to_string());
        assert_eq!(
            report_state(None, &res, None),
            ReportState::Malformed("missing '## Lessons' section".to_string())
        );

        // Subheading ### Lessons does not count
        let deep_heading = "# Run Summary\n\n### Lessons\nnone\n";
        let res2 = ReportFileResult::Content(deep_heading.to_string());
        assert_eq!(
            report_state(None, &res2, None),
            ReportState::Malformed("missing '## Lessons' section".to_string())
        );
    }

    #[test]
    fn test_report_state_empty_content() {
        let empty = ReportFileResult::Content("   \n\t  ".to_string());
        assert_eq!(report_state(None, &empty, None), ReportState::Pending);
    }

    #[test]
    fn test_report_state_exists_variant() {
        assert_eq!(
            report_state(None, &ReportFileResult::Exists(false), None),
            ReportState::Pending
        );
        assert_eq!(
            report_state(None, &ReportFileResult::Exists(true), None),
            ReportState::Pending
        );
        let valid = "## Lessons\nnone";
        assert_eq!(
            report_state(None, &ReportFileResult::Exists(true), Some(valid)),
            ReportState::Ready
        );
        let invalid = "## Summary\nDone";
        assert_eq!(
            report_state(None, &ReportFileResult::Exists(true), Some(invalid)),
            ReportState::Malformed("missing '## Lessons' section".to_string())
        );
    }

    #[test]
    fn test_report_state_reviewer_validation() {
        let pass_review = "Overall thoughts\n\nVerdict: PASS\n";
        let res = ReportFileResult::Content(pass_review.to_string());
        assert_eq!(
            report_state_validated(None, &res, None, true),
            ReportState::Ready
        );

        let fail_review = "Overall thoughts\n\nNo verdict provided\n";
        let res2 = ReportFileResult::Content(fail_review.to_string());
        assert_eq!(
            report_state_validated(None, &res2, None, true),
            ReportState::Malformed("missing 'Verdict:' line".to_string())
        );
    }

    #[test]
    fn test_report_state_from_activity_result() {
        let valid = "# Report\n\n## Lessons\nnone";
        let act = ActivityResult::ReportFile(ReportFileResult::Content(valid.to_string()));
        assert_eq!(
            report_state_from_activity(None, &act, None),
            Some(ReportState::Ready)
        );

        let non_report = ActivityResult::Cost(CostResult::Spend(1.0));
        assert_eq!(report_state_from_activity(None, &non_report, None), None);
    }
}
