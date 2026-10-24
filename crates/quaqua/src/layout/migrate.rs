//! Migration from team.toml schema_version 2 to 3 (`quaqua config migrate`).
//!
//! Performs line-by-line rewrite preserving comments and formatting:
//! - Upgrades `schema_version = 2` to `3`.
//! - Inserts `[layout] use = "slp"` when existing roles match the SLP built-in.
//! - Otherwise moves `[roles.*]` tables to `[layouts.custom.roles.*]` with `[layouts.custom] depth = ...`.
//! - Refuses `schema_version > 3` with an actionable hint.
//! - No-op on already-v3 files with exit 0 ("already at schema_version 3").

use crate::config::Config;
use crate::layout::builtin;
use crate::layout::model::Layout;
use std::path::{Path, PathBuf};

/// Outcome of the migration analysis on a team.toml string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrateResult {
    /// File was already schema_version 3; no changes needed.
    AlreadyV3,
    /// Successfully rewritten content for schema_version 3.
    Migrated(String),
}

/// Pure migration function: parses and rewrites `input` TOML text to schema_version 3.
pub fn migrate_team_toml(input: &str) -> Result<MigrateResult, String> {
    // 1. Detect schema_version from text
    let mut detected_version: Option<u64> = None;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("schema_version") {
            let rest = rest.trim_start();
            if let Some(val_str) = rest.strip_prefix('=') {
                let val_str = val_str.trim();
                let num_str: String = val_str
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(v) = num_str.parse::<u64>() {
                    detected_version = Some(v);
                    break;
                }
            }
        }
    }

    let version = detected_version.unwrap_or(2);
    if version > 3 {
        return Err(format!(
            "schema_version {version} is newer than 3: downgrade not supported (hint: upgrade quaqua binary)"
        ));
    }
    if version == 3 {
        return Ok(MigrateResult::AlreadyV3);
    }
    if version < 2 {
        return Err(format!("schema_version {version} is below 2: cannot migrate"));
    }

    // 2. Parse as v2 Config to inspect role topology
    let cfg: Config = toml::from_str(input)
        .map_err(|e| format!("cannot parse team.toml as Config: {e}"))?;

    let legacy = Layout::from_legacy(&cfg);
    let slp = builtin::get("slp").ok_or_else(|| "slp built-in layout missing".to_string())?;
    let matches_slp = legacy.edge_set() == slp.edge_set();

    // 3. Line-by-line rewrite preserving comments and formatting
    let mut out_lines = Vec::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut in_roles_table = false;
    let mut current_role_has_class = false;
    let mut current_role_name = String::new();

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Detect schema_version line
        if trimmed.starts_with("schema_version") {
            if let Some(eq_pos) = line.find('=') {
                let prefix = &line[..eq_pos];
                let after = &line[eq_pos + 1..];
                // Keep any trailing comment on the schema_version line
                let comment_pos = after.find('#');
                let comment_suffix = match comment_pos {
                    Some(cp) => &after[cp..],
                    None => "",
                };

                if matches_slp {
                    out_lines.push(format!("{prefix}= 3{comment_suffix}"));
                    out_lines.push(String::new());
                    out_lines.push("[layout]".to_string());
                    out_lines.push("use = \"slp\"".to_string());
                } else {
                    out_lines.push(format!("{prefix}= 3{comment_suffix}"));
                    out_lines.push(String::new());
                    out_lines.push("[layout]".to_string());
                    out_lines.push("use = \"custom\"".to_string());
                    out_lines.push(String::new());
                    out_lines.push("[layouts.custom]".to_string());
                    out_lines.push(format!("depth = {}", cfg.limits.depth.max(1)));
                }
                continue;
            }
        }

        // For non-SLP roles: move [roles.<name>] to [layouts.custom.roles.<name>]
        if !matches_slp {
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                // If exiting a previous role table and class was missing, insert it before next table
                if in_roles_table && !current_role_has_class && !current_role_name.is_empty() {
                    let class_val = match current_role_name.as_str() {
                        "cos" => "cos",
                        "lead" => "lead",
                        "peer" => "peer",
                        _ => "peer",
                    };
                    out_lines.push(format!("class = \"{class_val}\""));
                    current_role_has_class = true;
                }

                if let Some(inner) = trimmed.strip_prefix("[roles.").and_then(|s| s.strip_suffix(']')) {
                    in_roles_table = true;
                    current_role_has_class = false;
                    current_role_name = inner.trim().to_string();
                    let rewritten = line.replace("[roles.", "[layouts.custom.roles.");
                    out_lines.push(rewritten);
                    continue;
                } else {
                    in_roles_table = false;
                    current_role_name.clear();
                }
            } else if in_roles_table && trimmed.starts_with("class") && trimmed.contains('=') {
                current_role_has_class = true;
            }
        }

        out_lines.push((*line).to_string());

        // End of file check: if last table was a custom role without class
        if !matches_slp && idx + 1 == lines.len() && in_roles_table && !current_role_has_class && !current_role_name.is_empty() {
            let class_val = match current_role_name.as_str() {
                "cos" => "cos",
                "lead" => "lead",
                "peer" => "peer",
                _ => "peer",
            };
            out_lines.push(format!("class = \"{class_val}\""));
        }
    }

    let mut result = out_lines.join("\n");
    if input.ends_with('\n') {
        result.push('\n');
    }

    // 4. Validate rewritten TOML against Config parser
    let migrated_cfg: Config = toml::from_str(&result)
        .map_err(|e| format!("migrated TOML failed parse check: {e}"))?;
    crate::config::validate(&migrated_cfg)
        .map_err(|e| format!("migrated TOML failed validation check: {e}"))?;

    Ok(MigrateResult::Migrated(result))
}

/// Compute a unified diff between old and new text.
pub fn format_diff(old_text: &str, new_text: &str, file_name: &str) -> String {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();

    let mut out = String::new();
    out.push_str(&format!("--- {}\n", file_name));
    out.push_str(&format!("+++ {}\n", file_name));

    // Simple line diff
    let mut i = 0;
    let mut j = 0;
    while i < old_lines.len() || j < new_lines.len() {
        if i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
            out.push_str(&format!(" {}\n", old_lines[i]));
            i += 1;
            j += 1;
        } else {
            // Find next match or flush differences
            if i < old_lines.len() && (j >= new_lines.len() || old_lines[i] != new_lines[j]) {
                out.push_str(&format!("-{}\n", old_lines[i]));
                i += 1;
            }
            if j < new_lines.len() && (i >= old_lines.len() || j < new_lines.len()) {
                out.push_str(&format!("+{}\n", new_lines[j]));
                j += 1;
            }
        }
    }
    out
}

/// CLI runner for `quaqua config migrate`.
///
/// - Defaults to dry-run diff printed to stdout.
/// - If `write == true`, saves `<path>.bak` then writes new content via `state::atomic_write`.
/// - If file is already v3, prints "already at schema_version 3" and returns Ok.
pub fn run_migrate(path_override: Option<&Path>, write: bool) -> Result<String, String> {
    let target_path = path_override
        .map(PathBuf::from)
        .or_else(crate::config::team_config_path)
        .unwrap_or_else(|| PathBuf::from("team.toml"));

    if !target_path.is_file() {
        return Err(format!(
            "cannot read team configuration at {}",
            target_path.display()
        ));
    }

    let content = std::fs::read_to_string(&target_path)
        .map_err(|e| format!("cannot read {}: {e}", target_path.display()))?;

    match migrate_team_toml(&content)? {
        MigrateResult::AlreadyV3 => Ok("already at schema_version 3\n".to_string()),
        MigrateResult::Migrated(new_content) => {
            if write {
                let bak_path = target_path.with_extension("toml.bak");
                std::fs::write(&bak_path, content.as_bytes())
                    .map_err(|e| format!("failed to write backup {}: {e}", bak_path.display()))?;
                crate::state::atomic_write(&target_path, new_content.as_bytes())
                    .map_err(|e| format!("failed to write {}: {e}", target_path.display()))?;
                Ok(format!(
                    "migrated {} to schema_version 3 (backup at {})\n",
                    target_path.display(),
                    bak_path.display()
                ))
            } else {
                let fname = target_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("team.toml");
                Ok(format_diff(&content, &new_content, fname))
            }
        }
    }
}
