//! Worker cleanup logic (`quaqua worker cleanup <name> <repo-path>`).
//! Reproduces `scripts/worker/cleanup-worker.sh`.

use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::json::parse_json;
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

/// Locate an engine script (a `scripts/` shim) by name, in the same order the
/// grid script was historically found: `QUAQUA_SCRIPT_DIR`, its `../worker`
/// and parent/`worker` siblings, the binary's own directory, the
/// `QUAQUA_TEAM_TOML` sibling `scripts/worker`, the workspace root's
/// `scripts/worker` (`QUAQUA_HOME`, else `$HOME/quaqua`), then
/// `scripts/worker` under the cwd. Returns the file only when it exists.
pub fn find_engine_script(name: &str) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("QUAQUA_SCRIPT_DIR") {
        let p = PathBuf::from(&dir);
        let candidate = p.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        let candidate2 = p.join("../worker").join(name);
        if candidate2.is_file() {
            return Some(candidate2);
        }
        if let Some(parent) = p.parent() {
            let candidate3 = parent.join("worker").join(name);
            if candidate3.is_file() {
                return Some(candidate3);
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Ok(team_path) = std::env::var("QUAQUA_TEAM_TOML") {
        if let Some(parent) = Path::new(&team_path).parent() {
            let candidate = parent.join("scripts/worker").join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let candidate = crate::workspace::workspace_root()
        .join("scripts/worker")
        .join(name);
    if candidate.is_file() {
        return Some(candidate);
    }
    let candidate = PathBuf::from("scripts/worker").join(name);
    if candidate.is_file() {
        return Some(candidate);
    }
    None
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

    let runs_dir = default_runs_dir();

    let owner = std::env::var("QUAQUA_OWNER")
        .ok()
        .or_else(|| {
            std::fs::read_to_string(runs_dir.join(format!("{name}.owner")))
                .ok()
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "cos".to_string());

    let repo_name = repo
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo");
    let grid_tab = std::env::var("GRID_TAB")
        .ok()
        .or_else(|| {
            std::fs::read_to_string(runs_dir.join(format!("{name}.grid")))
                .ok()
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| format!("{repo_name}-workers"));

    run_grid_hook(&grid_tab, Some(&owner))?;

    let rm_out = git
        .worktree_remove(&repo, &wt)
        .map_err(|e| (1, format!("error: worktree remove failed: {e}")))?;

    if !rm_out.status.success() {
        let err_text = String::from_utf8_lossy(&rm_out.stderr).to_string();
        return Err((rm_out.status.code().unwrap_or(1), err_text));
    }

    let _ = std::fs::remove_file(runs_dir.join(format!("{name}.owner")));
    let _ = std::fs::remove_file(runs_dir.join(format!("{name}.grid")));
    let _ = std::fs::remove_file(runs_dir.join(format!("{name}.pipeline")));
    let _ = std::fs::remove_file(runs_dir.join(format!("{name}.srt.json")));
    println!("cleaned {name} (branch quaqua/{name} kept)");
    Ok(())
}
