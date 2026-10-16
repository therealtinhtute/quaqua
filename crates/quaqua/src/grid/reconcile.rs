use super::apply::{apply_ops, HerdrDriver};
use super::plan::{is_group_tab, plan};
use super::types::{Op, Snapshot};
use std::collections::HashSet;
use std::thread::sleep;
use std::time::{Duration, Instant};

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

    ops.iter()
        .filter(|op| {
            if op.deferred {
                return false;
            }
            let pane = snapshot.panes.iter().find(|p| p.pane_id == op.pane);
            let agent = snapshot.agents.iter().find(|a| a.pane_id.as_deref() == Some(&op.pane));
            let name = agent
                .and_then(|a| a.name.as_deref())
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
                    return false;
                }
            }

            if let Some(set) = &only_set {
                if !set.is_empty() && !set.contains(name) {
                    return false;
                }
            }

            // dec5a30 bugfix: --tab always scopes the check, with or without --only
            if let Some(tab) = tab_filter {
                if !tab.is_empty() && !is_group_tab(pgroup, Some(tab)) {
                    return false;
                }
            }

            true
        })
        .collect()
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

        let start = Instant::now();
        loop {
            let snap = driver.snapshot()?;
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

            if start.elapsed() >= Duration::from_secs(timeout_sec) {
                let serialized = serde_json::to_string(&rem_ops).unwrap_or_default();
                return Err(format!(
                    "grid check timed out after {timeout_sec}s; pending ops: {serialized}"
                ));
            }

            sleep(Duration::from_millis(500));
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
        Some(tab),
    );

    let owned_ops: Vec<Op> = filtered_ops.into_iter().cloned().collect();

    if opts.dry_run {
        let serialized = serde_json::to_string(&owned_ops).map_err(|e| e.to_string())?;
        println!("{serialized}");
        return Ok(());
    }

    apply_ops(driver, &owned_ops)?;
    Ok(())
}
