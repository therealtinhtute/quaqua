//! Standardized state and metadata management for workers and execution runs.
//! Provides atomic reads and writes for worker metadata and execution states.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Atomically write `content` to `path` via a temporary file in the same directory.
pub fn atomic_write(path: &Path, content: impl AsRef<[u8]>) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp_path = parent.join(format!(".{file_name}.tmp.{}", std::process::id()));
    fs::write(&tmp_path, content)?;
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

/// Metadata operations and paths for task workers in `runs_dir`.
pub struct WorkerMetadata;

impl WorkerMetadata {
    pub fn owner_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!("{name}.owner"))
    }

    pub fn grid_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!("{name}.grid"))
    }

    pub fn pipeline_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!("{name}.pipeline"))
    }

    pub fn srt_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!("{name}.srt.json"))
    }

    pub fn budget_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!(".{name}.budget"))
    }

    pub fn grace_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!(".{name}.grace"))
    }

    pub fn autoext_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!(".{name}.autoext"))
    }

    pub fn cost80_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!(".{name}.cost80"))
    }

    pub fn cost100_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!(".{name}.cost100"))
    }

    pub fn provider_retry_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!(".{name}.provider_retry"))
    }

    pub fn progress_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!(".{name}.progress"))
    }

    pub fn elapsed_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!(".{name}.elapsed"))
    }

    pub fn nudged_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!(".{name}.nudged"))
    }

    pub fn submit_log_path(runs_dir: &Path, name: &str) -> PathBuf {
        runs_dir.join(format!(".{name}.submit.log"))
    }

    pub fn load_owner(runs_dir: &Path, name: &str) -> String {
        fs::read_to_string(Self::owner_path(runs_dir, name))
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "cos".to_string())
    }

    pub fn save_owner(runs_dir: &Path, name: &str, owner: &str) -> io::Result<()> {
        atomic_write(&Self::owner_path(runs_dir, name), format!("{owner}\n"))
    }

    pub fn load_grid(runs_dir: &Path, name: &str) -> Option<String> {
        fs::read_to_string(Self::grid_path(runs_dir, name))
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
    }

    pub fn save_grid(runs_dir: &Path, name: &str, grid: &str) -> io::Result<()> {
        atomic_write(&Self::grid_path(runs_dir, name), format!("{grid}\n"))
    }

    pub fn load_budget(runs_dir: &Path, name: &str) -> Option<u64> {
        fs::read_to_string(Self::budget_path(runs_dir, name))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    pub fn save_budget(runs_dir: &Path, name: &str, budget: u64) -> io::Result<()> {
        atomic_write(&Self::budget_path(runs_dir, name), format!("{budget}\n"))
    }

    pub fn has_grace(runs_dir: &Path, name: &str) -> bool {
        Self::grace_path(runs_dir, name).exists()
    }

    pub fn record_grace(runs_dir: &Path, name: &str) -> io::Result<()> {
        atomic_write(&Self::grace_path(runs_dir, name), "")
    }

    pub fn load_autoext(runs_dir: &Path, name: &str) -> u64 {
        fs::read_to_string(Self::autoext_path(runs_dir, name))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    pub fn save_autoext(runs_dir: &Path, name: &str, total: u64) -> io::Result<()> {
        atomic_write(&Self::autoext_path(runs_dir, name), format!("{total}\n"))
    }

    pub fn has_cost80(runs_dir: &Path, name: &str) -> bool {
        Self::cost80_path(runs_dir, name).exists()
    }

    pub fn record_cost80(runs_dir: &Path, name: &str, cost: f64) -> io::Result<()> {
        atomic_write(&Self::cost80_path(runs_dir, name), format!("{cost}\n"))
    }

    pub fn has_cost100(runs_dir: &Path, name: &str) -> bool {
        Self::cost100_path(runs_dir, name).exists()
    }

    pub fn record_cost100(runs_dir: &Path, name: &str, cost: f64) -> io::Result<()> {
        atomic_write(&Self::cost100_path(runs_dir, name), format!("{cost}\n"))
    }

    pub fn has_provider_retry(runs_dir: &Path, name: &str) -> bool {
        Self::provider_retry_path(runs_dir, name).exists()
    }

    pub fn record_provider_retry(runs_dir: &Path, name: &str, ts: u64) -> io::Result<()> {
        atomic_write(&Self::provider_retry_path(runs_dir, name), format!("{ts}\n"))
    }

    pub fn save_pipeline(runs_dir: &Path, name: &str, task: &str) -> io::Result<()> {
        atomic_write(&Self::pipeline_path(runs_dir, name), format!("{task}\n"))
    }

    pub fn save_srt(runs_dir: &Path, name: &str, srt_json: &str) -> io::Result<()> {
        atomic_write(&Self::srt_path(runs_dir, name), srt_json)
    }

    /// Reset transient watchdog state files when initiating a fresh prompt.
    pub fn reset_prompt_state(runs_dir: &Path, name: &str) {
        let _ = fs::remove_file(Self::elapsed_path(runs_dir, name));
        let _ = fs::remove_file(Self::nudged_path(runs_dir, name));
        let _ = fs::remove_file(Self::grace_path(runs_dir, name));
        let _ = fs::remove_file(Self::progress_path(runs_dir, name));
        let _ = fs::remove_file(Self::provider_retry_path(runs_dir, name));
        let _ = fs::remove_file(Self::autoext_path(runs_dir, name));
        let _ = fs::remove_file(Self::cost80_path(runs_dir, name));
        let _ = fs::remove_file(Self::cost100_path(runs_dir, name));
    }

    /// Clean up all metadata and transient state files for a worker.
    /// Ensures `.budget`, `.grace`, `.cost80`, `.cost100`, `.owner`, `.grid`,
    /// `.pipeline`, `.srt.json`, `.autoext`, `.provider_retry`, etc. are all deleted.
    pub fn cleanup_worker_files(runs_dir: &Path, name: &str) {
        let _ = fs::remove_file(Self::owner_path(runs_dir, name));
        let _ = fs::remove_file(Self::grid_path(runs_dir, name));
        let _ = fs::remove_file(Self::pipeline_path(runs_dir, name));
        let _ = fs::remove_file(Self::srt_path(runs_dir, name));
        let _ = fs::remove_file(Self::provider_retry_path(runs_dir, name));
        let _ = fs::remove_file(Self::autoext_path(runs_dir, name));
        let _ = fs::remove_file(Self::cost80_path(runs_dir, name));
        let _ = fs::remove_file(Self::cost100_path(runs_dir, name));
        let _ = fs::remove_file(Self::budget_path(runs_dir, name));
        let _ = fs::remove_file(Self::grace_path(runs_dir, name));
        let _ = fs::remove_file(Self::progress_path(runs_dir, name));
        let _ = fs::remove_file(Self::elapsed_path(runs_dir, name));
        let _ = fs::remove_file(Self::nudged_path(runs_dir, name));
        let _ = fs::remove_file(Self::submit_log_path(runs_dir, name));
        let _ = fs::remove_dir_all(runs_dir.join(".opencode-state").join(name));
    }
}

/// Execution state parsed from `pipeline-*.state` and `taskforce-*.state` files.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionState {
    Pipeline(PipelineExecutionState),
    Taskforce(TaskforceExecutionState),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PipelineExecutionState {
    pub task: String,
    pub lead: String,
    pub pipeline_pid: String,
    pub builder: String,
    pub finished: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskforceExecutionState {
    pub task: String,
    pub lead: String,
    pub pid: String,
    pub kind: String,
    pub finished: bool,
}

impl PipelineExecutionState {
    pub fn owner(&self) -> &str {
        if self.lead.is_empty() {
            "cos"
        } else {
            &self.lead
        }
    }
}

impl TaskforceExecutionState {
    pub fn owner(&self) -> &str {
        if self.lead.is_empty() {
            "cos"
        } else {
            &self.lead
        }
    }
}

impl ExecutionState {
    pub fn load(path: &Path) -> Option<Self> {
        let file_name = path.file_name().and_then(|n| n.to_str())?;
        let is_pipeline = file_name.starts_with("pipeline-") && file_name.ends_with(".state");
        let is_taskforce = file_name.ends_with(".state")
            && (file_name.starts_with("taskforce-")
                || file_name.starts_with("dual-lens-")
                || file_name.starts_with("advisor-"));

        if !is_pipeline && !is_taskforce {
            return None;
        }

        let content = fs::read_to_string(path).ok()?;

        if is_pipeline {
            let mut state = PipelineExecutionState::default();
            for line in content.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("lead=") {
                    state.lead = v.to_string();
                } else if let Some(v) = line.strip_prefix("pipeline_pid=") {
                    state.pipeline_pid = v.to_string();
                } else if let Some(v) = line.strip_prefix("builder=") {
                    state.builder = v.to_string();
                } else if let Some(v) = line.strip_prefix("task=") {
                    state.task = v.to_string();
                } else if let Some(v) = line.strip_prefix("finished=") {
                    state.finished = v == "1" || v == "true";
                }
            }
            if state.task.is_empty() {
                state.task = file_name
                    .strip_prefix("pipeline-")
                    .and_then(|n| n.strip_suffix(".state"))
                    .unwrap_or("")
                    .to_string();
            }
            Some(ExecutionState::Pipeline(state))
        } else {
            let mut state = TaskforceExecutionState::default();
            for line in content.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("lead=").or_else(|| line.strip_prefix("owner=")) {
                    state.lead = v.to_string();
                } else if let Some(v) = line.strip_prefix("taskforce_pid=").or_else(|| line.strip_prefix("pid=")) {
                    state.pid = v.to_string();
                } else if let Some(v) = line.strip_prefix("kind=").or_else(|| line.strip_prefix("taskforce=")) {
                    state.kind = v.to_string();
                } else if let Some(v) = line.strip_prefix("task=") {
                    state.task = v.to_string();
                } else if let Some(v) = line.strip_prefix("finished=") {
                    state.finished = v == "1" || v == "true";
                }
            }
            Some(ExecutionState::Taskforce(state))
        }
    }

    pub fn is_finished(&self) -> bool {
        match self {
            ExecutionState::Pipeline(p) => p.finished,
            ExecutionState::Taskforce(t) => t.finished,
        }
    }

    pub fn owner(&self) -> &str {
        let lead = match self {
            ExecutionState::Pipeline(p) => &p.lead,
            ExecutionState::Taskforce(t) => &t.lead,
        };
        if lead.is_empty() {
            "cos"
        } else {
            lead
        }
    }
}
