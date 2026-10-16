use super::apply::{apply_ops, CliHerdrDriver, HerdrDriver};
use super::plan::{is_candidate_worker, plan};
use super::types::{Op, Snapshot, Target};
use std::collections::HashSet;
use std::fs::{create_dir, create_dir_all, remove_dir, remove_file, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("HERDR_PLUGIN_STATE_DIR") {
        PathBuf::from(dir)
    } else if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg).join("herdr/plugins/herdr-grid")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/state/herdr/plugins/herdr-grid")
    } else {
        PathBuf::from(".herdr-grid-state")
    }
}

pub fn is_shadow_mode(state_dir: &Path) -> bool {
    if let Ok(v) = std::env::var("HERDR_GRID_SHADOW") {
        let lower = v.trim().to_lowercase();
        if lower == "0" || lower == "false" || lower == "off" {
            return false;
        }
        return true;
    }
    let shadow_file = state_dir.join("shadow");
    if shadow_file.is_file() {
        if let Ok(content) = std::fs::read_to_string(&shadow_file) {
            let lower = content.trim().to_lowercase();
            if lower.contains("0") || lower.contains("false") || lower.contains("off") {
                return false;
            }
        }
    }
    true
}

fn is_pid_alive(pid_str: &str) -> bool {
    if pid_str.trim().is_empty() {
        return false;
    }
    std::process::Command::new("kill")
        .args(["-0", pid_str.trim()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct LockGuard {
    lock_dir: PathBuf,
    pid_file: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = remove_file(&self.pid_file);
        let _ = remove_dir(&self.lock_dir);
    }
}

fn acquire_lock(lock_dir: &Path, pid_file: &Path) -> Result<LockGuard, String> {
    let mut attempts = 0;
    let my_pid = std::process::id().to_string();

    while attempts < 100 {
        if create_dir(lock_dir).is_ok() {
            let _ = std::fs::write(pid_file, &my_pid);
            return Ok(LockGuard {
                lock_dir: lock_dir.to_path_buf(),
                pid_file: pid_file.to_path_buf(),
            });
        }

        if pid_file.is_file() {
            let holder_pid = std::fs::read_to_string(pid_file).unwrap_or_default();
            if !holder_pid.trim().is_empty() && !is_pid_alive(&holder_pid) {
                // Dead PID reclaim
                let _ = remove_file(pid_file);
                let _ = remove_dir(lock_dir);
                continue;
            }
        } else if attempts > 5 {
            let _ = remove_file(pid_file);
            let _ = remove_dir(lock_dir);
            continue;
        }

        sleep(Duration::from_millis(50));
        attempts += 1;
    }

    Err("[herdr-grid] could not acquire lock after timeout".to_string())
}

/// Early exit check on pane.agent_status_changed events that change nothing
fn should_early_exit(event_name: Option<&str>, event_json: Option<&str>, last_snap_file: &Path) -> bool {
    if event_name != Some("pane.agent_status_changed") {
        return false;
    }
    let ej = match event_json {
        Some(s) if !s.is_empty() => s,
        _ => return false,
    };
    if !last_snap_file.is_file() {
        return false;
    }

    let val: serde_json::Value = match serde_json::from_str(ej) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let pid = match val.get("pane_id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return false,
    };
    let new_status = val
        .get("agent_status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let last_snap_text = match std::fs::read_to_string(last_snap_file) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let last_val: serde_json::Value = match serde_json::from_str(&last_snap_text) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let snap = match super::types::parse_snapshot_value(&last_val) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let p = match snap.panes.iter().find(|pane| pane.pane_id == pid) {
        Some(pane) => pane,
        None => return true, // Foreign/untracked pane
    };

    let role = p.tokens.as_ref().and_then(|t| t.grid_role.as_deref()).unwrap_or("");
    if role != "worker" {
        return true; // Not a worker pane
    }

    let old_working = p.agent_status.as_deref() == Some("working");
    let new_working = new_status == "working";

    // If working flag unchanged (idle <-> done <-> unknown), early exit
    old_working == new_working
}

fn compute_snapshot_signature(snap: &Snapshot) -> String {
    let mut sigs = Vec::new();
    for p in &snap.panes {
        if is_candidate_worker(p) {
            let role = p.tokens.as_ref().and_then(|t| t.grid_role.as_deref()).unwrap_or("");
            let owner = p.tokens.as_ref().and_then(|t| t.grid_owner.as_deref()).unwrap_or("");
            let group = p.tokens.as_ref().and_then(|t| t.grid_group.as_deref()).unwrap_or("");
            let status = if p.agent_status.as_deref() == Some("working") {
                "working"
            } else {
                "idle"
            };
            let tab = p.tab_id.as_deref().unwrap_or("");
            sigs.push(format!("{}:{}:{}:{}:{}:{}", p.pane_id, tab, status, role, owner, group));
        }
    }
    sigs.sort();
    sigs.join(";")
}

fn log_line(log_file: &Path, msg: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_file) {
        let ts = chrono_or_simple_time();
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

fn chrono_or_simple_time() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

fn reconcile_step(sdir: &Path, driver: &mut CliHerdrDriver) -> Result<(), String> {
    let mut snap = driver.snapshot()?;
    let shadow_log = sdir.join("shadow.log");
    let last_snap_file = sdir.join("last_snapshot.json");
    let failed_ops_file = sdir.join("failed_ops.json");
    let failed_snap_file = sdir.join("failed_snap.json");

    // Closed pane detection via snapshot diff
    if last_snap_file.is_file() {
        if let Ok(last_text) = std::fs::read_to_string(&last_snap_file) {
            if let Ok(last_val) = serde_json::from_str::<serde_json::Value>(&last_text) {
                if let Ok(last_snap) = super::types::parse_snapshot_value(&last_val) {
                    let old_workers: HashSet<String> = last_snap
                        .panes
                        .iter()
                        .filter(|p| is_candidate_worker(p))
                        .map(|p| p.pane_id.clone())
                        .collect();
                    let cur_workers: HashSet<String> = snap
                        .panes
                        .iter()
                        .filter(|p| is_candidate_worker(p))
                        .map(|p| p.pane_id.clone())
                        .collect();
                    for closed in old_workers.difference(&cur_workers) {
                        log_line(&shadow_log, &format!("closed pane detected via snapshot diff: {closed}"));
                    }
                }
            }
        }
    }

    // In-flight spawn poll: if worker present without grid_ready=1, poll briefly
    let mut has_unready = snap.panes.iter().any(|p| {
        p.tokens.as_ref().and_then(|t| t.grid_role.as_deref()) == Some("worker")
            && !is_candidate_worker(p)
    });
    if has_unready {
        for _ in 0..15 {
            sleep(Duration::from_millis(200));
            if let Ok(fresh) = driver.snapshot() {
                has_unready = fresh.panes.iter().any(|p| {
                    p.tokens.as_ref().and_then(|t| t.grid_role.as_deref()) == Some("worker")
                        && !is_candidate_worker(p)
                });
                snap = fresh;
                if !has_unready {
                    break;
                }
            }
        }
    }

    // Atomically cache snapshot
    if let Ok(snap_json) = serde_json::to_string(&snap) {
        let tmp = last_snap_file.with_extension("tmp");
        if std::fs::write(&tmp, &snap_json).is_ok() {
            let _ = std::fs::rename(tmp, &last_snap_file);
        }
    }

    let ops = plan(&snap)?;
    if ops.is_empty() {
        let _ = remove_file(&failed_ops_file);
        let _ = remove_file(&failed_snap_file);
    }

    let shadow_mode = is_shadow_mode(sdir);
    if shadow_mode {
        let ops_json = serde_json::to_string(&ops).unwrap_or_else(|_| "[]".to_string());
        log_line(&shadow_log, &format!("shadow ops: {ops_json}"));
        println!("{ops_json}");
        return Ok(());
    }

    // Live mode: filter non-deferred ops and dependent ops
    let mut deferred_panes: HashSet<String> = HashSet::new();
    let mut workers_tab_deferred = false;
    let mut non_deferred: Vec<Op> = Vec::new();

    for op in ops {
        if op.deferred {
            deferred_panes.insert(op.pane.clone());
            if matches!(op.target, Target::NewTab { .. }) {
                workers_tab_deferred = true;
            }
        } else if let Target::Split { tab, anchor, .. } = &op.target {
            if deferred_panes.contains(anchor) || (tab == "@workers" && workers_tab_deferred) {
                deferred_panes.insert(op.pane.clone());
            } else {
                non_deferred.push(op);
            }
        } else {
            non_deferred.push(op);
        }
    }

    if non_deferred.is_empty() {
        return Ok(());
    }

    // Failure backoff: avoid error storms when an op fails on an unchanged layout
    let snap_sig = compute_snapshot_signature(&snap);
    if failed_ops_file.is_file() && failed_snap_file.is_file() {
        let last_snap_sig = std::fs::read_to_string(&failed_snap_file).unwrap_or_default();
        if !snap_sig.is_empty() && last_snap_sig == snap_sig {
            let failed_text = std::fs::read_to_string(&failed_ops_file).unwrap_or_default();
            if let Ok(failed_ops) = serde_json::from_str::<Vec<Op>>(&failed_text) {
                let prev_len = non_deferred.len();
                non_deferred.retain(|op| {
                    !failed_ops.iter().any(|f| f.pane == op.pane && f.target == op.target)
                });
                if non_deferred.len() < prev_len {
                    log_line(&shadow_log, "backoff: skipping previously failed ops");
                }
            }
        } else {
            let _ = remove_file(&failed_ops_file);
            let _ = remove_file(&failed_snap_file);
        }
    }

    if non_deferred.is_empty() {
        return Ok(());
    }

    let non_deferred_json = serde_json::to_string(&non_deferred).unwrap_or_default();
    log_line(&shadow_log, &format!("apply ops: {non_deferred_json}"));

    match apply_ops(driver, &non_deferred, Some(&snap)) {
        Ok(_) => {
            let _ = remove_file(&failed_ops_file);
            let _ = remove_file(&failed_snap_file);
            // Refresh snapshot cache
            if let Ok(post_snap) = driver.snapshot() {
                if let Ok(post_json) = serde_json::to_string(&post_snap) {
                    let tmp = last_snap_file.with_extension("tmp");
                    if std::fs::write(&tmp, &post_json).is_ok() {
                        let _ = std::fs::rename(tmp, &last_snap_file);
                    }
                }
            }
            Ok(())
        }
        Err(e) => {
            log_line(&shadow_log, &format!("apply error: {e}"));
            let _ = std::fs::write(&failed_ops_file, &non_deferred_json);
            let _ = std::fs::write(&failed_snap_file, &snap_sig);
            Err(e)
        }
    }
}

pub fn run_hook() -> Result<(), String> {
    let sdir = state_dir();
    create_dir_all(&sdir).map_err(|e| format!("cannot create state dir {}: {e}", sdir.display()))?;

    let event_name = std::env::var("HERDR_PLUGIN_EVENT").ok();
    let event_json = std::env::var("HERDR_PLUGIN_EVENT_JSON").ok();
    let last_snap_file = sdir.join("last_snapshot.json");

    // Finding 4: Early exit on irrelevant status events without acquiring lock
    if should_early_exit(event_name.as_deref(), event_json.as_deref(), &last_snap_file) {
        return Ok(());
    }

    let pending_file = sdir.join("pending");
    let pid_str = std::process::id().to_string();
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&pending_file) {
        let _ = writeln!(f, "{pid_str}");
    }

    let lock_dir = sdir.join("lock");
    let pid_file = lock_dir.join("pid");
    let _guard = acquire_lock(&lock_dir, &pid_file)?;

    let mut driver = CliHerdrDriver::new();

    // Main loop handling pending counter
    loop {
        let _ = remove_file(&pending_file);
        reconcile_step(&sdir, &mut driver)?;
        if !pending_file.is_file() {
            break;
        }
    }

    Ok(())
}
