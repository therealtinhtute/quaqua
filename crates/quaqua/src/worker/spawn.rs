//! Worker spawn logic (`quaqua worker spawn [--base <ref>] [--review] <name> <kind> <repo-path>`).
//! Reproduces `scripts/worker/spawn-worker.sh`.

use crate::cap::count_owner_slots;
use crate::config::Config;
use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::json::parse_json;
use crate::worker::cleanup::run_grid_hook;
use std::path::PathBuf;
use std::process::Command;

pub const USAGE: &str = "\
# Usage: spawn-worker.sh [--base <ref>] [--review] <name> <kind: tieude|detu|sude|ni|suhuynh|truonglao> <repo-path>
#   name = <kind>-<task>, ascii [a-z0-9-], <=20 chars, not ending in -lead
#          (*-lead is the project Lead convention — spawn-lead.sh <slug>-lead)
#   --base <ref>  start branch quaqua/<name> at <ref> instead of <repo> HEAD
#   --review      ni|suhuynh|truonglao: repo read-only, $HOME/quaqua/runs writable";

pub struct SpawnOptions {
    pub base: Option<String>,
    pub review: bool,
    pub name: String,
    pub kind: String,
    pub repo_path: PathBuf,
}

pub fn parse_spawn_args(args: &[String]) -> Result<SpawnOptions, (i32, String)> {
    let mut base: Option<String> = None;
    let mut review = false;
    let mut pos = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--review" {
            review = true;
            i += 1;
        } else if arg == "--base" {
            if i + 1 >= args.len() {
                return Err((1, format!("{}\n", USAGE)));
            }
            let val = &args[i + 1];
            if val.is_empty() || val.starts_with('-') {
                return Err((1, format!("{}\n", USAGE)));
            }
            base = Some(val.clone());
            i += 2;
        } else if let Some(val) = arg.strip_prefix("--base=") {
            if val.is_empty() || val.starts_with('-') {
                return Err((1, format!("{}\n", USAGE)));
            }
            base = Some(val.to_string());
            i += 1;
        } else if arg.starts_with('-') {
            return Err((1, format!("{}\n", USAGE)));
        } else {
            pos.push(arg.clone());
            i += 1;
        }
    }

    if pos.len() < 3 {
        return Err((1, format!("{}\n", USAGE)));
    }

    Ok(SpawnOptions {
        base,
        review,
        name: pos[0].clone(),
        kind: pos[1].clone(),
        repo_path: PathBuf::from(&pos[2]),
    })
}

fn is_valid_name(name: &str, kind: &str) -> bool {
    let prefix = format!("{kind}-");
    if !name.starts_with(&prefix) {
        return false;
    }
    if name.len() > 20 || name.ends_with("-lead") {
        return false;
    }
    let rest = &name[prefix.len()..];
    if rest.is_empty() {
        return false;
    }
    rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn get_iso_now() -> String {
    if let Ok(out) = Command::new("date").arg("-Iseconds").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    if let Ok(out) = Command::new("date").arg("+%Y-%m-%dT%H:%M:%S%z").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    "2026-10-16T09:00:00+0000".to_string()
}


/// Is Claude Code's workspace-trust dialog still on screen? Its wording varies
/// between versions; every variant contains the word "trust" ("Quick safety
/// check: is this a project you created or one you trust?" / "Yes, I trust this
/// folder"), and it is word-wrapped to fit the pane, so match that one word
/// rather than a phrase. The look is deliberately sensitive: answering a dialog
/// that is not there costs a harmless Down and Enter on an empty prompt box,
/// missing one leaves the worker stalled.
fn claude_trust_dialog_up<H: HerdrOps>(herdr: &H, pane: &str) -> bool {
    herdr
        .pane_read(pane, "visible", 30)
        .map(|text| text.contains("trust"))
        .unwrap_or(false)
}

pub fn spawn_worker<G: GitOps, H: HerdrOps>(
    opts: SpawnOptions,
    cfg: &Config,
    git: &G,
    herdr: &H,
) -> Result<(), (i32, String)> {
    let kind_info = cfg.kinds.get(&opts.kind).ok_or_else(|| {
        let line2 = USAGE.lines().next().unwrap_or(USAGE);
        (1, format!("{line2}\n"))
    })?;

    if !is_valid_name(&opts.name, &opts.kind) {
        let lines: Vec<&str> = USAGE.lines().take(2).collect();
        return Err((1, format!("{}\n", lines.join("\n"))));
    }

    if opts.review && !kind_info.can_review {
        eprintln!(
            "error: --review is only supported for claude kinds (ni, suhuynh, truonglao), not {}",
            opts.kind
        );
        return Err((1, format!("{}\n", USAGE)));
    }

    let harness = kind_info.harness.as_str();
    if !["opencode", "omp", "claude"].contains(&harness) {
        return Err((
            1,
            format!("error: unsupported harness '{harness}' for kind '{}'\n", opts.kind),
        ));
    }

    let repo = match std::fs::canonicalize(&opts.repo_path) {
        Ok(p) => p,
        Err(_) => opts.repo_path.clone(),
    };

    let wt = match repo.parent() {
        Some(p) => p.join(".wt").join(&opts.name),
        None => PathBuf::from(".wt").join(&opts.name),
    };

    if let Some(base_ref) = &opts.base {
        if !git.rev_parse_commit(&repo, base_ref) {
            return Err((
                1,
                format!("error: --base '{base_ref}' is not a known commit-ish in {}\n", repo.display()),
            ));
        }
    }

    let owner = std::env::var("QUAQUA_OWNER").unwrap_or_else(|_| "cos".to_string());
    let runs_dir = crate::worker::cleanup::default_runs_dir();
    let _ = std::fs::create_dir_all(&runs_dir);

    if std::env::var("PIPELINE_TASK").is_err() {
        let slots_used = count_owner_slots(&runs_dir, &owner, Some(&opts.name), herdr);
        let max_workers = cfg.limits.max_workers_per_orchestrator;
        if slots_used >= max_workers {
            return Err((
                1,
                format!(
                    "error: worker cap exceeded ({slots_used} live worker slots for owner '{owner}'; cap is {max_workers}) — cannot spawn {}\n",
                    opts.name
                ),
            ));
        }
    }

    if wt.is_dir() {
        let porcelain_wts = git.worktree_list_porcelain(&repo).unwrap_or_default();
        let wt_str = wt.to_string_lossy().to_string();
        if porcelain_wts.iter().any(|w| w == &wt_str) {
            eprintln!("note: {} already a registered worktree — reusing it (resumed spawn)", wt.display());
        } else {
            return Err((
                1,
                format!(
                    "error: {} exists but is not a git worktree of {} — remove it or pick another name\n",
                    wt.display(),
                    repo.display()
                ),
            ));
        }
    } else {
        let branch = format!("quaqua/{}", opts.name);
        let add_out = if git.branch_exists(&repo, &branch) {
            // cleanup-worker.sh drops the worktree but keeps the branch for the
            // Lead to merge, so a same-name retry after a failed (or cleaned)
            // spawn would die on `worktree add -b` with "a branch named
            // 'quaqua/<name>' already exists". Reuse the kept branch — the
            // retry resumes that task, commits included — and say so: a --base
            // cannot apply to a branch that already exists.
            let base_note = match &opts.base {
                Some(b) => format!("; --base {b} ignored"),
                None => String::new(),
            };
            eprintln!(
                "note: branch {branch} already exists — reusing it (same-name retry){base_note}"
            );
            git.worktree_add_existing(&repo, &branch, &wt)
        } else {
            git.worktree_add(&repo, &branch, &wt, opts.base.as_deref())
        }
        .map_err(|e| (1, format!("error: failed to add worktree: {e}\n")))?;
        if !add_out.status.success() {
            let err_text = String::from_utf8_lossy(&add_out.stderr).to_string();
            return Err((1, err_text));
        }
    }

    // Push block check
    if git.is_core_bare(&repo) || git.has_core_worktree(&repo) {
        return Err((
            1,
            format!(
                "error: {} has core.bare=true or core.worktree in its common config; enabling extensions.worktreeConfig for the push block would break every worktree of it. Move those keys to the main worktree's config.worktree (see git-worktree(1), CONFIGURATION FILE) or spawn from another clone.\n",
                repo.display()
            ),
        ));
    }

    let _ = git.enable_worktree_config(&repo);
    if let Ok(remotes) = git.remote_list(&repo) {
        for remote in remotes {
            let _ = git.set_worktree_pushurl_blocked(&wt, &remote);
        }
    }

    // Worker rules dump
    let rules_dir = crate::workspace::workspace_root().join("rules");
    let rf = runs_dir.join(format!("{}.rules.md", opts.name));
    if let Some(parent) = rf.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut rules_content = String::new();
    let all_rules = rules_dir.join("_all");
    if let Ok(entries) = std::fs::read_dir(all_rules) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(c) = std::fs::read_to_string(&p) {
                    rules_content.push_str(&c);
                }
            }
        }
    }
    let repo_name = repo.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
    let repo_rules = rules_dir.join(repo_name);
    if let Ok(entries) = std::fs::read_dir(repo_rules) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(c) = std::fs::read_to_string(&p) {
                    rules_content.push_str(&c);
                }
            }
        }
    }
    let _ = std::fs::write(&rf, &rules_content);

    let grid_tab = std::env::var("GRID_TAB").unwrap_or_else(|_| format!("{repo_name}-workers"));
    let _ = std::fs::write(runs_dir.join(format!("{}.owner", opts.name)), format!("{owner}\n"));
    let _ = std::fs::write(runs_dir.join(format!("{}.grid", opts.name)), format!("{grid_tab}\n"));
    if let Ok(pt) = std::env::var("PIPELINE_TASK") {
        let _ = std::fs::write(runs_dir.join(format!("{}.pipeline", opts.name)), format!("{pt}\n"));
    }

    let iso_now = get_iso_now();
    let srt_json = format!(
        "{{\"name\":\"{}\",\"kind\":\"{}\",\"created\":\"{}\",\"sandbox\":true,\"owner\":\"{}\",\"review\":{}}}\n",
        opts.name, opts.kind, iso_now, owner, opts.review
    );
    let _ = std::fs::write(runs_dir.join(format!("{}.srt.json", opts.name)), srt_json);

    // Split pane
    let rf_str = rf.to_string_lossy().to_string();
    let mut split_envs = Vec::new();
    let oc_env_val;
    if kind_info.harness == "opencode" {
        oc_env_val = format!(
            "{{\"model\":\"{}\",\"instructions\":[\"{}\"]}}",
            kind_info.model,
            rf_str
        );
        split_envs.push(("OPENCODE_CONFIG_CONTENT", oc_env_val.as_str()));
    }

    let split_out = herdr
        .pane_split(&wt.to_string_lossy(), &owner, &split_envs)
        .map_err(|e| (1, format!("error: herdr pane split failed: {e}\n")))?;

    if !split_out.status.success() {
        return Err((1, String::from_utf8_lossy(&split_out.stderr).to_string()));
    }

    let split_json_str = String::from_utf8_lossy(&split_out.stdout);
    let pane_id = parse_json(&split_json_str)
        .ok()
        .and_then(|j| j.get("result").and_then(|r| r.get("pane")).and_then(|p| p.get_str("pane_id")).map(String::from))
        .unwrap_or_default();

    if pane_id.is_empty() {
        return Err((1, "error: herdr pane split returned no pane_id\n".to_string()));
    }

    let spawn_fail = |pane: &str, code: i32| -> (i32, String) {
        let _ = herdr.pane_close(pane);
        (code, String::new())
    };

    // Report initial metadata tokens. `grid_tab` carries GRID_TAB (the workers
    // tab label, which need not equal the repo-derived group) so the planner,
    // the repair and the pre-prompt check can accept that tab by its exact
    // label instead of guessing from the group token.
    let initial_tokens = [
        "grid_role=worker",
        &format!("grid_group={repo_name}"),
        &format!("grid_owner={owner}"),
        &format!("grid_tab={grid_tab}"),
    ];
    let rep_res = herdr.pane_report_metadata(&pane_id, &initial_tokens);
    if rep_res.is_err() || !rep_res.unwrap().status.success() {
        return Err(spawn_fail(&pane_id, 1));
    }

    // Start agent based on team.toml harness, model, and effort
    let harness = kind_info.harness.as_str();
    let model = kind_info.model.as_str();
    let effort = kind_info.effort.as_deref().unwrap_or("medium");

    let start_res = match harness {
        "opencode" => {
            herdr.agent_start(&opts.name, "opencode", &pane_id, 60000, &["--", "--standalone", "--auto"])
        }
        "omp" => {
            let omp_effort = kind_info.effort.as_deref().unwrap_or("high");
            herdr.agent_start(
                &opts.name,
                "omp",
                &pane_id,
                60000,
                &[
                    "--",
                    "--model",
                    model,
                    "--thinking",
                    omp_effort,
                    "--auto-approve",
                    "--append-system-prompt",
                    &rf_str,
                ],
            )
        }
        "claude" => {
            let claude_model = if model.starts_with("claude-") {
                model.to_string()
            } else {
                format!("claude-{}", model.replace('.', "-"))
            };
            let mut extra_args: Vec<String> = vec![
                "--".to_string(),
                "--model".to_string(),
                claude_model,
                "--effort".to_string(),
                effort.to_string(),
            ];
            if opts.review {
                let runs_str = runs_dir.to_string_lossy().to_string();
                let wt_str = wt.to_string_lossy().to_string();
                let runs_clean = runs_str.trim_start_matches('/');
                let wt_clean = wt_str.trim_start_matches('/');
                let settings = format!(
                    "{{\"permissions\":{{\"allow\":[\"Edit(//{}/**)\",\"Bash(cargo test:*)\",\"Bash(mv *runs/.*.tmp *runs/*.md)\"],\"deny\":[\"Edit(//{}/**)\"]}}}}",
                    runs_clean, wt_clean
                );
                extra_args.push("--permission-mode".to_string());
                extra_args.push("dontAsk".to_string());
                extra_args.push("--add-dir".to_string());
                extra_args.push(runs_str);
                extra_args.push("--settings".to_string());
                extra_args.push(settings);
            }
            extra_args.push("--append-system-prompt-file".to_string());
            extra_args.push(rf_str.clone());

            let extra_slices: Vec<&str> = extra_args.iter().map(String::as_str).collect();
            herdr.agent_start(&opts.name, "claude", &pane_id, 60000, &extra_slices)
        }
        _ => {
            return Err(spawn_fail(&pane_id, 1));
        }
    };

    let start_failed = match &start_res {
        Ok(out) => !out.status.success(),
        Err(_) => true,
    };

    if harness == "claude" {
        // Claude Code's first session in a directory tree it has not been
        // trusted in shows a workspace-trust dialog ("Accessing workspace: ...
        // / Quick safety check: is this a project you created or one you
        // trust?") with "No, exit" focused; the worker's brief would be typed
        // into it and lost. The trust decision is per repo and Claude Code
        // writes it down on acceptance, so this costs the wait-output timeout
        // only until the repo is trusted. herdr's own dialog rules do not know
        // this text, so it can report the agent ready — or 'blocked during
        // startup' when the start is slower — with the dialog still up: accept
        // it here the way a person would (one Down to "Yes, I trust this
        // folder", then Enter), then let it clear before continuing.
        if herdr.pane_wait_output(&pane_id, "trust", 15000) {
            let _ = herdr.pane_send_keys(&pane_id, "down");
            let _ = herdr.pane_send_keys(&pane_id, "enter");
            let mut dialog_up = false;
            for _ in 0..8 {
                dialog_up = claude_trust_dialog_up(herdr, &pane_id);
                if !dialog_up {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            if dialog_up {
                let _ = herdr.pane_close(&pane_id);
                return Err((
                    1,
                    format!(
                        "error: claude workspace-trust dialog did not clear in {pane_id} after accepting it — nothing was sent to {}; inspect the pane, then retry the spawn\n",
                        opts.name
                    ),
                ));
            }
        }
        if start_failed {
            // herdr's start timeout is not proof of failure for claude either:
            // herdr only sees 'timed out waiting for agent startup' when the
            // dialog or a slow SessionStart hook held the start back, so wait
            // for idle rather than abandoning a worker that is merely slow.
            let ok = herdr
                .agent_wait_idle(&opts.name, 120000)
                .map(|out| out.status.success())
                .unwrap_or(false);
            if !ok {
                return Err(spawn_fail(&pane_id, 1));
            }
        }
    } else if start_failed {
        return Err(spawn_fail(&pane_id, 1));
    }

    let ready_res = herdr.pane_report_metadata(&pane_id, &["grid_ready=1"]);
    if ready_res.is_err() || !ready_res.unwrap().status.success() {
        return Err(spawn_fail(&pane_id, 1));
    }

    println!("worker={} kind={} pane={} worktree={}", opts.name, opts.kind, pane_id, wt.display());
    run_grid_hook(&grid_tab);

    Ok(())
}
