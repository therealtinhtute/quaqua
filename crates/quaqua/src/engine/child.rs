//! Child workflow contracts.
//!
//! Exposes input and output structures for executing isolated worker tasks as child workflows.

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// Input parameters for starting a single task worker child workflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SingleWorkerInput {
    pub name: String,
    pub kind: String,
    pub repo: String,
    pub brief: String,
    pub budget_sec: u64,
    pub cost_budget: Option<f64>,
}

impl SingleWorkerInput {
    pub fn new(
        name: impl Into<String>,
        kind: impl Into<String>,
        repo: impl Into<String>,
        brief: impl Into<String>,
        budget_sec: u64,
        cost_budget: Option<f64>,
    ) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            repo: repo.into(),
            brief: brief.into(),
            budget_sec,
            cost_budget,
        }
    }
}

/// Output returned by a single task worker child workflow upon completion.
///
/// Exit code conventions match the quaqua watchdog protocol:
/// - `0`: Success (run file printed and verified)
/// - `2`: Hard timeout / max time exceeded
/// - `3`: Stall / idle / agent gone
/// - `4`: Time budget exceeded
/// - `5`: Blocked on question / permission
/// - `6`: Malformed report
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SingleWorkerOutput {
    pub exit: i32,
    pub report_path: PathBuf,
}

impl SingleWorkerOutput {
    /// Validates and constructs a [`SingleWorkerOutput`].
    /// Returns error if exit code is not within 0 or 2..=6.
    pub fn new(exit: i32, report_path: impl Into<PathBuf>) -> Result<Self, String> {
        let valid = exit == 0 || (2..=6).contains(&exit);
        if !valid {
            return Err(format!("invalid child worker exit code {exit}; expected 0 or 2..=6"));
        }
        Ok(Self {
            exit,
            report_path: report_path.into(),
        })
    }
}
