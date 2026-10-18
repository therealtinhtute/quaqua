//! Worker cleanup logic (`quaqua worker cleanup <name> <repo-path>`).
//! Reproduces `scripts/worker/cleanup-worker.sh`.

use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::state::WorkerMetadata;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn resolve_home_quaqua_runs(home: Option<&Path>) -> PathBuf {
    crate::workspace::resolve_workspace_root(None, home).join("runs")
}

pub fn resolve_default_runs_dir(override_runs: Option<&Path>, home: Option<&Path>) -> PathBuf {
    if let Some(dir) = override_runs {
        dir.to_path_buf()
    } else {
        resolve_home_quaqua_runs(home)
    }
}

pub fn home_quaqua_runs() -> PathBuf {
    crate::workspace::workspace_root().join("runs")
}

pub fn default_runs_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("QUAQUA_RUNS_DIR") {
        return PathBuf::from(dir);
    }
    home_quaqua_runs()
}


pub fn run_grid_hook(grid_tab: &str, owner: Option<&str>) -> Result<(), (i32, String)> {
    if std::env::var("GRID_SKIP").as_deref() == Ok("1") {
        let is_pipeline = std::env::var("PIPELINE_TASK").is_ok();
        let is_test = std::env::var("QUAQUA_TEST").as_deref() == Ok("1");
        if is_pipeline || is_test {
            return Ok(());
        }
        eprintln!("warning: GRID_SKIP=1 ignored outside pipeline.sh/tests (rules/_all/fanout-agent-grid.md)");
    }

    if let Err(e) = crate::grid::run_grid_repair(grid_tab, owner) {
        return Err((1, format!("error: grid repair failed: {e}\n")));
    }

    if let Err(e) = crate::grid::run_grid_check(grid_tab, None, owner) {
        return Err((1, format!("error: grid check failed ({e})\n")));
    }

    Ok(())
}

pub fn cleanup_worker<G: GitOps, H: HerdrOps>(name: &str, repo_path: &Path, git: &G, herdr: &H) -> Result<(), (i32, String)> {
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

    let parsed_json = serde_json::from_str::<Value>(&agent_info).ok();
    let mut pane_to_close: Option<String> = None;
    let mut inspect_ok = false;

    if let Some(json) = &parsed_json {
        if let Some(agent) = json.get("result").and_then(|r| r.get("agent")) {
            inspect_ok = true;
            if let Some(pane) = agent.get("pane_id").and_then(|p| p.as_str()) {
                if !pane.is_empty() {
                    pane_to_close = Some(pane.to_string());
                }
            }
        } else if let Some(err) = json.get("error") {
            if err.get("code").and_then(|c| c.as_str()) == Some("agent_not_found") {
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

    let runs_dir = default_runs_dir();

    let owner = std::env::var("QUAQUA_OWNER")
        .ok()
        .unwrap_or_else(|| WorkerMetadata::load_owner(&runs_dir, name));

    let repo_name = repo
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo");
    let grid_tab = std::env::var("GRID_TAB")
        .ok()
        .or_else(|| WorkerMetadata::load_grid(&runs_dir, name))
        .unwrap_or_else(|| format!("{repo_name}-workers"));

    let rm_out = git
        .worktree_remove(&repo, &wt)
        .map_err(|e| (1, format!("error: worktree remove failed: {e}")))?;

    if !rm_out.status.success() {
        let err_text = String::from_utf8_lossy(&rm_out.stderr).to_string();
        return Err((rm_out.status.code().unwrap_or(1), err_text));
    }

    WorkerMetadata::cleanup_worker_files(&runs_dir, name);
    println!("cleaned {name} (branch quaqua/{name} kept)");

    run_grid_hook(&grid_tab, Some(&owner))?;
    Ok(())
}
