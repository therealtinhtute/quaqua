use super::apply::{apply_ops, HerdrDriver};
use super::plan::{is_candidate_worker, is_group_tab, plan};
use super::types::{Op, Snapshot, Target};
use std::collections::HashSet;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ReconcileOptions {
    pub check: bool,
    pub repair: bool,
    pub tab: Option<String>,
    pub only: Option<String>,
    pub owner: Option<String>,
    pub dry_run: bool,
    pub timeout_sec: Option<u64>,
}

impl Default for ReconcileOptions {
    fn default() -> Self {
        ReconcileOptions {
            check: false,
            repair: true,
            tab: None,
            only: None,
            owner: None,
            dry_run: false,
            timeout_sec: None,
        }
    }
}

/// Read timestamp via shared fake-clock hook (PROMPT_WORKER_CLOCK_FILE), else system time
pub fn now_sec() -> u64 {
    if let Ok(clock_file) = std::env::var("PROMPT_WORKER_CLOCK_FILE") {
        if let Ok(content) = std::fs::read_to_string(&clock_file) {
            if let Ok(sec) = content.trim().parse::<u64>() {
                return sec;
            }
        }
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn poll_interval() -> Duration {
    if let Ok(poll_s) = std::env::var("GRID_CHECK_POLL_SEC") {
        if let Ok(s) = poll_s.trim().parse::<f64>() {
            if s > 0.0 {
                return Duration::from_secs_f64(s);
            }
        }
    }
    Duration::from_millis(500)
}

pub fn filter_ops_for_check<'a>(
    ops: &'a [Op],
    snapshot: &Snapshot,
    owner_filter: Option<&str>,
    only_filter: Option<&str>,
    tab_filter: Option<&str>,
) -> Vec<&'a Op> {
    let only_set: Option<HashSet<&str>> = only_filter.map(|s| {
        s.split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .collect()
    });

    let mut included_panes: HashSet<&str> = HashSet::new();
    let mut workers_tab_included = false;
    let mut result = Vec::new();

    for op in ops {
        if op.deferred {
            continue;
        }

        let pane = snapshot.panes.iter().find(|p| p.pane_id == op.pane);
        let agent = snapshot.agents.iter().find(|a| a.pane_id.as_deref() == Some(&op.pane));

        // Finding 6: name includes $a.name // $p.tokens.grid_name // $p.label
        let name = agent
            .and_then(|a| a.name.as_deref())
            .or_else(|| pane.and_then(|p| p.tokens.as_ref()).and_then(|t| t.grid_name.as_deref()))
            .or_else(|| pane.and_then(|p| p.label.as_deref()))
            .unwrap_or("");

        let powner = pane
            .and_then(|p| p.tokens.as_ref())
            .and_then(|t| t.grid_owner.as_deref())
            .unwrap_or("cos");
        let pgroup = pane
            .and_then(|p| p.tokens.as_ref())
            .and_then(|t| t.grid_group.as_deref())
            .unwrap_or("");

        if let Some(req_owner) = owner_filter {
            if !req_owner.is_empty() && powner != req_owner {
                continue;
            }
        }

        if let Some(set) = &only_set {
            if !set.is_empty() && !set.contains(name) {
                continue;
            }
        }

        // dec5a30 bugfix: --tab always scopes the check, with or without --only
        if let Some(tab) = tab_filter {
            if !tab.is_empty() && !is_group_tab(pgroup, Some(tab)) {
                continue;
            }
        }

        // Finding 3: Prevent orphaned splits if parent new_tab or anchor was filtered out
        match &op.target {
            Target::NewTab { .. } => {
                workers_tab_included = true;
                included_panes.insert(op.pane.as_str());
            }
            Target::Split { tab, anchor, .. } => {
                if tab == "@workers" && !workers_tab_included {
                    continue;
                }
                if !included_panes.contains(anchor.as_str()) && !snapshot.panes.iter().any(|p| &p.pane_id == anchor) {
                    continue;
                }
                included_panes.insert(op.pane.as_str());
            }
        }

        result.push(op);
    }

    result
}

pub fn run_reconcile<D: HerdrDriver>(
    driver: &mut D,
    opts: &ReconcileOptions,
    default_tab: &str,
) -> Result<(), String> {
    let tab = opts.tab.as_deref().unwrap_or(default_tab);
    let owner_env = opts
        .owner
        .clone()
        .or_else(|| std::env::var("QUAQUA_OWNER").ok())
        .unwrap_or_else(|| "cos".to_string());

    if opts.check {
        let timeout_sec = opts.timeout_sec.unwrap_or_else(|| {
            std::env::var("GRID_CHECK_TIMEOUT_SEC")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(60)
        });

        let start_ts = now_sec();
        let poll = poll_interval();

        loop {
            // Finding 7: Retrying snapshot failure until timeout
            let snap = match driver.snapshot() {
                Ok(s) => s,
                Err(e) => {
                    let elapsed = now_sec().saturating_sub(start_ts);
                    if elapsed >= timeout_sec {
                        return Err(format!("grid check snapshot failed after {timeout_sec}s: {e}"));
                    }
                    sleep(poll);
                    continue;
                }
            };

            let ops = plan(&snap)?;
            let rem_ops = filter_ops_for_check(
                &ops,
                &snap,
                Some(&owner_env),
                opts.only.as_deref(),
                Some(tab),
            );

            if opts.dry_run {
                let serialized = serde_json::to_string(&rem_ops).map_err(|e| e.to_string())?;
                println!("{serialized}");
                if rem_ops.is_empty() {
                    return Ok(());
                } else {
                    return Err(format!("grid check pending ops: {}", rem_ops.len()));
                }
            }

            if rem_ops.is_empty() {
                return Ok(());
            }

            let elapsed = now_sec().saturating_sub(start_ts);
            if elapsed >= timeout_sec {
                let serialized = serde_json::to_string(&rem_ops).unwrap_or_default();
                return Err(format!(
                    "grid check timed out after {timeout_sec}s; pending ops: {serialized}"
                ));
            }

            sleep(poll);
        }
    }

    // Repair / apply mode
    let snap = driver.snapshot()?;
    let all_ops = plan(&snap)?;

    // Filter ops by owner, --only and --tab
    let filtered_ops = filter_ops_for_check(
        &all_ops,
        &snap,
        Some(&owner_env),
        opts.only.as_deref(),
        if opts.tab.is_some() { Some(tab) } else { None },
    );

    let owned_ops: Vec<Op> = filtered_ops.into_iter().cloned().collect();

    if opts.dry_run {
        let serialized = serde_json::to_string(&owned_ops).map_err(|e| e.to_string())?;
        println!("{serialized}");
        return Ok(());
    }

    // Finding 5: Apply ops with scratch parking and dependent deferrals
    apply_ops(driver, &owned_ops, Some(&snap))?;

    // Finding 5: Pane renaming to agent name
    for p in &snap.panes {
        if is_candidate_worker(p) {
            let powner = p.tokens.as_ref().and_then(|t| t.grid_owner.as_deref()).unwrap_or("cos");
            if !owner_env.is_empty() && powner != owner_env {
                continue;
            }
            let target_name = snap
                .agents
                .iter()
                .find(|a| a.pane_id.as_deref() == Some(&p.pane_id))
                .and_then(|a| a.name.as_deref())
                .or_else(|| p.tokens.as_ref().and_then(|t| t.grid_name.as_deref()));
            if let Some(tname) = target_name {
                if p.label.as_deref() != Some(tname) {
                    let _ = driver.rename_pane(&p.pane_id, tname);
                }
            }
        }
    }

    Ok(())
}
