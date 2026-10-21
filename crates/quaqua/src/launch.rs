//! Agent launch engine (`Launch`, `resolve_launch`, `build_launch_cmd`, `claude_trust_dialog_up`).
//!
//! Shared across task workers (`worker::spawn`) and project leads (`lead::spawn`).

use crate::config::Kind;
use crate::herdr::HerdrOps;

/// The harness/model/effort a spawn launches with, resolved from team.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub harness: String,
    pub model: String,
    pub effort: String,
}

/// Resolve a kind's launch from team.toml, failing closed when the kind has no
/// model or effort, or the harness cannot take that effort value. No defaults:
/// team.toml is the only source (the old `unwrap_or("medium"/"high")` are gone).
pub fn resolve_launch(kind: &Kind, kind_name: &str) -> Result<Launch, (i32, String)> {
    let Some(harness) = crate::harness::Harness::parse(&kind.harness) else {
        return Err((
            1,
            format!("error: unsupported harness '{}' for kind '{kind_name}'\n", kind.harness),
        ));
    };
    if kind.model.trim().is_empty() {
        return Err((1, format!("error: kind '{kind_name}' has no model in team.toml\n")));
    }
    let effort = match kind.effort.as_deref() {
        Some(e) if !e.trim().is_empty() => e,
        _ => return Err((1, format!("error: kind '{kind_name}' has no effort in team.toml\n"))),
    };
    if !harness.effort_allowed(effort) {
        return Err((
            1,
            format!(
                "error: harness '{}' does not take effort '{effort}' (kind '{kind_name}')\n",
                kind.harness
            ),
        ));
    }
    if harness == crate::harness::Harness::Opencode {
        match crate::config::opencode_model_variants(&kind.model) {
            Some(allowed) => {
                if !allowed.contains(&effort) {
                    return Err((
                        1,
                        format!(
                            "error: harness 'opencode' model '{}' does not take effort '{effort}' (kind '{kind_name}')\n",
                            kind.model
                        ),
                    ));
                }
            }
            None => {
                return Err((
                    1,
                    format!(
                        "error: opencode model '{}' is not in allowlist (fix: add model and variants to config.rs model_effort_allowed) (kind '{kind_name}')\n",
                        kind.model
                    ),
                ));
            }
        }
    }
    Ok(Launch {
        harness: kind.harness.clone(),
        model: kind.model.clone(),
        effort: effort.to_string(),
    })
}

/// omp tools a reviewer keeps: no `edit`/`write`/`notebook`/`python`.
pub const OMP_REVIEW_TOOLS: &str = "read,bash,grep,glob";

/// One attempt's `herdr` invocation: the pane-split envs and the args after
/// `herdr agent start <name> --kind <harness> --pane <p> --timeout <ms>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchCmd {
    pub split_envs: Vec<(String, String)>,
    pub agent_args: Vec<String>,
}

/// Build the exact launch command for a resolved kind. opencode has no `--model`
/// on the interactive TUI (`opencode --help` flags are --standalone/--server/
/// --auto/--continue/--session/--prompt), so its model+effort travel as the
/// config `model` object `{"providerID": "...", "model": "...", "variant": "..."}`
/// in OPENCODE_CONFIG_CONTENT in the pane env.
/// omp and pi take `--model`/`--thinking`, claude takes `--model`/`--effort`.
pub fn build_launch_cmd(
    launch: &Launch,
    name: &str,
    rules_file: &str,
    runs_dir: &str,
    wt: &str,
    review: bool,
) -> Result<LaunchCmd, (i32, String)> {
    let mut split_envs: Vec<(String, String)> = Vec::new();

    let agent_args: Vec<String> = match launch.harness.as_str() {
        "opencode" => {
            let (provider_id, model_id) = match launch.model.split_once('/') {
                Some((p, m)) => (p, m),
                None => ("opencode-go", launch.model.as_str()),
            };
            let cfg = format!(
                "{{\"model\":{{\"providerID\":\"{}\",\"model\":\"{}\",\"variant\":\"{}\"}},\"instructions\":[\"{}\"]}}",
                provider_id, model_id, launch.effort, rules_file
            );
            split_envs.push(("OPENCODE_CONFIG_CONTENT".to_string(), cfg));
            let state_dir = format!("{runs_dir}/.opencode-state/{name}");
            split_envs.push(("XDG_STATE_HOME".to_string(), state_dir));
            vec!["--".to_string(), "--standalone".to_string(), "--auto".to_string()]
        }
        "omp" => {
            let mut args = vec![
                "--".to_string(),
                "--model".to_string(),
                launch.model.clone(),
                "--thinking".to_string(),
                launch.effort.clone(),
                "--auto-approve".to_string(),
            ];
            // Review: read tools + bash only, no edit/write (`omp --help`).
            // Detection (`worker::prompt::readonly`) is the guarantee.
            if review {
                args.push(format!("--tools={OMP_REVIEW_TOOLS}"));
            }
            args.push("--append-system-prompt".to_string());
            args.push(rules_file.to_string());
            args
        }
        "pi" => {
            // Isolated: only herdr's state extension loads, so personal
            // subagent/memory extensions cannot spawn outside the grid.
            let home = std::env::var("HOME").unwrap_or_default();
            let mut args = vec![
                "--".to_string(),
                "--model".to_string(),
                launch.model.clone(),
                "--thinking".to_string(),
                launch.effort.clone(),
                "--append-system-prompt".to_string(),
                rules_file.to_string(),
                "--session-dir".to_string(),
                format!("{runs_dir}/.pi-sessions/{name}"),
                "--no-extensions".to_string(),
                "-e".to_string(),
                format!("{home}/.pi/agent/extensions/herdr-agent-state.ts"),
                "--no-approve".to_string(),
            ];
            if review {
                args.push("--tools".to_string());
                args.push("read,grep,find,ls,bash".to_string());
            }
            args
        }
        "claude" => {
            let claude_model = if launch.model.starts_with("claude-") {
                launch.model.clone()
            } else {
                format!("claude-{}", launch.model.replace('.', "-"))
            };
            let mut args = vec![
                "--".to_string(),
                "--model".to_string(),
                claude_model,
                "--effort".to_string(),
                launch.effort.clone(),
            ];
            if review {
                let runs_clean = runs_dir.trim_start_matches('/');
                let wt_clean = wt.trim_start_matches('/');
                let settings = format!(
                    "{{\"permissions\":{{\"allow\":[\"Edit(//{runs_clean}/**)\",\"Bash(cargo test:*)\",\"Bash(mv *runs/.*.tmp *runs/*.md)\"],\"deny\":[\"Edit(//{wt_clean}/**)\"]}}}}"
                );
                args.push("--permission-mode".to_string());
                args.push("dontAsk".to_string());
                args.push("--add-dir".to_string());
                args.push(runs_dir.to_string());
                args.push("--settings".to_string());
                args.push(settings);
            }
            args.push("--append-system-prompt-file".to_string());
            args.push(rules_file.to_string());
            args
        }
        other => return Err((1, format!("error: unsupported harness '{other}'\n"))),
    };

    Ok(LaunchCmd { split_envs, agent_args })
}

/// Is Claude Code's workspace-trust dialog still on screen? Its wording varies
/// between versions; every variant contains the word "trust" ("Quick safety
/// check: is this a project you created or one you trust?" / "Yes, I trust this
/// folder"), and it is word-wrapped to fit the pane, so match that one word
/// rather than a phrase. The look is deliberately sensitive: answering a dialog
/// that is not there costs a harmless Down and Enter on an empty prompt box,
/// missing one leaves the worker stalled.
pub fn claude_trust_dialog_up<H: HerdrOps>(herdr: &H, pane: &str) -> bool {
    herdr
        .pane_read(pane, "visible", 30)
        .map(|text| text.contains("trust"))
        .unwrap_or(false)
}
