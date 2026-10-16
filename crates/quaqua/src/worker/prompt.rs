//! Worker prompt & watchdog logic (`quaqua worker prompt <name> <task>`).
//! Reproduces `scripts/worker/prompt-worker.sh`.

use crate::git::Git;
use crate::hash::md5_hex;
use crate::herdr::Herdr;
use crate::json::parse_json;
use crate::worker::cleanup::default_runs_dir;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const USAGE: &str = "\
# Usage: prompt-worker.sh <name> <task>        submit task brief, wait for runs file
#        prompt-worker.sh --resume <name> [--extend <sec>]
#                                             keep waiting; --extend adds <sec> to
#                                             this name's allowance (persisted in
#                                             runs/.<name>.budget; the cap still wins)
#        prompt-worker.sh --check-brief \"<task>\"   validate a brief + print its budget";

pub fn brief_headers_ok(text: &str) -> bool {
    text.contains("Goal:")
        && text.contains("Hard constraints:")
        && (text.contains("Current design (open to challenge):")
            || text.contains("Current design:"))
}

pub fn brief_budget_sec(text: &str) -> Option<u64> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        if let Some(idx) = lower.find("time budget:") {
            let rest = &lower[idx + 12..];
            let mut num_str = String::new();
            for c in rest.chars() {
                if c.is_ascii_digit() {
                    num_str.push(c);
                } else if !num_str.is_empty() {
                    break;
                }
            }
            if let Ok(n) = num_str.parse::<u64>() {
                if rest.contains("min") || rest.contains("m") {
                    return Some(n * 60);
                }
            }
        }
    }
    None
}

pub fn resolve_budget_sec(task: &str) -> (u64, &'static str) {
    if let Ok(env_val) = std::env::var("PROMPT_WORKER_BUDGET_SEC") {
        if let Ok(n) = env_val.parse::<u64>() {
            return (n, "env");
        }
    }
    if let Some(n) = brief_budget_sec(task) {
        return (n, "brief");
    }
    (2700, "default")
}

pub fn now_sec() -> u64 {
    if let Ok(clock_file) = std::env::var("PROMPT_WORKER_CLOCK_FILE") {
        if let Ok(c) = std::fs::read_to_string(&clock_file) {
            if let Ok(v) = c.trim().parse::<u64>() {
                return v;
            }
        }
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn nap(sec: u64) {
    if let Ok(sleep_cmd) = std::env::var("PROMPT_WORKER_SLEEP_CMD") {
        let _ = Command::new(sleep_cmd).arg(sec.to_string()).status();
    } else {
        std::thread::sleep(std::time::Duration::from_secs(sec));
    }
}

pub fn do_check_brief(task: &str) -> Result<(), (i32, String)> {
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

fn question_seen(pane_text: &str) -> bool {
    let mut clean_lines: Vec<&str> = Vec::new();
    for line in pane_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let is_decorative = trimmed.chars().all(|c| {
            "─│|═━┈┉┄┅┌┐└┘├┤┬┴┼╭╮╯╰ 0123456789%·".contains(c)
        });
        if is_decorative
            || trimmed.contains('👾')
            || trimmed.contains('✦')
            || trimmed.chars().any(|c| "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".contains(c))
        {
            continue;
        }
        clean_lines.push(trimmed);
    }

    let tail_count = clean_lines.len().min(3);
    let tail = &clean_lines[clean_lines.len() - tail_count..];

    for line in tail {
        let l = line.to_lowercase();
        if l.contains("type something")
            || l.contains("esc to abandon")
            || l.contains("esc to cancel")
            || l.contains("esc to undo")
            || l.contains("do you want to")
            || l.contains("needs approval")
            || l.contains("needs permission")
            || l.contains("requires approval")
            || l.contains("requires permission")
            || l.contains("[y/n]")
            || l.contains("[y/n]")
        {
            return true;
        }
        // [y/N] or y/n bounded by non-alphabetic
        if let Some(idx) = l.find("y/n") {
            let before = if idx == 0 { ' ' } else { l.chars().nth(idx - 1).unwrap_or(' ') };
            let after = l.chars().nth(idx + 3).unwrap_or(' ');
            if !before.is_alphabetic() && !after.is_alphabetic() {
                return true;
            }
        }
    }
    false
}

fn lead_waiting(runs_dir: &Path, name: &str, watcher_pane: &str) -> bool {
    if !name.ends_with("-lead") {
        return false;
    }
    if let Ok(entries) = std::fs::read_dir(runs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if fname.starts_with("pipeline-") && fname.ends_with(".state") {
                if let Ok(c) = std::fs::read_to_string(&path) {
                    if !c.lines().any(|l| l.trim() == "finished=0") {
                        continue;
                    }
                    let mut p_lead = "";
                    let mut p_pane = "";
                    let mut p_pid = "";
                    for line in c.lines() {
                        let l = line.trim();
                        if let Some(v) = l.strip_prefix("lead=") {
                            p_lead = v;
                        } else if let Some(v) = l.strip_prefix("lead_pane=") {
                            p_pane = v;
                        } else if let Some(v) = l.strip_prefix("pipeline_pid=") {
                            p_pid = v;
                        }
                    }
                    if !p_pid.is_empty() {
                        let is_alive = Command::new("kill").arg("-0").arg(p_pid).status().map(|s| s.success()).unwrap_or(false);
                        if !is_alive {
                            continue;
                        }
                        if p_lead == name || (!watcher_pane.is_empty() && p_pane == watcher_pane) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn report_stat(path: &Path) -> Option<(u64, u64)> {
    let md = std::fs::metadata(path).ok()?;
    let len = md.len();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((len, md.mtime() as u64))
    }
    #[cfg(not(unix))]
    {
        Some((len, 0))
    }
}

fn report_ready(path: &Path) -> bool {
    if let Some((len1, mtime1)) = report_stat(path) {
        if len1 > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400));
            if let Some((len2, mtime2)) = report_stat(path) {
                return len1 == len2 && mtime1 == mtime2;
            }
        }
    }
    false
}

pub struct Observation {
    pub status: String,
    pub ctx: String,
    pub last_tool: String,
    pub diffstat: String,
    pub pane_text: String,
    pub fp: String,
    pub watcher_pane: String,
}

pub fn observe(herdr: &Herdr, git: &Git, name: &str) -> Option<Observation> {
    let mut status = "unknown".to_string();
    let mut ctx = "?".to_string();
    let mut last_tool = String::new();
    let mut diffstat = String::new();
    let mut watcher_pane = String::new();
    let mut cwd = String::new();

    let (_ok, get_info) = herdr.agent_get(name).unwrap_or((false, String::new()));
    if let Ok(json) = parse_json(&get_info) {
        if let Some(agent) = json.get("result").and_then(|r| r.get("agent")) {
            status = agent.get_str("agent_status").unwrap_or("unknown").to_string();
            cwd = agent.get_str("cwd").unwrap_or("").to_string();
            watcher_pane = agent.get_str("pane_id").unwrap_or("").to_string();
        } else if let Some(err) = json.get("error") {
            if err.get_str("code") == Some("agent_not_found") {
                status = "gone".to_string();
            }
        }
    }
    if status == "unknown" && get_info.contains("agent_not_found") {
        status = "gone".to_string();
    }

    let mut pane_text = herdr
        .agent_read(name, "recent-unwrapped", 80)
        .unwrap_or_default();
    if pane_text.trim().is_empty() || pane_text.contains("{\"error\"") {
        pane_text = herdr
            .agent_read(name, "visible", 80)
            .unwrap_or_default();
    }
    if pane_text.contains("{\"error\"") {
        pane_text.clear();
    }

    if get_info.is_empty() && pane_text.is_empty() {
        return None;
    }

    // Body fingerprint: filter spinner/status rows, collapse digits to 'N'
    let mut body_lines = Vec::new();
    for line in pane_text.lines() {
        let t = line.trim();
        let is_sep = t.chars().all(|c| "─│| ".contains(c));
        if is_sep || t.contains('👾') || t.contains('✦') || t.chars().any(|c| "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".contains(c)) {
            continue;
        }
        let mut collapsed = String::new();
        let mut in_digits = false;
        for c in line.chars() {
            if c.is_ascii_digit() {
                if !in_digits {
                    collapsed.push('N');
                    in_digits = true;
                }
            } else {
                in_digits = false;
                collapsed.push(c);
            }
        }
        body_lines.push(collapsed);
    }
    let body_text = body_lines.join("\n");
    let body_hash = md5_hex(body_text.as_bytes());

    for line in pane_text.lines() {
        if let Some(idx) = line.find('%') {
            let mut num = String::new();
            for c in line[..idx].chars().rev() {
                if c.is_ascii_digit() {
                    num.push(c);
                } else {
                    break;
                }
            }
            if !num.is_empty() {
                let n: String = num.chars().rev().collect();
                ctx = format!("{n}%");
                break;
            }
        }
    }

    for line in pane_text.lines().rev() {
        if line.contains('▸') || line.contains('⏺') {
            let t = line.trim_start();
            last_tool = t.chars().take(90).collect();
            break;
        }
    }

    let mut dh = String::new();
    if !cwd.is_empty() && git.is_git_dir(&cwd) {
        let (d_hash, d_stat) = git.diff_and_status(&cwd);
        dh = d_hash;
        diffstat = d_stat;
    }

    let fp = format!("{body_hash}|{dh}");

    Some(Observation {
        status,
        ctx,
        last_tool,
        diffstat,
        pane_text,
        fp,
        watcher_pane,
    })
}

pub fn prompt_worker(
    name: &str,
    task: Option<&str>,
    resume: bool,
    extend: Option<u64>,
    git: &Git,
    herdr: &Herdr,
) -> Result<(), (i32, String)> {
    if let Ok(v) = std::env::var("PROMPT_WORKER_BUDGET_SEC") {
        if !v.chars().all(|c| c.is_ascii_digit()) {
            return Err((1, format!("error: PROMPT_WORKER_BUDGET_SEC must be whole seconds (got '{v}')\n")));
        }
    }

    let runs_dir = default_runs_dir();
    let _ = std::fs::create_dir_all(&runs_dir);

    let out = runs_dir.join(format!("{name}.md"));
    let submit_log = runs_dir.join(format!(".{name}.submit.log"));
    let elapsed_file = runs_dir.join(format!(".{name}.elapsed"));
    let sentinel = runs_dir.join(format!(".{name}.nudged"));
    let budget_file = runs_dir.join(format!(".{name}.budget"));
    let grace_file = runs_dir.join(format!(".{name}.grace"));
    let progress_file = runs_dir.join(format!(".{name}.progress"));
    let srt = runs_dir.join(format!("{name}.srt.json"));

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

    let prev_elapsed: u64;
    let mut budget: u64;

    if resume {
        prev_elapsed = std::fs::read_to_string(&elapsed_file)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let saved_budget = std::fs::read_to_string(&budget_file)
            .ok()
            .and_then(|s| s.trim().parse().ok());
        budget = saved_budget.unwrap_or_else(|| resolve_budget_sec("").0);
        if let Some(ext) = extend {
            budget += ext;
            let _ = std::fs::write(&budget_file, format!("{budget}\n"));
            eprintln!("WATCHDOG extend +{ext}s (allowance {budget}s, cap {max}s)");
        }
    } else {
        let task_str = task.unwrap_or("");
        prev_elapsed = 0;
        budget = resolve_budget_sec(task_str).0;
        let _ = std::fs::write(&budget_file, format!("{budget}\n"));
        let _ = std::fs::remove_file(&elapsed_file);
        let _ = std::fs::remove_file(&sentinel);
        let _ = std::fs::remove_file(&grace_file);
        let _ = std::fs::remove_file(&progress_file);

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
            let grid_tab = grid_tab.trim();
            let mut grid_script: Option<PathBuf> = None;
            if let Ok(dir) = std::env::var("QUAQUA_SCRIPT_DIR") {
                let c = PathBuf::from(dir).join("grid-workers.sh");
                if c.is_file() {
                    grid_script = Some(c);
                }
            }
            if grid_script.is_none() {
                if let Ok(exe) = std::env::current_exe() {
                    if let Some(dir) = exe.parent() {
                        let c = dir.join("grid-workers.sh");
                        if c.is_file() {
                            grid_script = Some(c);
                        }
                    }
                }
            }
            if grid_script.is_none() {
                let c = PathBuf::from("scripts/worker/grid-workers.sh");
                if c.is_file() {
                    grid_script = Some(c);
                }
            }
            if let Some(script) = grid_script {
                let mut cmd = Command::new("bash");
                cmd.arg(script).arg("--check");
                if !grid_tab.is_empty() {
                    cmd.arg("--tab").arg(grid_tab);
                }
                cmd.arg("--only").arg(name);
                if let Ok(st) = cmd.status() {
                    if !st.success() {
                        return Err((1, format!("error: grid check failed for {name}; arrange panes manually before prompting\n")));
                    }
                }
            }
        }

        // Background prompt dispatch
        let prompt_body = format!(
            "{task_str}. When done, write summary (changes, test cmd+result, risks) to a temp file and rename it to ~/quaqua/runs/{name}.md (publish it atomically, never a partial file); reply only the path."
        );
        let submit_file = std::fs::File::create(&submit_log).ok();
        if let Some(file) = submit_file {
            let _ = Command::new("herdr")
                .arg("agent")
                .arg("prompt")
                .arg(name)
                .arg(&prompt_body)
                .arg("--wait")
                .arg("--timeout")
                .arg("1800000")
                .stdout(file.try_clone().unwrap())
                .stderr(file)
                .spawn();
        }
    }

    let start = now_sec();
    let rem_max = (max.saturating_sub(prev_elapsed)).max(1);
    let rem_budget = (budget.saturating_sub(prev_elapsed)).max(1);
    let deadline = start + rem_max;
    let mut budget_deadline = start + rem_budget;
    let nudge_at = start + (budget * nudge_pct / 100).saturating_sub(prev_elapsed);

    let save_elapsed = |now: u64| {
        let el = prev_elapsed + now.saturating_sub(start);
        let _ = std::fs::write(&elapsed_file, format!("{el}\n"));
    };

    let watchdog_exit = |code: i32, reason: &str, obs: &Option<Observation>, change_ts: u64| -> (i32, String) {
        let now = now_sec();
        save_elapsed(now);
        let status = obs.as_ref().map(|o| o.status.as_str()).unwrap_or("unknown");
        let ctx = obs.as_ref().map(|o| o.ctx.as_str()).unwrap_or("?");
        let quiet = now.saturating_sub(change_ts);
        let last_tool = obs.as_ref().map(|o| o.last_tool.as_str()).filter(|s| !s.is_empty()).unwrap_or("none");
        let diffstat = obs.as_ref().map(|o| o.diffstat.as_str()).filter(|s| !s.is_empty()).unwrap_or("n/a");
        let elapsed = now.saturating_sub(start);
        eprintln!(
            "WATCHDOG exit={code} {reason} | name={name} elapsed={elapsed}s status={status} ctx={ctx} quiet={quiet}s last={last_tool} | diff={diffstat}"
        );
        (code, String::new())
    };

    let malformed_report = |missing: &str| -> (i32, String) {
        let now = now_sec();
        save_elapsed(now);
        eprintln!("WATCHDOG malformed-report | name={name} missing={missing} report={}", out.display());
        (6, String::new())
    };

    let mut prev_fp = String::new();
    let mut fp_seen = false;
    let mut change_ts = start;
    let mut idle_since = 0;
    let mut last_hb = start;
    let mut nudged = sentinel.exists();
    let mut grace_used = grace_file.exists();
    let mut gone_since = 0;
    let mut blind_warned = false;
    let mut last_progress: u64 = std::fs::read_to_string(&progress_file)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    eprintln!(
        "waiting for {} (poll {}s, stall {}s, idle {}s, budget {}s, cap {}s)",
        out.display(),
        poll,
        stall,
        idle_sec,
        budget,
        max
    );

    let mut last_obs: Option<Observation> = None;

    while !report_ready(&out) {
        let now = now_sec();
        if now >= deadline {
            return Err(watchdog_exit(2, &format!("error: no {} after {}s", out.display(), max), &last_obs, change_ts));
        }

        if let Some(obs) = observe(herdr, git, name) {
            let st = obs.status.clone();
            let is_waiting = lead_waiting(&runs_dir, name, &obs.watcher_pane);

            if st == "gone" || (st == "unknown" && obs.pane_text.is_empty()) {
                if gone_since == 0 {
                    gone_since = now;
                }
                if now - gone_since >= gone_sec {
                    return Err(watchdog_exit(3, &format!("agent gone: status={st} for {}s", now - gone_since), &Some(obs), change_ts));
                }
            } else {
                gone_since = 0;
            }

            if st == "blocked" {
                return Err(watchdog_exit(5, "agent blocked (question/permission pending)", &Some(obs), change_ts));
            }
            if question_seen(&obs.pane_text) {
                return Err(watchdog_exit(5, "pane shows a pending question/permission prompt", &Some(obs), change_ts));
            }

            if obs.fp != prev_fp {
                prev_fp = obs.fp.clone();
                if fp_seen {
                    change_ts = now;
                    last_progress = now;
                    idle_since = 0;
                    let _ = std::fs::write(&progress_file, format!("{now}\n"));
                }
                fp_seen = true;
            }

            if now >= budget_deadline {
                if !grace_used && last_progress > 0 && now - last_progress <= poll * 2 {
                    let mut grace = (budget / 4).max(1);
                    if budget_deadline + grace > deadline {
                        grace = deadline.saturating_sub(budget_deadline);
                    }
                    if grace > 0 {
                        budget_deadline += grace;
                        grace_used = true;
                        let _ = std::fs::write(&grace_file, "");
                        eprintln!("WATCHDOG grace +{grace}s (progressing)");
                    }
                }
                if now >= budget_deadline {
                    return Err(watchdog_exit(4, &format!("time budget {budget}s exceeded"), &Some(obs), change_ts));
                }
            }

            if is_waiting {
                idle_since = 0;
            } else if st == "idle" || st == "done" {
                if idle_since == 0 {
                    idle_since = now;
                }
                if now - idle_since >= idle_sec {
                    return Err(watchdog_exit(3, &format!("agent {st} for {}s without run file", now - idle_since), &Some(obs), change_ts));
                }
            } else {
                idle_since = 0;
            }

            if !is_waiting && now - change_ts >= stall {
                return Err(watchdog_exit(3, &format!("stall: no pane/worktree change for {}s", now - change_ts), &Some(obs), change_ts));
            }

            last_obs = Some(obs);
        } else {
            if gone_since == 0 {
                gone_since = now;
            }
            if now - gone_since >= gone_sec {
                return Err(watchdog_exit(3, &format!("agent gone: unobservable (no get/read) for {}s", now - gone_since), &last_obs, change_ts));
            }
            if !blind_warned {
                eprintln!("WATCHDOG observe failed (no agent get/read); only cap/budget/agent-gone apply");
                blind_warned = true;
            }
            if now >= budget_deadline {
                return Err(watchdog_exit(4, &format!("time budget {budget}s exceeded (agent unobservable)"), &last_obs, change_ts));
            }
        }

        if !nudged && now >= nudge_at {
            let nudge_msg = format!(
                "Time check: you are past {nudge_pct}% of this task's time budget. Wrap up now: finish the current step, commit your work, and write ~/quaqua/runs/{name}.md (via a temp file + rename) with what is done plus a Blockers section for the rest. Reply only the path."
            );
            let _ = herdr.agent_prompt_nowait(name, &nudge_msg);
            let _ = std::fs::write(&sentinel, "");
            nudged = true;
            eprintln!("WATCHDOG nudge sent (once, at {nudge_pct}% of {budget}s budget)");
        }

        if now - last_hb >= heartbeat {
            let status = last_obs.as_ref().map(|o| o.status.as_str()).unwrap_or("unknown");
            let ctx = last_obs.as_ref().map(|o| o.ctx.as_str()).unwrap_or("?");
            let quiet = now.saturating_sub(change_ts);
            let last_tool = last_obs.as_ref().map(|o| o.last_tool.as_str()).filter(|s| !s.is_empty()).unwrap_or("none");
            let elapsed = now.saturating_sub(start);
            eprintln!("WATCHDOG heartbeat | name={name} elapsed={elapsed}s status={status} ctx={ctx} quiet={quiet}s last={last_tool}");
            last_hb = now;
        }

        let left = (deadline.saturating_sub(now)).max(1);
        nap(left.min(poll));
    }

    // Validate report shape
    let mut expect_verdict = false;
    if srt.is_file() {
        if let Ok(c) = std::fs::read_to_string(&srt) {
            if let Ok(j) = parse_json(&c) {
                if j.get_bool("review") == Some(true) {
                    expect_verdict = true;
                }
            }
        }
    }

    let report_content = std::fs::read_to_string(&out).unwrap_or_default();
    if expect_verdict {
        let has_verdict = report_content.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("Verdict:") || t.starts_with("verdict:")
        });
        if !has_verdict {
            return Err(malformed_report("'Verdict:' line"));
        }
    } else {
        let has_lessons = report_content.lines().any(|l| l.trim_start().starts_with("## Lessons"));
        if !has_lessons {
            return Err(malformed_report("'## Lessons' section"));
        }
    }

    let now = now_sec();
    save_elapsed(now);
    println!("{}", out.display());
    Ok(())
}
