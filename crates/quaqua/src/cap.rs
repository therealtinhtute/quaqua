//! Worker cap calculation and enforcement (`quaqua cap`).
//! Reproduces `scripts/worker/worker-cap.sh` and `count_owner_slots` in `scripts/lib/common.sh`.

use crate::herdr::Herdr;
use crate::json::parse_json;
use std::path::Path;
use std::process::Command;

const WORKER_KINDS: [&str; 8] = [
    "tieude", "detu", "sude", "ni", "suhuynh", "truonglao", "bacdau", "quansu",
];

/// Check if a worker name matches `<kind>-...` and does not end with `-lead`.
pub fn is_task_worker_name(name: &str) -> bool {
    if name.ends_with("-lead") {
        return false;
    }
    for k in WORKER_KINDS {
        if name.starts_with(&format!("{k}-")) {
            return true;
        }
    }
    false
}

/// Check if process with `pid` is currently alive.
fn is_pid_alive(pid_str: &str) -> bool {
    if let Ok(pid) = pid_str.parse::<i32>() {
        if pid > 0 {
            let res = Command::new("kill").arg("-0").arg(pid_str).status();
            return res.map(|s| s.success()).unwrap_or(false);
        }
    }
    false
}

/// Check if a worker name matches `*-pr[0-9]*` or `*-<task>r[0-9]*`.
pub fn matches_pipeline_reviewer(w: &str, task: &str) -> bool {
    if let Some(idx) = w.find("-pr") {
        let after = &w[idx + 3..];
        if after.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            return true;
        }
    }
    if !task.is_empty() {
        let pat = format!("-{}r", task);
        if let Some(idx) = w.find(&pat) {
            let after = &w[idx + pat.len()..];
            if after.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                return true;
            }
        }
    }
    false
}

/// Calculate the number of worker slots used by `owner` (defaults to "cos").
pub fn count_owner_slots(
    runs_dir: &Path,
    owner: &str,
    exclude_worker: Option<&str>,
    herdr: &Herdr,
) -> i64 {
    let mut live_workers = Vec::new();
    if let Ok(agent_list_json) = herdr.agent_list() {
        if let Ok(json) = parse_json(&agent_list_json) {
            if let Some(agents) = json.get("result").and_then(|r| r.get("agents")).and_then(|a| a.as_array()) {
                for agent in agents {
                    let status = agent.get_str("agent_status").unwrap_or("");
                    let name = agent.get_str("name").unwrap_or("");
                    if status != "dead" && !name.is_empty() && is_task_worker_name(name) {
                        if Some(name) != exclude_worker {
                            live_workers.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    let mut o_live: i64 = 0;
    let mut o_workers = Vec::new();
    for w in &live_workers {
        let owner_file = runs_dir.join(format!("{w}.owner"));
        let w_owner = std::fs::read_to_string(&owner_file)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let w_owner = if w_owner.is_empty() { "cos" } else { &w_owner };
        if w_owner == owner {
            o_live += 1;
            o_workers.push(w.clone());
        }
    }

    let mut res_slots: i64 = 0;
    if let Ok(entries) = std::fs::read_dir(runs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if file_name.starts_with("pipeline-") && file_name.ends_with(".state") {
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                if !content.lines().any(|l| l.trim() == "finished=0") {
                    continue;
                }

                let mut p_lead = String::new();
                let mut p_pid = String::new();
                let mut p_builder = String::new();
                let mut p_task = String::new();

                for line in content.lines() {
                    let line = line.trim();
                    if let Some(v) = line.strip_prefix("lead=") {
                        p_lead = v.to_string();
                    } else if let Some(v) = line.strip_prefix("pipeline_pid=") {
                        p_pid = v.to_string();
                    } else if let Some(v) = line.strip_prefix("builder=") {
                        p_builder = v.to_string();
                    } else if let Some(v) = line.strip_prefix("task=") {
                        p_task = v.to_string();
                    }
                }

                let p_owner = if p_lead.is_empty() { "cos" } else { &p_lead };
                if p_owner != owner {
                    continue;
                }

                let p_bname = format!("{}-{}", p_builder, p_task);
                let mut p_alive = false;
                if !p_pid.is_empty() && is_pid_alive(&p_pid) {
                    p_alive = true;
                } else if live_workers.iter().any(|a| a == &p_bname) {
                    p_alive = true;
                }

                if p_alive {
                    let mut p_cur_live: i64 = 0;
                    for w in &o_workers {
                        let pipe_file = runs_dir.join(format!("{w}.pipeline"));
                        let w_pipe = std::fs::read_to_string(&pipe_file)
                            .map(|s| s.trim().to_string())
                            .unwrap_or_default();
                        let is_reviewer = matches_pipeline_reviewer(w, &p_task);
                        if (!p_task.is_empty() && w_pipe == p_task)
                            || w == &p_bname
                            || is_reviewer
                        {
                            p_cur_live += 1;
                        }
                    }
                    if p_cur_live < 2 {
                        res_slots += 2 - p_cur_live;
                    }
                }
            }
        }
    }

    o_live + res_slots
}
