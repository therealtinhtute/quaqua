//! CoS pane tagging, metadata management, and protection helpers.
//!
//! # Safety & Protection Contract
//!
//! The Chief of Staff (CoS) pane is the central orchestrator pane in the
//! workspace. To prevent accidental termination or displacement:
//! - `worker cleanup` refuses to close a pane tagged `grid_role=cos`.
//! - `grid reconcile` (`apply_ops`) refuses to move a pane tagged `grid_role=cos`.
//! - PreToolUse `gate/` denies any `herdr pane close` or `herdr pane move`
//!   targeting a pane tagged `grid_role=cos`.
//!
//! A manual close from the herdr UI cannot be intercepted or blocked by quaqua
//! (herdr has no pane-level pin/lock primitive). If the CoS pane is closed
//! manually in herdr, recovery is:
//! 1. Open a new terminal in the repository checkout root.
//! 2. Run `claude --resume` to restore the agent session.
//! 3. Run `quaqua cos tag` to reapply the `grid_role=cos` token, display name,
//!    and pane title.

use crate::herdr::HerdrOps;
use std::path::Path;

/// Name of the CoS identity metadata file in runs_dir.
pub const COS_IDENTITY_FILE: &str = "cos.identity.json";

pub const COS_DISPLAY_AGENT: &str = "🍑 CoS Quả Quả";
pub const COS_PANE_TITLE: &str = "🍑 CoS";
pub const COS_ROLE: &str = "cos";
pub const COS_OWNER: &str = "cos";
pub const COS_ROLE_TOKEN: &str = "grid_role=cos";
pub const COS_OWNER_TOKEN: &str = "grid_owner=cos";

/// Check whether a role token value indicates a CoS pane (`"cos"`).
pub fn is_cos_pane_token(role: &str) -> bool {
    role == COS_ROLE
}

/// Check whether a JSON object of tokens indicates a CoS pane (`grid_role=cos`).
pub fn is_cos_tokens(tokens: &serde_json::Value) -> bool {
    tokens
        .get("grid_role")
        .and_then(|r| r.as_str())
        .map(is_cos_pane_token)
        .unwrap_or(false)
}

/// Shared helper for cleanup and gate: query `herdr pane get <pane>` and check
/// whether its tokens have `grid_role=cos`.
pub fn pane_is_cos<H: HerdrOps>(herdr: &H, pane: &str) -> bool {
    if pane.is_empty() {
        return false;
    }
    if let Ok(out) = herdr.run(&["pane", "get", pane]) {
        if out.status.success() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                if let Some(tokens) = v.pointer("/result/pane/tokens") {
                    return is_cos_tokens(tokens);
                }
            }
        }
    }
    false
}

/// Query `herdr pane get <pane>` and return `Ok(Some(grid_role))` if a `grid_role` token exists,
/// `Ok(None)` if no `grid_role` token is set on the pane, or `Err(reason)` on failure.
pub fn pane_grid_role<H: HerdrOps>(herdr: &H, pane: &str) -> Result<Option<String>, String> {
    if pane.is_empty() {
        return Err("pane is empty".to_string());
    }
    let out = herdr
        .run(&["pane", "get", pane])
        .map_err(|e| format!("herdr pane get failed for {pane}: {e}"))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("herdr pane get failed for {pane}: {err}"));
    }

    let val: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("invalid JSON from herdr pane get {pane}: {e}"))?;

    let role = val
        .pointer("/result/pane/tokens/grid_role")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(role)
}

/// The opt-in sidebar chip template snippet for ~/.config/herdr/config.toml.
pub fn sidebar_chip_snippet() -> &'static str {
    r##"[ui.sidebar.agents]
row_gap = 0
rows = [["state_icon", "$logo", { token = "agent", fg = "#888888", rules = [{ contains = "CoS", fg = "#ff5fa2", bold = true }, { contains = "-lead", fg = "#ff9f43" }] }], ["workspace", "tab"]]

# Brand colours from hhdebb/herdr-radar lib/palette.js; ink-only vendors keep the default.
[ui.sidebar.agents.rows_by_agent]
claude = [["state_icon", { token = "$logo", fg = "#d97757" }, { token = "agent", fg = "#888888", rules = [{ contains = "CoS", fg = "#ff5fa2", bold = true }, { contains = "-lead", fg = "#ff9f43" }] }], ["workspace", "tab"]]
gemini = [["state_icon", { token = "$logo", fg = "#4285f4" }, { token = "agent", fg = "#888888", rules = [{ contains = "CoS", fg = "#ff5fa2", bold = true }, { contains = "-lead", fg = "#ff9f43" }] }], ["workspace", "tab"]]
kimi = [["state_icon", { token = "$logo", fg = "#1783ff" }, { token = "agent", fg = "#888888", rules = [{ contains = "CoS", fg = "#ff5fa2", bold = true }, { contains = "-lead", fg = "#ff9f43" }] }], ["workspace", "tab"]]
qwen = [["state_icon", { token = "$logo", fg = "#615ced" }, { token = "agent", fg = "#888888", rules = [{ contains = "CoS", fg = "#ff5fa2", bold = true }, { contains = "-lead", fg = "#ff9f43" }] }], ["workspace", "tab"]]
kiro = [["state_icon", { token = "$logo", fg = "#9046ff" }, { token = "agent", fg = "#888888", rules = [{ contains = "CoS", fg = "#ff5fa2", bold = true }, { contains = "-lead", fg = "#ff9f43" }] }], ["workspace", "tab"]]
cline = [["state_icon", { token = "$logo", fg = "#586876" }, { token = "agent", fg = "#888888", rules = [{ contains = "CoS", fg = "#ff5fa2", bold = true }, { contains = "-lead", fg = "#ff9f43" }] }], ["workspace", "tab"]]
kilo = [["state_icon", { token = "$logo", fg = "#9a9808" }, { token = "agent", fg = "#888888", rules = [{ contains = "CoS", fg = "#ff5fa2", bold = true }, { contains = "-lead", fg = "#ff9f43" }] }], ["workspace", "tab"]]"##
}

/// Resolve the target pane for CoS tagging:
/// 1. Explicit pane if given.
/// 2. `$HERDR_PANE` or `$HERDR_PANE_ID` environment variable if set and non-empty.
/// 3. Query `herdr pane current` via HerdrOps.
pub fn resolve_cos_pane<H: HerdrOps>(
    herdr: &H,
    explicit_pane: Option<&str>,
) -> Result<String, String> {
    if let Some(p) = explicit_pane {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let env_pane = std::env::var("HERDR_PANE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("HERDR_PANE_ID")
                .ok()
                .filter(|s| !s.trim().is_empty())
        });
    if let Some(p) = env_pane {
        return Ok(p.trim().to_string());
    }

    let out = herdr
        .run(&["pane", "current"])
        .map_err(|e| format!("cannot determine pane: failed to run herdr pane current: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "cannot determine pane: herdr pane current exited non-zero: {stderr}"
        ));
    }

    let val: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("cannot determine pane: invalid JSON from herdr pane current: {e}"))?;

    val.pointer("/result/pane/pane_id")
        .and_then(|p| p.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            "cannot determine pane: missing /result/pane/pane_id in herdr output".to_string()
        })
}

/// Resolve the CoS display agent name and pane title for a workspace.
/// Ensures display_agent always contains "CoS" to trigger sidebar pink-highlighting.
pub fn resolve_cos_display_info(ws_dir: &Path) -> (String, String) {
    let identity_path = if ws_dir.join("identity.md").is_file() {
        Some(ws_dir.join("identity.md"))
    } else if ws_dir.join(".cos/identity.md").is_file() {
        Some(ws_dir.join(".cos/identity.md"))
    } else {
        None
    };

    let dir_name = ws_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| {
            if s == ".cos" {
                ws_dir
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or(s)
            } else {
                s
            }
        })
        .unwrap_or("quaqua");

    if dir_name.contains("quaqua") {
        return (COS_DISPLAY_AGENT.to_string(), COS_PANE_TITLE.to_string());
    }

    if let Some(path) = identity_path {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(first_line) = content.lines().next() {
                let trimmed = first_line.trim().trim_start_matches('#').trim();
                if trimmed.contains("Quả Quả") {
                    return (COS_DISPLAY_AGENT.to_string(), COS_PANE_TITLE.to_string());
                }
                let name_part = if let Some((left, right)) =
                    trimmed.split_once('—').or_else(|| trimmed.split_once('-'))
                {
                    let left = left.trim();
                    let right = right.trim();
                    if left.eq_ignore_ascii_case("identity") || left.eq_ignore_ascii_case("agents")
                    {
                        dir_name.to_string()
                    } else if left.eq_ignore_ascii_case("cos") {
                        let cleaned = right
                            .replace("Chief of Staff", "")
                            .replace("CoS", "")
                            .trim()
                            .to_string();
                        if !cleaned.is_empty() {
                            cleaned
                        } else {
                            dir_name.to_string()
                        }
                    } else {
                        left.to_string()
                    }
                } else {
                    dir_name.to_string()
                };

                let display = if name_part.contains("CoS") {
                    name_part
                } else {
                    format!("CoS {name_part}")
                };
                return (display.clone(), display);
            }
        }
    }

    let display = format!("CoS {dir_name}");
    (display.clone(), display)
}

/// Resolve the effective runs_dir for CoS tagging in `ws_dir`.
pub fn resolve_cos_runs_dir(ws_dir: &Path) -> std::path::PathBuf {
    if ws_dir.join(".cos/runs").is_dir() {
        ws_dir.join(".cos/runs")
    } else if ws_dir.join("runs").is_dir() {
        ws_dir.join("runs")
    } else {
        crate::worker::cleanup::default_runs_dir()
    }
}

/// Tag the specified pane as the CoS pane for a specific workspace directory:
/// - Reports metadata: `--source herdr-quaqua --token grid_role=cos --token grid_owner=cos --token grid_farm=<farm> --display-agent "<display_agent>"`
/// - Renames pane: `"<pane_title>"`
/// - Records `<runs_dir>/cos.identity.json = {agent_session, pane_id}` if an agent session is present
pub fn tag_cos_pane_for_workspace<H: HerdrOps>(
    herdr: &H,
    pane: &str,
    ws_dir: &Path,
) -> Result<String, String> {
    let (display_agent, pane_title) = resolve_cos_display_info(ws_dir);
    let runs_dir = resolve_cos_runs_dir(ws_dir);
    tag_cos_pane_with(herdr, pane, &runs_dir, &display_agent, &pane_title)
}

/// Tag the specified pane as the CoS pane:
/// - Reports metadata: `--source herdr-quaqua --token grid_role=cos --token grid_owner=cos --token grid_farm=<farm> --display-agent "🍑 CoS Quả Quả"`
/// - Renames pane: `"🍑 CoS"`
/// - Prints confirmation and sidebar configuration hint.
///
/// Idempotent: safe to run multiple times or retry after a crash.
pub fn tag_cos_pane<H: HerdrOps>(herdr: &H, pane: &str) -> Result<String, String> {
    tag_cos_pane_in(herdr, pane, &crate::worker::cleanup::default_runs_dir())
}

/// Tag the specified pane as the CoS pane in the given `runs_dir`:
/// - Reports metadata: `--source herdr-quaqua --token grid_role=cos --token grid_owner=cos --token grid_farm=<farm> --display-agent "🍑 CoS Quả Quả"`
/// - Renames pane: `"🍑 CoS"`
/// - Records `runs/cos.identity.json = {agent_session, pane_id}` if an agent session is present
/// - Prints confirmation and sidebar configuration hint.
///
/// Idempotent: safe to run multiple times or retry after a crash.
pub fn tag_cos_pane_in<H: HerdrOps>(
    herdr: &H,
    pane: &str,
    runs_dir: &Path,
) -> Result<String, String> {
    tag_cos_pane_with(herdr, pane, runs_dir, COS_DISPLAY_AGENT, COS_PANE_TITLE)
}

/// Tag the specified pane as the CoS pane with explicit runs_dir, display_agent, and pane_title.
pub fn tag_cos_pane_with<H: HerdrOps>(
    herdr: &H,
    pane: &str,
    runs_dir: &Path,
    display_agent: &str,
    pane_title: &str,
) -> Result<String, String> {
    // 1. Report metadata
    let farm_tok = format!("grid_farm={}", crate::workspace::farm_id());
    let report_out = herdr
        .run(&[
            "pane",
            "report-metadata",
            pane,
            "--source",
            "herdr-quaqua",
            "--token",
            COS_ROLE_TOKEN,
            "--token",
            COS_OWNER_TOKEN,
            "--token",
            &farm_tok,
            "--display-agent",
            display_agent,
        ])
        .map_err(|e| format!("herdr pane report-metadata failed for {pane}: {e}"))?;

    if !report_out.status.success() {
        let err = String::from_utf8_lossy(&report_out.stderr);
        return Err(format!(
            "herdr pane report-metadata failed for {pane}: {err}"
        ));
    }

    // 2. Rename pane
    let rename_out = herdr
        .run(&["pane", "rename", pane, pane_title])
        .map_err(|e| format!("herdr pane rename failed for {pane}: {e}"))?;

    if !rename_out.status.success() {
        let err = String::from_utf8_lossy(&rename_out.stderr);
        return Err(format!("herdr pane rename failed for {pane}: {err}"));
    }

    // 3. Record runs/cos.identity.json if agent_session is found, or remove stale record
    let sess_recorded = record_cos_identity_if_present(herdr, pane, runs_dir)?;

    let snippet = sidebar_chip_snippet();
    let msg = if sess_recorded.is_some() {
        format!(
            "Tagged pane {pane} as grid_role=cos ({display_agent})\n\
             \n\
             Hint: to display the CoS badge in the herdr sidebar, add this snippet to ~/.config/herdr/config.toml:\n\
             {snippet}\n\
             then run `herdr server reload-config`.\n"
        )
    } else {
        format!(
            "Tagged pane {pane} as grid_role=cos ({display_agent}) [no agent_session recorded]\n\
             \n\
             Hint: to display the CoS badge in the herdr sidebar, add this snippet to ~/.config/herdr/config.toml:\n\
             {snippet}\n\
             then run `herdr server reload-config`.\n"
        )
    };

    Ok(msg)
}

/// Record `runs/cos.identity.json = {agent_session, pane_id}` when agent_session is available on the pane,
/// or remove the record if the pane carries no agent_session.
pub fn record_cos_identity_if_present<H: HerdrOps>(
    herdr: &H,
    pane: &str,
    runs_dir: &Path,
) -> Result<Option<String>, String> {
    let mut session_val = None;
    // Check pane get <pane>
    if let Ok(out) = herdr.run(&["pane", "get", pane]) {
        if out.status.success() {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                session_val = val
                    .pointer("/result/pane/agent_session/value")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| {
                        val.pointer("/result/pane/agent_session")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    });
            }
        }
    }

    let path = runs_dir.join(COS_IDENTITY_FILE);
    if let Some(sess) = session_val {
        let payload = serde_json::json!({
            "agent_session": sess,
            "pane_id": pane,
        });
        let content = serde_json::to_string_pretty(&payload)
            .map_err(|e| format!("cannot serialize CoS identity: {e}"))?;
        crate::state::atomic_write(&path, content.as_bytes()).map_err(|e| {
            eprintln!("warning: failed to write {}: {e}", path.display());
            format!("failed to write {}: {e}", path.display())
        })?;
        Ok(Some(sess))
    } else {
        if let Some((_, existing_pane)) = load_cos_identity(runs_dir) {
            if existing_pane == pane {
                let _ = std::fs::remove_file(&path);
                eprintln!(
                    "warning: pane {pane} carries no agent_session; runs/cos.identity.json removed"
                );
            } else {
                eprintln!(
                    "warning: pane {pane} carries no agent_session; keeping existing runs/cos.identity.json for pane '{existing_pane}'"
                );
            }
        }
        Ok(None)
    }
}

/// Read `runs/cos.identity.json`, returning `Some((agent_session, pane_id))`.
/// Handles both `"agent_session": "..."` and `"agent_session": {"value": "..."}`.
pub fn load_cos_identity(runs_dir: &Path) -> Option<(String, String)> {
    let path = runs_dir.join(COS_IDENTITY_FILE);
    let content = std::fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    let session = val
        .get("agent_session")
        .and_then(|s| {
            s.get("value")
                .and_then(|v| v.as_str())
                .or_else(|| s.as_str())
        })?
        .trim();
    if session.is_empty() {
        return None;
    }
    let pane_id = val
        .get("pane_id")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .trim();
    Some((session.to_string(), pane_id.to_string()))
}
