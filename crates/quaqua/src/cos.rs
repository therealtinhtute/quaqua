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

pub const COS_DISPLAY_AGENT: &str = "🍑 CoS Quả Quả";
pub const COS_PANE_TITLE: &str = "🍑 CoS";
pub const COS_ROLE: &str = "cos";
pub const COS_ROLE_TOKEN: &str = "grid_role=cos";

/// Check whether a role token string indicates a CoS pane.
/// Matches either "cos" or "grid_role=cos".
pub fn is_cos_pane_token(role: &str) -> bool {
    role == COS_ROLE || role == COS_ROLE_TOKEN
}

/// Check whether a JSON object of tokens indicates a CoS pane (`grid_role=cos`).
pub fn is_cos_tokens(tokens: &serde_json::Value) -> bool {
    tokens
        .get("grid_role")
        .and_then(|r| r.as_str())
        .map(is_cos_pane_token)
        .unwrap_or(false)
}

/// Check whether a structured `Tokens` object indicates a CoS pane.
pub fn is_cos_tokens_struct(tokens: &crate::grid::types::Tokens) -> bool {
    tokens
        .grid_role
        .as_deref()
        .map(is_cos_pane_token)
        .unwrap_or(false)
}

/// The opt-in sidebar chip template snippet for ~/.config/herdr/config.toml.
pub fn sidebar_chip_snippet() -> &'static str {
    r##"[ui.sidebar.agents]
row_gap = 0
rows = [
  [{ token = "$grid_role", fg = "#888888", rules = [{ equals = "cos", fg = "#ff5fa2", bold = true }] }, "state_icon", "machine", "workspace", "tab"],
  ["agent"],
]"##
}

/// Resolve the target pane for CoS tagging:
/// 1. Explicit pane if given.
/// 2. `$HERDR_PANE_ID` environment variable if set and non-empty.
/// 3. Query `herdr pane current` via HerdrOps.
pub fn resolve_cos_pane<H: HerdrOps>(herdr: &H, explicit_pane: Option<&str>) -> Result<String, String> {
    if let Some(p) = explicit_pane {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    if let Ok(env_pane) = std::env::var("HERDR_PANE_ID") {
        let trimmed = env_pane.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
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
        .ok_or_else(|| "cannot determine pane: missing /result/pane/pane_id in herdr output".to_string())
}

/// Tag the specified pane as the CoS pane:
/// - Reports metadata: `--source herdr-quaqua --token grid_role=cos --display-agent "🍑 CoS Quả Quả"`
/// - Renames pane: `"🍑 CoS"`
/// - Prints confirmation and sidebar configuration hint.
///
/// Idempotent: safe to run multiple times or retry after a crash.
pub fn tag_cos_pane<H: HerdrOps>(herdr: &H, pane: &str) -> Result<String, String> {
    // 1. Report metadata
    let report_out = herdr
        .run(&[
            "pane",
            "report-metadata",
            pane,
            "--source",
            "herdr-quaqua",
            "--token",
            COS_ROLE_TOKEN,
            "--display-agent",
            COS_DISPLAY_AGENT,
        ])
        .map_err(|e| format!("herdr pane report-metadata failed for {pane}: {e}"))?;

    if !report_out.status.success() {
        let err = String::from_utf8_lossy(&report_out.stderr);
        return Err(format!("herdr pane report-metadata failed for {pane}: {err}"));
    }

    // 2. Rename pane
    let rename_out = herdr
        .run(&["pane", "rename", pane, COS_PANE_TITLE])
        .map_err(|e| format!("herdr pane rename failed for {pane}: {e}"))?;

    if !rename_out.status.success() {
        let err = String::from_utf8_lossy(&rename_out.stderr);
        return Err(format!("herdr pane rename failed for {pane}: {err}"));
    }

    let snippet = sidebar_chip_snippet();
    let msg = format!(
        "Tagged pane {pane} as grid_role=cos ({COS_DISPLAY_AGENT})\n\
         \n\
         Hint: to display the CoS badge in the herdr sidebar, add this snippet to ~/.config/herdr/config.toml:\n\
         {snippet}\n\
         then run `herdr server reload-config`.\n"
    );

    Ok(msg)
}
