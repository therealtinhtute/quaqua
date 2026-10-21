//! Report readiness and Lead/pipeline liveness helpers.
//!
//! The watchdog no longer reads pane text: progress comes from the worktree
//! diff and the harness session store.

use crate::util::is_pid_alive;
use std::path::Path;

/// True when `name` is the Lead of a live pipeline (a `pipeline-*.state` with
/// `finished=0` and a live pid). Runs inside an activity, never in a step.
pub fn lead_waiting(runs_dir: &Path, name: &str, watcher_pane: &str) -> bool {
    if !name.ends_with("-lead") {
        return false;
    }
    if let Ok(entries) = std::fs::read_dir(runs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if fname.starts_with("pipeline-") && fname.ends_with(".state") {
                if let Ok(c) = std::fs::read_to_string(&path) {
                    if !c.lines().any(|l| l.trim() == "finished=0") {
                        continue;
                    }
                    let mut p_lead = "";
                    let mut p_pane = "";
                    let mut p_pid = "";
                    for line in c.lines() {
                        let l = line.trim();
                        if let Some(v) = l.strip_prefix("lead=") {
                            p_lead = v;
                        } else if let Some(v) = l.strip_prefix("lead_pane=") {
                            p_pane = v;
                        } else if let Some(v) = l.strip_prefix("pipeline_pid=") {
                            p_pid = v;
                        }
                    }
                    if !p_pid.is_empty() {
                        let is_alive = is_pid_alive(p_pid);
                        if !is_alive {
                            continue;
                        }
                        if p_lead == name || (!watcher_pane.is_empty() && p_pane == watcher_pane) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

pub fn report_stat(path: &Path) -> Option<(u64, u64)> {
    let md = std::fs::metadata(path).ok()?;
    let len = md.len();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((len, md.mtime() as u64))
    }
    #[cfg(not(unix))]
    {
        Some((len, 0))
    }
}

/// One report-ready sample: non-empty and stable over 400 ms. Called once per
/// wake; the workflow reads the report immediately when this returns true.
pub fn report_ready(path: &Path) -> bool {
    if let Some((len1, mtime1)) = report_stat(path) {
        if len1 > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400));
            if let Some((len2, mtime2)) = report_stat(path) {
                return len1 == len2 && mtime1 == mtime2;
            }
        }
    }
    false
}
