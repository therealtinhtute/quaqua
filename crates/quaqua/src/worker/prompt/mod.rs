//! Worker prompt & watchdog logic (`quaqua worker prompt <name> <task>`).

pub mod cost;
pub mod observer;
pub mod watchdog;

pub use cost::{brief_cost_budget, read_worker_cost, resolve_cost_budget};
pub use observer::{is_provider_timeout, observe, Observation};
pub use watchdog::{
    fallback_header_line, is_lessons_heading, nap, prepend_header, run_watchdog,
    validate_report_content, WatchdogParams,
};
pub use crate::util::now_sec;

use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::state::WorkerMetadata;
use crate::worker::cleanup::default_runs_dir;

pub const USAGE: &str = "\
# Usage: quaqua worker prompt <name> <task>        submit task brief, wait for runs file
#        quaqua worker prompt --resume <name> [--extend <sec>]
#                                             keep waiting; --extend adds <sec> to
#                                             this name's allowance (persisted in
#                                             runs/.<name>.budget; the cap still wins)
#        quaqua worker prompt --check-brief \"<task>\"   validate a brief + print its budget";

pub fn brief_headers_ok(text: &str) -> bool {
    text.contains("Goal:")
        && text.contains("Hard constraints:")
        && (text.contains("Current design (open to challenge):")
            || text.contains("Current design:"))
}

pub fn brief_budget_sec(text: &str) -> Option<u64> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        let mut search_from = 0;
        while let Some(idx) = lower[search_from..].find("time budget:") {
            let start = search_from + idx + 12;
            let rest = &lower[start..];
            search_from = start;

            // [^0-9]{0,8} using byte offsets via char_indices to avoid slicing inside multi-byte characters
            let mut digit_start = None;
            let mut non_digit_chars = 0;
            for (byte_offset, c) in rest.char_indices() {
                if c.is_ascii_digit() {
                    digit_start = Some((byte_offset, non_digit_chars));
                    break;
                }
                non_digit_chars += 1;
                if non_digit_chars > 8 {
                    break;
                }
            }

            let Some((byte_offset, _)) = digit_start else {
                continue;
            };

            let after_prefix = &rest[byte_offset..];

            // [0-9]+
            let digit_bytes = after_prefix
                .bytes()
                .take_while(|b| b.is_ascii_digit())
                .count();
            if digit_bytes == 0 {
                continue;
            }

            let num_str = &after_prefix[..digit_bytes];
            let Ok(n) = num_str.parse::<u64>() else {
                continue;
            };

            // [[:space:]]*
            let after_digits = after_prefix[digit_bytes..].trim_start();

            // (min|mins|minutes|m)([^[:alnum:]]|$)
            let matched_unit = if let Some(rem) = after_digits.strip_prefix("minutes") {
                Some(rem)
            } else if let Some(rem) = after_digits.strip_prefix("mins") {
                Some(rem)
            } else if let Some(rem) = after_digits.strip_prefix("min") {
                Some(rem)
            } else if let Some(rem) = after_digits.strip_prefix('m') {
                Some(rem)
            } else {
                None
            };

            if let Some(rem) = matched_unit {
                let valid_end = match rem.chars().next() {
                    None => true,
                    Some(c) => !c.is_alphanumeric(),
                };
                if valid_end {
                    return Some(n * 60);
                }
            }
        }
    }
    None
}

pub fn validate_env_budget_val(val: Option<&str>) -> Result<(), (i32, String)> {
    if let Some(v) = val {
        if !v.is_empty() && !v.chars().all(|c| c.is_ascii_digit()) {
            return Err((
                1,
                format!("error: PROMPT_WORKER_BUDGET_SEC must be whole seconds (got '{v}')\n"),
            ));
        }
    }
    Ok(())
}

pub fn validate_env_budget() -> Result<(), (i32, String)> {
    let var = std::env::var("PROMPT_WORKER_BUDGET_SEC").ok();
    validate_env_budget_val(var.as_deref())
}

pub fn resolve_budget_sec(task: &str) -> (u64, &'static str) {
    if let Ok(env_val) = std::env::var("PROMPT_WORKER_BUDGET_SEC") {
        if !env_val.is_empty() {
            if let Ok(n) = env_val.parse::<u64>() {
                return (n, "env");
            }
        }
    }
    if let Some(n) = brief_budget_sec(task) {
        return (n, "brief");
    }
    (2700, "default")
}

pub fn do_check_brief(task: &str) -> Result<(), (i32, String)> {
    validate_env_budget()?;
    if !brief_headers_ok(task) {
        return Err((
            1,
            "error: task must be a brief with 'Goal:', 'Hard constraints:', 'Current design (open to challenge):' (rules/_all/brief.md)\n".to_string(),
        ));
    }
    let (budget, src) = resolve_budget_sec(task);
    println!("budget={budget}s source={src}");
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum PromptAction {
    CheckBrief(String),
    Resume {
        name: String,
        extend: Option<u64>,
    },
    Prompt {
        name: String,
        task: String,
    },
}

pub fn parse_prompt_args(p_args: &[String]) -> Result<PromptAction, (i32, String)> {
    if p_args.is_empty() {
        return Err((1, format!("{USAGE}\n")));
    }
    if p_args[0] == "--check-brief" {
        if p_args.len() != 2 {
            return Err((1, format!("{USAGE}\n")));
        }
        return Ok(PromptAction::CheckBrief(p_args[1].clone()));
    }

    if p_args[0] == "--resume" {
        if p_args.len() < 2 {
            return Err((1, format!("{USAGE}\n")));
        }
        let name = p_args[1].clone();
        let mut extend: Option<u64> = None;
        if p_args.len() > 2 {
            if p_args[2] == "--extend" {
                if p_args.len() != 4 {
                    return Err((1, format!("{USAGE}\n")));
                }
                let sec: u64 = p_args[3].parse().map_err(|_| {
                    (1, "error: --extend <sec> requires a positive integer\n".to_string())
                })?;
                extend = Some(sec);
            } else {
                return Err((1, format!("{USAGE}\n")));
            }
        }
        return Ok(PromptAction::Resume { name, extend });
    }

    // Normal prompt: <name> <task>
    if p_args.len() != 2 {
        return Err((1, format!("{USAGE}\n")));
    }
    let name = p_args[0].clone();
    let task = p_args[1].clone();
    if task.trim().is_empty() {
        return Err((1, "error: empty task brief\n".to_string()));
    }
    if !brief_headers_ok(&task) {
        return Err((
            1,
            "error: task must be a brief with 'Goal:', 'Hard constraints:', 'Current design (open to challenge):' (rules/_all/brief.md)\n".to_string(),
        ));
    }
    Ok(PromptAction::Prompt { name, task })
}

pub fn prompt_worker<G: GitOps, H: HerdrOps>(
    name: &str,
    task: Option<&str>,
    resume: bool,
    extend: Option<u64>,
    git: &G,
    herdr: &H,
) -> Result<(), (i32, String)> {
    validate_env_budget()?;

    let runs_dir = default_runs_dir();
    let _ = std::fs::create_dir_all(&runs_dir);

    let out = runs_dir.join(format!("{name}.md"));
    let submit_log = WorkerMetadata::submit_log_path(&runs_dir, name);
    let cost_budget: Option<f64> = resolve_cost_budget(task.unwrap_or(""));

    let prev_elapsed: u64;
    let mut budget: u64;

    let max: u64 = std::env::var("PROMPT_WORKER_MAX_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7200);

    if resume {
        prev_elapsed = WorkerMetadata::load_elapsed(&runs_dir, name);
        let saved_budget = WorkerMetadata::load_budget(&runs_dir, name);
        budget = saved_budget.unwrap_or_else(|| resolve_budget_sec("").0);
        if let Some(ext) = extend {
            budget += ext;
            let _ = WorkerMetadata::save_budget(&runs_dir, name, budget);
            eprintln!("WATCHDOG extend +{ext}s (allowance {budget}s, cap {max}s)");
        }
    } else {
        let task_str = task.unwrap_or("");
        prev_elapsed = 0;
        budget = resolve_budget_sec(task_str).0;
        let _ = WorkerMetadata::save_budget(&runs_dir, name, budget);
        WorkerMetadata::reset_prompt_state(&runs_dir, name);

        if out.exists() {
            let stale_path = format!("{}.stale.{}", out.display(), now_sec());
            let _ = std::fs::rename(&out, &stale_path);
            eprintln!("archived pre-existing {} — a new task needs a fresh report", out.display());
        }

        // Pre-prompt grid check
        if std::env::var("GRID_SKIP").as_deref() != Ok("1")
            || (std::env::var("PIPELINE_TASK").is_err() && std::env::var("QUAQUA_TEST").as_deref() != Ok("1"))
        {
            let grid_tab = std::fs::read_to_string(runs_dir.join(format!("{name}.grid")))
                .unwrap_or_default();
            let tab = {
                let t = grid_tab.trim();
                if t.is_empty() { "workers" } else { t }
            };
            if let Err(e) = crate::grid::run_grid_check(tab, Some(name), None) {
                return Err((1, format!("error: grid check failed for {name} ({e}); arrange panes manually before prompting\n")));
            }
        }

        // Background prompt dispatch
        let prompt_body = format!(
            "{task_str}. When done, write summary (changes, test cmd+result, risks) to a temp file and rename it to ~/quaqua/runs/{name}.md (publish it atomically, never a partial file); reply only the path."
        );
        let submit_file = std::fs::File::create(&submit_log).ok();
        if let Some(file) = submit_file {
            let _ = herdr.agent_prompt_wait_background(name, &prompt_body, 1800000, file);
        }
    }

    let params = WatchdogParams::from_env(budget, prev_elapsed, cost_budget, resume);
    run_watchdog(name, &runs_dir, &params, git, herdr)
}
