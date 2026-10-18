//! Worker cap calculation and enforcement (`quaqua cap`).
//! Reproduces `scripts/worker/worker-cap.sh` and `count_owner_slots` in `scripts/lib/common.sh`.

use crate::herdr::HerdrOps;
use crate::state::{ExecutionState, WorkerMetadata};
use crate::util::is_pid_alive;
use serde_json::Value;
use std::path::Path;

const WORKER_KINDS: [&str; 6] = [
    "tieude", "detu", "sude", "ni", "suhuynh", "truonglao",
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

fn contains_prefix_digit(w: &str, prefix: &str) -> bool {
    let mut search = w;
    while let Some(idx) = search.find(prefix) {
        let after = &search[idx + prefix.len()..];
        if after.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            return true;
        }
        search = &search[idx + prefix.len()..];
    }
    false
}

/// Check if a worker name matches `*-pr[0-9]*` or `*-<task>r[0-9]*` (when task is empty: `*-r[0-9]*`).
pub fn matches_pipeline_reviewer(w: &str, task: &str) -> bool {
    if contains_prefix_digit(w, "-pr") {
        return true;
    }
    let task_pat = format!("-{}r", task);
    if contains_prefix_digit(w, &task_pat) {
        return true;
    }
    false
}

/// Calculate the number of worker slots used by `owner` (defaults to "cos").
pub fn count_owner_slots<H: HerdrOps>(
    runs_dir: &Path,
    owner: &str,
    exclude_worker: Option<&str>,
    herdr: &H,
) -> i64 {
    let mut live_workers = Vec::new();
    if let Ok(agent_list_json) = herdr.agent_list() {
        if let Ok(json) = serde_json::from_str::<Value>(&agent_list_json) {
            if let Some(agents) = json.get("result").and_then(|r| r.get("agents")).and_then(|a| a.as_array()) {
                for agent in agents {
                    let status = agent.get("agent_status").and_then(|s| s.as_str()).unwrap_or("");
                    let name = agent.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if status != "dead" && !name.is_empty() && is_task_worker_name(name) {
                        if Some(name) != exclude_worker {
                            live_workers.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    let mut o_workers = Vec::new();
    for w in &live_workers {
        let w_owner = WorkerMetadata::load_owner(runs_dir, w);
        if w_owner == owner {
            o_workers.push(w.clone());
        }
    }
    let cfg_opt = crate::config::load_team_config().ok();
    let pipe_weight = cfg_opt
        .as_ref()
        .and_then(|c| c.limits.weights.get("pipeline"))
        .copied()
        .unwrap_or(2);
    let dual_lens_weight = cfg_opt
        .as_ref()
        .and_then(|c| c.limits.weights.get("dual-lens"))
        .copied()
        .unwrap_or(2);

    let mut total_managed_slots: i64 = 0;
    let mut covered_workers: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Ok(entries) = std::fs::read_dir(runs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let exec_state = match ExecutionState::load(&path) {
                Some(s) if !s.is_finished() => s,
                _ => continue,
            };

            match exec_state {
                ExecutionState::Pipeline(p) => {
                    if p.owner() != owner {
                        continue;
                    }
                    let p_bname = format!("{}-{}", p.builder, p.task);
                    let mut p_alive = false;
                    if !p.pipeline_pid.is_empty() && is_pid_alive(&p.pipeline_pid) {
                        p_alive = true;
                    } else if live_workers.iter().any(|a| {
                        a == &p_bname
                            || (!p.task.is_empty() && a.ends_with(&format!("-{}", p.task)))
                            || matches_pipeline_reviewer(a, &p.task)
                    }) {
                        p_alive = true;
                    }

                    if p_alive {
                        let mut p_cur_live: i64 = 0;
                        for w in &o_workers {
                            let pipe_file = runs_dir.join(format!("{w}.pipeline"));
                            let w_pipe = std::fs::read_to_string(&pipe_file)
                                .map(|s| s.trim().to_string())
                                .unwrap_or_default();
                            let is_reviewer = matches_pipeline_reviewer(w, &p.task);
                            if (!p.task.is_empty() && (w_pipe == p.task || w.ends_with(&format!("-{}", p.task))))
                                 || w == &p_bname
                                 || is_reviewer
                             {
                                 p_cur_live += 1;
                                 covered_workers.insert(w.clone());
                             }
                         }
                         total_managed_slots += p_cur_live.max(pipe_weight);
                     }
                 }
                ExecutionState::Taskforce(t) => {
                    if t.owner() != owner {
                        continue;
                    }
                    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let weight = if t.kind == "dual-lens" || file_name.starts_with("dual-lens-") {
                        dual_lens_weight
                    } else {
                        cfg_opt
                            .as_ref()
                            .and_then(|c| c.limits.weights.get(&t.kind))
                            .copied()
                            .unwrap_or(1)
                    };

                    let mut t_alive = false;
                    if !t.pid.is_empty() && is_pid_alive(&t.pid) {
                        t_alive = true;
                    } else if !t.task.is_empty() {
                        for w in &live_workers {
                            let tf_file = runs_dir.join(format!("{w}.taskforce"));
                            let w_tf = std::fs::read_to_string(&tf_file)
                                .map(|s| s.trim().to_string())
                                .unwrap_or_default();
                            if w_tf == t.task
                                || w.ends_with(&format!("-{}", t.task))
                                || w.ends_with(&format!("-{}dl1", t.task))
                                || w.ends_with(&format!("-{}dl2", t.task))
                            {
                                t_alive = true;
                                break;
                            }
                        }
                    }

                    if t_alive {
                        let mut t_cur_live: i64 = 0;
                        for w in &o_workers {
                            let tf_file = runs_dir.join(format!("{w}.taskforce"));
                            let w_tf = std::fs::read_to_string(&tf_file)
                                .map(|s| s.trim().to_string())
                                .unwrap_or_default();
                            if !t.task.is_empty()
                                && (w_tf == t.task
                                    || w.ends_with(&format!("-{}", t.task))
                                    || w.ends_with(&format!("-{}dl1", t.task))
                                    || w.ends_with(&format!("-{}dl2", t.task)))
                            {
                                t_cur_live += 1;
                                covered_workers.insert(w.clone());
                            }
                        }
                        total_managed_slots += t_cur_live.max(weight);
                    }
                }
            }
        }
    }
    let standalone_slots = o_workers
        .iter()
        .filter(|w| !covered_workers.contains(*w))
        .count() as i64;
    standalone_slots + total_managed_slots
}
