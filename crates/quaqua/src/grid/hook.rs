use super::apply::{apply_ops, CliHerdrDriver, HerdrDriver};
use super::plan::plan;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

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

pub fn run_hook() -> Result<(), String> {
    let sdir = state_dir();
    create_dir_all(&sdir).map_err(|e| format!("cannot create state dir {}: {e}", sdir.display()))?;

    let shadow_mode = std::env::var("HERDR_GRID_SHADOW")
        .map(|v| v != "0")
        .unwrap_or(true);

    let mut driver = CliHerdrDriver::new();
    let snap = driver.snapshot()?;
    let ops = plan(&snap)?;

    let shadow_log = sdir.join("shadow.log");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&shadow_log)
        .map_err(|e| format!("cannot open {}: {e}", shadow_log.display()))?;

    let ops_json = serde_json::to_string(&ops).unwrap_or_else(|_| "[]".to_string());
    writeln!(file, "{ops_json}").map_err(|e| e.to_string())?;

    let last_snap_file = sdir.join("last_snapshot.json");
    if let Ok(snap_json) = serde_json::to_string(&snap) {
        let _ = std::fs::write(last_snap_file, snap_json);
    }

    if !shadow_mode {
        apply_ops(&mut driver, &ops)?;
    }

    Ok(())
}
