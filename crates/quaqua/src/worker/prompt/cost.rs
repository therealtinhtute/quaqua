//! Worker cost tracking and budget resolution via opencode API and sqlite database.

use crate::state::WorkerMetadata;
use crate::worker::cleanup::default_runs_dir;
use crate::worker::prompt::observer::Observation;
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
                        if let Some(c) = v.get("data").and_then(|d| d.get("cost")).and_then(|c| c.as_f64()) {
                            return Some(c);
                        }
                    }
                }
            }
        }
    }

    let mut db_candidates = Vec::new();
    let state_db = default_runs_dir().join(".opencode-state").join(name).join("opencode").join("opencode.db");
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
                if let Some(current_cost) = read_worker_cost(name, &obs.session_id, &obs.cwd, &obs.harness) {
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
