//! Worker watchdog loop, checking stall, idle, budget deadline, grace, autoext, and validating report format.

use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::state::WorkerMetadata;
use crate::util::now_sec;
use crate::worker::prompt::cost::CostTracker;
use crate::worker::prompt::observer::{
    is_provider_timeout, lead_waiting, observe, question_seen, report_ready, Observation,
};
use std::path::Path;
use std::process::Command;

pub fn nap(sec: u64) {
    if let Ok(sleep_cmd) = std::env::var("PROMPT_WORKER_SLEEP_CMD") {
        let _ = Command::new(sleep_cmd).arg(sec.to_string()).status();
    } else {
        std::thread::sleep(std::time::Duration::from_secs(sec));
    }
}

/// Prepend a header line to a report unless it is already present. Returns
/// `None` when the line is already there (idempotent stamping).
pub fn prepend_header(report: &str, line: &str) -> Option<String> {
    if report.lines().any(|l| l == line) {
        return None;
    }
    Some(format!("{line}\n{report}"))
}

/// Matches Lessons headings in markdown reports.
/// Accepts exact `## Lessons` as well as numbered headings like `## 5. Lessons`.
pub fn is_lessons_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(after_hash) = trimmed.strip_prefix("##") else {
        return false;
    };
    // Reject deeper headings (###, ####, ...)
    if after_hash.starts_with('#') {
        return false;
    }
    // Must be followed by space or tab
    if !after_hash.starts_with(' ') && !after_hash.starts_with('\t') {
        return false;
    }
    let rest = after_hash.trim_start();
    if let Some(rem) = rest.strip_prefix("Lessons") {
        return rem.is_empty() || !rem.chars().next().unwrap().is_alphabetic();
    }
    // Check for numbered prefix: e.g. "5. Lessons", "5) Lessons", "5 Lessons"
    let digits_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits_len > 0 {
        let after_digits = &rest[digits_len..];
        let after_punct = after_digits
            .strip_prefix('.')
            .or_else(|| after_digits.strip_prefix(')'))
            .unwrap_or(after_digits);
        let after_num = after_punct.trim_start();
        if let Some(rem) = after_num.strip_prefix("Lessons") {
            return rem.is_empty() || !rem.chars().next().unwrap().is_alphabetic();
        }
    }
    false
}

/// Validates the structure of the finished run report.
pub fn validate_report_content(
    report_content: &str,
    expect_verdict: bool,
) -> Result<(), &'static str> {
    if expect_verdict {
        let has_verdict = report_content
            .lines()
            .any(|l| l.trim_start().starts_with("Verdict:"));
        if !has_verdict {
            return Err("'Verdict:' line");
        }
    } else {
        let has_lessons = report_content.lines().any(|l| is_lessons_heading(l));
        if !has_lessons {
            return Err("'## Lessons' section");
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct WatchdogParams {
    pub max: u64,
    pub poll: u64,
    pub stall: u64,
    pub idle_sec: u64,
    pub idle_fallback_sec: u64,
    pub gone_sec: u64,
    pub nudge_pct: u64,
    pub heartbeat: u64,
    pub land_sec: u64,
    pub time_budget_sec: u64,
    pub prev_elapsed: u64,
    pub cost_budget: Option<f64>,
    pub resume: bool,
}

impl WatchdogParams {
    pub fn from_env(
        time_budget_sec: u64,
        prev_elapsed: u64,
        cost_budget: Option<f64>,
        resume: bool,
    ) -> Self {
        let max: u64 = std::env::var("PROMPT_WORKER_MAX_SEC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(7200);
        let poll: u64 = std::env::var("PROMPT_WORKER_POLL_SEC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30)
            .max(1);
        let stall: u64 = std::env::var("PROMPT_WORKER_STALL_SEC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(600);
        let idle_sec: u64 = std::env::var("PROMPT_WORKER_IDLE_SEC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        let idle_fallback_sec: u64 = std::env::var("PROMPT_WORKER_IDLE_FALLBACK_SEC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300.max(idle_sec));
        let gone_sec: u64 = std::env::var("PROMPT_WORKER_AGENT_GONE_SEC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        let nudge_pct: u64 = std::env::var("PROMPT_WORKER_NUDGE_PCT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(80);
        let heartbeat: u64 = std::env::var("PROMPT_WORKER_HEARTBEAT_SEC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);
        let land_sec: u64 = std::env::var("PROMPT_WORKER_LAND_SEC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15);

        Self {
            max,
            poll,
            stall,
            idle_sec,
            idle_fallback_sec,
            gone_sec,
            nudge_pct,
            heartbeat,
            land_sec,
            time_budget_sec,
            prev_elapsed,
            cost_budget,
            resume,
        }
    }
}

pub fn run_watchdog<G: GitOps, H: HerdrOps>(
    name: &str,
    runs_dir: &Path,
    params: &WatchdogParams,
    git: &G,
    herdr: &H,
) -> Result<(), (i32, String)> {
    let out = runs_dir.join(format!("{name}.md"));
    let mut cost_tracker = CostTracker::new(runs_dir, name, params.cost_budget);

    let mut autoext_total: u64 = 0;
    let mut last_autoext_dh = String::new();
    let mut initial_dh_recorded = false;

    let start = now_sec();
    let rem_max = (params.max.saturating_sub(params.prev_elapsed)).max(1);
    let rem_budget = (params.time_budget_sec.saturating_sub(params.prev_elapsed)).max(1);
    let deadline = start + rem_max;
    let mut budget_deadline = start + rem_budget;
    let nudge_at = start
        + (params.time_budget_sec * params.nudge_pct / 100).saturating_sub(params.prev_elapsed);

    let save_elapsed = |now: u64| {
        let el = params.prev_elapsed + now.saturating_sub(start);
        let _ = WorkerMetadata::save_elapsed(runs_dir, name, el);
    };

    let watchdog_exit = |code: i32,
                         reason: &str,
                         obs: &Option<Observation>,
                         change_ts: u64|
     -> (i32, String) {
        let now = now_sec();
        save_elapsed(now);
        let status = obs.as_ref().map(|o| o.status.as_str()).unwrap_or("unknown");
        let ctx = obs.as_ref().map(|o| o.ctx.as_str()).unwrap_or("?");
        let quiet = now.saturating_sub(change_ts);
        let last_tool = obs
            .as_ref()
            .map(|o| o.last_tool.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("none");
        let diffstat = obs
            .as_ref()
            .map(|o| o.diffstat.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("n/a");
        let elapsed = now.saturating_sub(start);
        eprintln!(
            "WATCHDOG exit={code} {reason} | name={name} elapsed={elapsed}s status={status} ctx={ctx} quiet={quiet}s last={last_tool} | diff={diffstat}"
        );
        (code, String::new())
    };

    let malformed_report = |missing: &str| -> (i32, String) {
        let now = now_sec();
        save_elapsed(now);
        eprintln!(
            "WATCHDOG malformed-report | name={name} missing={missing} report={}",
            out.display()
        );
        (6, String::new())
    };

    let mut prev_fp = String::new();
    let mut fp_seen = false;
    let mut change_ts = start;
    let mut idle_since = 0;
    let mut last_hb = start;
    let mut nudged = WorkerMetadata::has_nudged(runs_dir, name);
    let mut grace_used = false;
    let mut gone_since = 0;
    let mut blind_warned = false;
    // Prompt-landing state (fresh prompts only; reset by --resume).
    let land_deadline = start + params.land_sec;
    let mut prompt_landed = params.resume || params.land_sec == 0;
    let mut prompt_recovered = false;
    let mut recover_at: u64 = 0;
    let mut recover_deadline: u64 = 0;
    let mut last_progress: u64 = WorkerMetadata::load_progress(runs_dir, name);

    eprintln!(
        "waiting for {} (poll {}s, stall {}s, idle {}s, budget {}s, cap {}s)",
        out.display(),
        params.poll,
        params.stall,
        params.idle_sec,
        params.time_budget_sec,
        params.max
    );

    let mut last_obs: Option<Observation> = None;

    while !report_ready(&out) {
        let now = now_sec();
        if now >= deadline {
            return Err(watchdog_exit(
                2,
                &format!("error: no {} after {}s", out.display(), params.max),
                &last_obs,
                change_ts,
            ));
        }

        if let Some(obs) = observe(herdr, git, name) {
            let st = obs.status.clone();
            let is_waiting = lead_waiting(runs_dir, name, &obs.watcher_pane);
            cost_tracker.check(runs_dir, name, &obs);

            if st == "gone" || (st == "unknown" && obs.pane_text.is_empty()) {
                if gone_since == 0 {
                    gone_since = now;
                }
                if now - gone_since >= params.gone_sec {
                    return Err(watchdog_exit(
                        3,
                        &format!("agent gone: status={st} for {}s", now - gone_since),
                        &Some(obs),
                        change_ts,
                    ));
                }
            } else {
                gone_since = 0;
            }
            if (st == "idle" || st == "done")
                && !WorkerMetadata::has_provider_retry(runs_dir, name)
                && is_provider_timeout(&obs.pane_text)
            {
                let _ = WorkerMetadata::record_provider_retry(runs_dir, name, now);
                let _ = herdr.agent_prompt_nowait(name, "continue");
                eprintln!("WATCHDOG provider timeout detected | name={name} — re-prompted session with 'continue'");
                idle_since = 0;
            }

            if st == "blocked" {
                return Err(watchdog_exit(
                    5,
                    "agent blocked (question/permission pending)",
                    &Some(obs),
                    change_ts,
                ));
            }
            if question_seen(&obs.pane_text) {
                return Err(watchdog_exit(
                    5,
                    "pane shows a pending question/permission prompt",
                    &Some(obs),
                    change_ts,
                ));
            }

            // Prompt landing: a fresh prompt that leaves the agent idle and
            // silent never reached the pane. Recover once with a stray Enter
            // (submits text typed but not sent); still idle -> exit 3.
            if !prompt_landed {
                if st == "working" || st == "blocked" {
                    prompt_landed = true;
                } else if st == "idle" || st == "done" {
                    if is_waiting {
                        prompt_landed = true;
                    } else if prompt_recovered {
                        if now >= recover_deadline {
                            return Err(watchdog_exit(
                                3,
                                &format!(
                                    "prompt not landed: agent {st} for {}s after a send-keys enter",
                                    now - recover_at
                                ),
                                &Some(obs),
                                change_ts,
                            ));
                        }
                    } else if now >= land_deadline {
                        let _ = herdr.agent_send_keys(name, "enter");
                        prompt_recovered = true;
                        recover_at = now;
                        recover_deadline = now + params.land_sec;
                        eprintln!(
                            "WATCHDOG prompt-not-landed | name={name} {st} {}s after prompt — sent enter once (input may be typed but unsubmitted)",
                            now - start
                        );
                    }
                }
            }

            if obs.fp != prev_fp {
                prev_fp = obs.fp.clone();
                if fp_seen {
                    change_ts = now;
                    last_progress = now;
                    idle_since = 0;
                    let _ = WorkerMetadata::save_progress(runs_dir, name, now);
                }
                fp_seen = true;
            }
            if !initial_dh_recorded && !obs.diff_hash.is_empty() {
                last_autoext_dh = obs.diff_hash.clone();
                initial_dh_recorded = true;
            }

            if now >= budget_deadline {
                if !grace_used && last_progress > 0 && now - last_progress <= params.poll * 2 {
                    let mut grace = (params.time_budget_sec / 4).max(1);
                    if budget_deadline + grace > deadline {
                        grace = deadline.saturating_sub(budget_deadline);
                    }
                    if grace > 0 {
                        budget_deadline += grace;
                        grace_used = true;
                        // grace record removed (journal authoritative)
                        eprintln!("WATCHDOG grace +{grace}s (progressing)");
                    }
                }
                let max_autoext = params.time_budget_sec / 2;
                if autoext_total < max_autoext
                    && !obs.diff_hash.is_empty()
                    && obs.diff_hash != last_autoext_dh
                {
                    let rem = max_autoext.saturating_sub(autoext_total);
                    let mut ext = (params.time_budget_sec / 4).max(1).min(rem);
                    if budget_deadline + ext > deadline {
                        ext = deadline.saturating_sub(budget_deadline);
                    }
                    if ext > 0 {
                        budget_deadline += ext;
                        autoext_total += ext;
                        last_autoext_dh = obs.diff_hash.clone();
                        // autoext record removed (journal authoritative)
                        eprintln!("WATCHDOG auto-extend +{ext}s (diff progressing, cap {}s, total +{autoext_total}s/+{max_autoext}s)", params.max);
                    }
                }
                if now >= budget_deadline {
                    return Err(watchdog_exit(
                        4,
                        &format!("time budget {}s exceeded", params.time_budget_sec),
                        &Some(obs),
                        change_ts,
                    ));
                }
            }

            if is_waiting {
                idle_since = 0;
            } else if st == "idle" || st == "done" {
                if idle_since == 0 {
                    idle_since = now;
                }
                let effective_idle =
                    if !obs.hook_active && (obs.harness == "opencode" || obs.harness == "omp") {
                        params.idle_fallback_sec
                    } else {
                        params.idle_sec
                    };
                if now - idle_since >= effective_idle {
                    return Err(watchdog_exit(
                        3,
                        &format!("agent {st} for {}s without run file", now - idle_since),
                        &Some(obs),
                        change_ts,
                    ));
                }
            } else {
                idle_since = 0;
            }

            if !is_waiting && now - change_ts >= params.stall {
                return Err(watchdog_exit(
                    3,
                    &format!("stall: no pane/worktree change for {}s", now - change_ts),
                    &Some(obs),
                    change_ts,
                ));
            }

            last_obs = Some(obs);
        } else {
            if gone_since == 0 {
                gone_since = now;
            }
            if now - gone_since >= params.gone_sec {
                return Err(watchdog_exit(
                    3,
                    &format!(
                        "agent gone: unobservable (no get/read) for {}s",
                        now - gone_since
                    ),
                    &last_obs,
                    change_ts,
                ));
            }
            if !blind_warned {
                eprintln!(
                    "WATCHDOG observe failed (no agent get/read); only cap/budget/agent-gone apply"
                );
                blind_warned = true;
            }
            if now >= budget_deadline {
                return Err(watchdog_exit(
                    4,
                    &format!(
                        "time budget {}s exceeded (agent unobservable)",
                        params.time_budget_sec
                    ),
                    &last_obs,
                    change_ts,
                ));
            }
        }

        if !nudged && now >= nudge_at {
            let nudge_msg = format!(
                "Time check: you are past {}% of this task's time budget. Wrap up now: finish the current step, commit your work, and write ~/quaqua/runs/{name}.md (via a temp file + rename) with what is done plus a Blockers section for the rest. Reply only the path.",
                params.nudge_pct
            );
            let _ = herdr.agent_prompt_nowait(name, &nudge_msg);
            let _ = WorkerMetadata::record_nudged(runs_dir, name);
            nudged = true;
            eprintln!(
                "WATCHDOG nudge sent (once, at {}% of {}s budget)",
                params.nudge_pct, params.time_budget_sec
            );
        }

        if now - last_hb >= params.heartbeat {
            let status = last_obs
                .as_ref()
                .map(|o| o.status.as_str())
                .unwrap_or("unknown");
            let ctx = last_obs.as_ref().map(|o| o.ctx.as_str()).unwrap_or("?");
            let quiet = now.saturating_sub(change_ts);
            let last_tool = last_obs
                .as_ref()
                .map(|o| o.last_tool.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("none");
            let elapsed = now.saturating_sub(start);
            eprintln!("WATCHDOG heartbeat | name={name} elapsed={elapsed}s status={status} ctx={ctx} quiet={quiet}s last={last_tool}");
            last_hb = now;
        }

        let left = (deadline.saturating_sub(now)).max(1);
        nap(left.min(params.poll));
    }

    // Validate report shape
    let expect_verdict = WorkerMetadata::has_review(runs_dir, name);

    let report_content = std::fs::read_to_string(&out).unwrap_or_default();
    if let Err(missing) = validate_report_content(&report_content, expect_verdict) {
        return Err(malformed_report(missing));
    }

    // spawn must never create runs/<name>.md (its existence is the completion
    // signal), so the `fallback:` header is stamped here, after the report is
    // validated. Idempotent; atomic tmp+rename.

    let now = now_sec();
    save_elapsed(now);
    println!("{}", out.display());
    Ok(())
}
