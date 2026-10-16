//! Worker cleanup logic (`quaqua worker cleanup <name> <repo-path>`).
//! Reproduces `scripts/worker/cleanup-worker.sh`.

use crate::git::Git;
use crate::herdr::Herdr;
use crate::json::parse_json;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn default_runs_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("QUAQUA_RUNS_DIR") {
        PathBuf::from(dir)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join("quaqua/runs")
    } else {
        PathBuf::from("runs")
    }
}

pub fn run_grid_hook(repo: &Path, grid_tab: &str) {
    if std::env::var("GRID_SKIP").as_deref() == Ok("1") {
        let is_pipeline = std::env::var("PIPELINE_TASK").is_ok();
        let is_test = std::env::var("QUAQUA_TEST").as_deref() == Ok("1");
        if is_pipeline || is_test {
            return;
        }
        eprintln!("warning: GRID_SKIP=1 ignored outside pipeline.sh/tests (rules/_all/fanout-agent-grid.md)");
    }

    let mut grid_script: Option<PathBuf> = None;
    if let Ok(dir) = std::env::var("QUAQUA_SCRIPT_DIR") {
        let candidate = PathBuf::from(dir).join("grid-workers.sh");
        if candidate.is_file() {
            grid_script = Some(candidate);
        }
    }
    if grid_script.is_none() {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let candidate = dir.join("grid-workers.sh");
                if candidate.is_file() {
                    grid_script = Some(candidate);
                }
            }
        }
    }
    if grid_script.is_none() {
        let candidate = repo.join("scripts/worker/grid-workers.sh");
        if candidate.is_file() {
            grid_script = Some(candidate);
        }
    }
    if grid_script.is_none() {
        let candidate = PathBuf::from("scripts/worker/grid-workers.sh");
        if candidate.is_file() {
            grid_script = Some(candidate);
        }
    }

    if let Some(script) = grid_script {
        let res = Command::new("bash")
            .arg(script)
            .arg("--check")
            .arg("--tab")
            .arg(grid_tab)
            .status();
        match res {
            Ok(st) if !st.success() => {
                eprintln!(
                    "warning: grid-workers.sh failed (exit {}); arrange panes manually",
                    st.code().unwrap_or(1)
                );
            }
            Err(e) => {
                eprintln!("warning: grid-workers.sh failed: {e}; arrange panes manually");
            }
            _ => {}
        }
    }
}

pub fn cleanup_worker(name: &str, repo_path: &Path, git: &Git, herdr: &Herdr) -> Result<(), (i32, String)> {
    let repo = match std::fs::canonicalize(repo_path) {
        Ok(p) => p,
        Err(_) => repo_path.to_path_buf(),
    };

    let wt = match repo.parent() {
        Some(parent) => parent.join(".wt").join(name),
        None => PathBuf::from(".wt").join(name),
    };

    let (_get_ok, agent_info) = herdr
        .agent_get(name)
        .map_err(|e| (1, format!("error: failed to inspect agent {name}: {e}")))?;

    let parsed_json = parse_json(&agent_info).ok();
    let mut pane_to_close: Option<String> = None;
    let mut inspect_ok = false;

    if let Some(json) = &parsed_json {
        if let Some(agent) = json.get("result").and_then(|r| r.get("agent")) {
            inspect_ok = true;
            if let Some(pane) = agent.get_str("pane_id") {
                if !pane.is_empty() {
                    pane_to_close = Some(pane.to_string());
                }
            }
        } else if let Some(err) = json.get("error") {
            if err.get_str("code") == Some("agent_not_found") {
                inspect_ok = true; // agent already gone: no pane to close
            }
        }
    }

    if !inspect_ok && agent_info.contains("agent_not_found") {
        inspect_ok = true;
    }

    if !inspect_ok {
        return Err((1, format!("error: failed to inspect agent {name}: {agent_info}")));
    }

    if let Some(pane) = pane_to_close {
        let _ = herdr.pane_close(&pane);
    }

    let repo_name = repo
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo");
    let grid_tab = std::env::var("GRID_TAB").unwrap_or_else(|_| format!("{repo_name}-workers"));

    run_grid_hook(&repo, &grid_tab);

    let rm_out = git
        .worktree_remove(&repo, &wt)
        .map_err(|e| (1, format!("error: worktree remove failed: {e}")))?;

    if !rm_out.status.success() {
        let err_text = String::from_utf8_lossy(&rm_out.stderr).to_string();
        return Err((rm_out.status.code().unwrap_or(1), err_text));
    }

    let runs_dir = default_runs_dir();
    let _ = std::fs::remove_file(runs_dir.join(format!("{name}.owner")));
    let _ = std::fs::remove_file(runs_dir.join(format!("{name}.grid")));
    let _ = std::fs::remove_file(runs_dir.join(format!("{name}.pipeline")));
    let _ = std::fs::remove_file(runs_dir.join(format!("{name}.srt.json")));

    println!("cleaned {name} (branch quaqua/{name} kept)");
    Ok(())
}
