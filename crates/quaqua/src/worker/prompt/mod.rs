//! Worker prompt & watchdog logic (`quaqua worker prompt <name> <task>`).

pub mod cost;
pub mod observer;
pub mod watchdog;
pub mod workflow;

pub use crate::util::now_sec;
pub use cost::{brief_cost_budget, read_worker_cost, resolve_cost_budget};
pub use observer::{is_provider_timeout, observe, Observation};
pub use watchdog::{
    fallback_header_line, is_lessons_heading, nap, prepend_header, run_watchdog,
    validate_report_content, WatchdogParams,
};

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use crate::engine::activities::SystemActivities;
use crate::engine::activity::{
    Activity, ActivityError, ActivityRequest, ActivityResult, HerdrRequest,
};
use crate::engine::child::SingleWorkerInput;
use crate::engine::journal::Journal;
use crate::engine::journal_sqlite::SqliteJournal;
use crate::engine::runner::RunnerOutcome;
use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::state::WorkerMetadata;
use crate::worker::cleanup::default_runs_dir;
use crate::worker::prompt::cost::CostTracker;
use crate::worker::prompt::observer::lead_waiting;
use crate::worker::prompt::workflow::{SingleWorkerState, SingleWorkerWorkflow, WatchdogConfig};
pub const USAGE: &str = "\
# Usage: quaqua worker prompt <name> <task>        submit task brief, wait for runs file
#        quaqua worker prompt --resume <name> [--extend <sec>]
#                                             keep waiting; --extend adds <sec> to
#                                             this name's allowance (persisted in
#                                             journal; the cap still wins)
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
    Resume { name: String, extend: Option<u64> },
    Prompt { name: String, task: String },
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
                    (
                        1,
                        "error: --extend <sec> requires a positive integer\n".to_string(),
                    )
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

#[derive(Clone, Default)]
struct ObsSnapshot {
    status: String,
    ctx: String,
    last_tool: String,
    diffstat: String,
    pane_text: String,
}

impl ObsSnapshot {
    fn from_obs(obs: &Observation) -> Self {
        Self {
            status: obs.status.clone(),
            ctx: obs.ctx.clone(),
            last_tool: obs.last_tool.clone(),
            diffstat: obs.diffstat.clone(),
            pane_text: obs.pane_text.clone(),
        }
    }
}
#[derive(Debug, Clone, Default)]
struct PromptClock {
    timers: BinaryHeap<Reverse<(u64, u64, String)>>,
    seq: u64,
}

impl PromptClock {
    fn new() -> Self {
        Self {
            timers: BinaryHeap::new(),
            seq: 0,
        }
    }
}

impl crate::engine::runner::RunnerClock for PromptClock {
    fn now_ms(&self) -> u64 {
        if std::env::var_os("PROMPT_WORKER_CLOCK_FILE").is_some() {
            crate::util::now_sec() * 1000
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        }
    }

    fn schedule_timer(&mut self, id: &str, duration: Duration) {
        let deadline = self.now_ms() + duration.as_millis() as u64;
        self.seq = self.seq.wrapping_add(1);
        self.timers
            .push(Reverse((deadline, self.seq, id.to_string())));
    }

    fn advance_to_next_timer(&mut self) -> Option<(u64, Vec<String>)> {
        if self.timers.is_empty() {
            return None;
        }

        loop {
            let now = self.now_ms();
            if let Some(Reverse((earliest, _, _))) = self.timers.peek() {
                if *earliest > now {
                    let delta_ms = *earliest - now;
                    let delta_sec = (delta_ms / 1000).max(1);
                    nap(delta_sec);
                }
            } else {
                return None;
            }

            let now = self.now_ms();
            let mut fired = Vec::new();
            while let Some(Reverse((deadline, _, _))) = self.timers.peek() {
                if *deadline <= now {
                    if let Some(Reverse((_, _, id))) = self.timers.pop() {
                        fired.push(id);
                    }
                } else {
                    break;
                }
            }
            if !fired.is_empty() {
                return Some((now, fired));
            }
            if self.timers.is_empty() {
                return None;
            }
        }
    }

    fn pending_timers_count(&self) -> usize {
        self.timers.len()
    }
}

struct PromptActivities {
    inner: SystemActivities,
    runs_dir: PathBuf,
    time_budget_sec: u64,
    nudge_pct: u64,
    heartbeat_sec: u64,
    land_sec: u64,
    is_resume: bool,
    start_sec: u64,
    cost_tracker: Mutex<CostTracker>,
    last_obs: Mutex<Option<ObsSnapshot>>,
    last_change: Mutex<u64>,
    last_heartbeat: Mutex<u64>,
    prev_fp: Mutex<String>,
    fp_seen: Mutex<bool>,
    prompt_landed: Mutex<bool>,
    prompt_recovered: Mutex<bool>,
    recover_at: Mutex<u64>,
    recover_deadline: Mutex<u64>,
    prompt_not_landed: Mutex<bool>,
    provider_retried: Mutex<bool>,
}

impl Activity for PromptActivities {
    fn execute(&self, req: &ActivityRequest) -> Result<ActivityResult, ActivityError> {
        if let ActivityRequest::HerdrEvents(ev_req) = req {
            if let Ok(r) = self.inner.execute(req) {
                return Ok(r);
            }
            match ev_req {
                crate::engine::activity::HerdrEventRequest::Subscribe { agent, .. } => {
                    return Ok(ActivityResult::HerdrEvents(
                        crate::engine::activity::HerdrEventResult::Subscribed {
                            agent: agent.clone(),
                        },
                    ));
                }
                crate::engine::activity::HerdrEventRequest::Unsubscribe { agent } => {
                    return Ok(ActivityResult::HerdrEvents(
                        crate::engine::activity::HerdrEventResult::Unsubscribed {
                            agent: agent.clone(),
                        },
                    ));
                }
                crate::engine::activity::HerdrEventRequest::PollEventsFor { agent } => {
                    return Ok(ActivityResult::HerdrEvents(
                        crate::engine::activity::HerdrEventResult::ScopedEvents {
                            agent: agent.clone(),
                            events: Vec::new(),
                        },
                    ));
                }
                crate::engine::activity::HerdrEventRequest::PollEvents => {
                    return Ok(ActivityResult::HerdrEvents(
                        crate::engine::activity::HerdrEventResult::Events(Vec::new()),
                    ));
                }
            }
        }
        if let ActivityRequest::Git(crate::engine::activity::GitRequest::DiffHash { path }) = req {
            let target = std::path::Path::new(path);
            if !target.exists() || !target.is_dir() || !self.inner.git.is_git_dir(target) {
                return Ok(ActivityResult::Git(
                    crate::engine::activity::GitResult::DiffHash {
                        hash: String::new(),
                        stat: String::new(),
                    },
                ));
            }
        }
        if let ActivityRequest::ReportFile(
            crate::engine::activity::ReportFileRequest::CheckMtime { path },
        ) = req
        {
            let p = std::path::Path::new(path);
            if !p.exists() {
                return Ok(ActivityResult::ReportFile(
                    crate::engine::activity::ReportFileResult::MtimeMs(0),
                ));
            }
            if crate::worker::prompt::observer::report_ready(p) {
                return Ok(ActivityResult::ReportFile(
                    crate::engine::activity::ReportFileResult::MtimeMs(1),
                ));
            } else {
                return Ok(ActivityResult::ReportFile(
                    crate::engine::activity::ReportFileResult::MtimeMs(0),
                ));
            }
        }
        if let ActivityRequest::ReportFile(crate::engine::activity::ReportFileRequest::Read {
            path,
        }) = req
        {
            if !std::path::Path::new(path).exists() {
                return Ok(ActivityResult::ReportFile(
                    crate::engine::activity::ReportFileResult::Content(String::new()),
                ));
            }
        }
        if let ActivityRequest::Herdr(HerdrRequest::ReadAgent { .. }) = req {
            match self.inner.execute(req) {
                Ok(r) => return Ok(r),
                Err(e) => {
                    let err_msg = e.to_string();
                    if err_msg.contains("agent_not_found") {
                        return Ok(ActivityResult::Herdr(
                            crate::engine::activity::HerdrResult::AgentOutput {
                                content: String::new(),
                            },
                        ));
                    }
                    return Err(e);
                }
            }
        }
        let res = self.inner.execute(req);
        match req {
            ActivityRequest::Herdr(HerdrRequest::AgentInfo { name }) => {
                let obs = observe(&self.inner.herdr, &self.inner.git, name);
                *self.last_obs.lock().unwrap() = obs.as_ref().map(ObsSnapshot::from_obs);

                if let Some(o) = &obs {
                    self.cost_tracker
                        .lock()
                        .unwrap()
                        .check(&self.runs_dir, name, o);

                    let now = now_sec();
                    let mut lh = self.last_heartbeat.lock().unwrap();
                    if now.saturating_sub(*lh) >= self.heartbeat_sec {
                        *lh = now;
                        let elapsed = now.saturating_sub(self.start_sec);
                        let quiet = now.saturating_sub(*self.last_change.lock().unwrap());
                        let last_tool = if o.last_tool.is_empty() {
                            "none"
                        } else {
                            &o.last_tool
                        };
                        eprintln!(
                            "WATCHDOG heartbeat | name={name} elapsed={elapsed}s status={} ctx={} quiet={quiet}s last={last_tool}",
                            o.status, o.ctx
                        );
                    }

                    if (o.status == "idle" || o.status == "done")
                        && !*self.provider_retried.lock().unwrap()
                        && is_provider_timeout(&o.pane_text)
                    {
                        *self.provider_retried.lock().unwrap() = true;
                        let _ = WorkerMetadata::record_provider_retry(&self.runs_dir, name, now);
                        let _ = self.inner.herdr.agent_prompt_nowait(name, "continue");
                        eprintln!("WATCHDOG provider timeout detected | name={name} — re-prompted session with 'continue'");
                    }

                    if !self.is_resume && self.land_sec > 0 {
                        let mut pl = self.prompt_landed.lock().unwrap();
                        let has_report = self.runs_dir.join(format!("{name}.md")).is_file();
                        if has_report {
                            *pl = true;
                            *self.prompt_recovered.lock().unwrap() = false;
                            *self.prompt_not_landed.lock().unwrap() = false;
                        } else if !*pl {
                            let is_waiting = lead_waiting(&self.runs_dir, name, &o.watcher_pane);
                            if o.status == "working" || o.status == "blocked" || is_waiting {
                                *pl = true;
                            } else if o.status == "idle" || o.status == "done" {
                                let mut pr = self.prompt_recovered.lock().unwrap();
                                if *pr {
                                    if now >= *self.recover_deadline.lock().unwrap() {
                                        *self.prompt_not_landed.lock().unwrap() = true;
                                    }
                                } else if now >= self.start_sec + self.land_sec {
                                    *pr = true;
                                    *self.recover_at.lock().unwrap() = now;
                                    *self.recover_deadline.lock().unwrap() = now + self.land_sec;
                                    let _ = self.inner.herdr.agent_send_keys(name, "enter");
                                    eprintln!(
                                        "WATCHDOG prompt-not-landed | name={name} {} {}s after prompt — sent enter once (input may be typed but unsubmitted)",
                                        o.status,
                                        now.saturating_sub(self.start_sec)
                                    );
                                }
                            }
                        }
                    }

                    let mut prev_fp = self.prev_fp.lock().unwrap();
                    if *prev_fp != o.fp {
                        *prev_fp = o.fp.clone();
                        if *self.fp_seen.lock().unwrap() {
                            *self.last_change.lock().unwrap() = now;
                        }
                        *self.fp_seen.lock().unwrap() = true;
                    }
                }
            }
            ActivityRequest::Herdr(HerdrRequest::Prompt { task, .. }) => {
                if task.starts_with("Time check: you are past") {
                    eprintln!(
                        "WATCHDOG nudge sent (once, at {}% of {}s budget)",
                        self.nudge_pct, self.time_budget_sec
                    );
                }
            }
            _ => {}
        }

        res
    }
}

pub fn prompt_worker<G: GitOps, H: HerdrOps>(
    name: &str,
    task: Option<&str>,
    resume: bool,
    extend: Option<u64>,
    _git: &G,
    herdr: &H,
) -> Result<(), (i32, String)> {
    validate_env_budget()?;

    let runs_dir = default_runs_dir();
    let _ = std::fs::create_dir_all(&runs_dir);

    let out = runs_dir.join(format!("{name}.md"));
    let srt = WorkerMetadata::srt_path(&runs_dir, name);
    let submit_log = WorkerMetadata::submit_log_path(&runs_dir, name);
    let cost_budget: Option<f64> = resolve_cost_budget(task.unwrap_or(""));

    let max: u64 = std::env::var("PROMPT_WORKER_MAX_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7200);

    let poll_sec: u64 = std::env::var("PROMPT_WORKER_POLL_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30)
        .max(1);
    let stall_sec: u64 = std::env::var("PROMPT_WORKER_STALL_SEC")
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
    let heartbeat_sec: u64 = std::env::var("PROMPT_WORKER_HEARTBEAT_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let land_sec: u64 = std::env::var("PROMPT_WORKER_LAND_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);

    let mut expect_verdict = false;
    let mut fallback_line: Option<String> = None;
    if srt.is_file() {
        if let Ok(c) = std::fs::read_to_string(&srt) {
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(&c) {
                if j.get("review").and_then(|b| b.as_bool()) == Some(true) {
                    expect_verdict = true;
                }
            }
            fallback_line = fallback_header_line(&c);
        }
    }

    let config = WatchdogConfig {
        poll_sec,
        stall_sec,
        idle_sec,
        idle_fallback_sec,
        gone_sec,
        nudge_pct,
        hard_cap_sec: max,
        is_fallback_idle: false,
        expect_verdict,
        land_sec,
    };

    let db_path = runs_dir.join("quaqua.db");
    let mut journal = SqliteJournal::open(&db_path).map_err(|e| {
        (
            1,
            format!(
                "error: failed to open journal at {}: {e}\n",
                db_path.display()
            ),
        )
    })?;

    // Probe existing sessions worker:<name>#<k>
    let mut k_max = 0u64;
    let mut last_snap: Option<SingleWorkerState> = None;
    let mut last_session_is_terminal = true;

    for k in 1.. {
        let wf_id = format!("worker:{name}#{k}");
        let events = journal.load_events(&wf_id, 1).unwrap_or_default();
        let snap = journal.load_latest_snapshot(&wf_id).ok().flatten();
        if events.is_empty() && snap.is_none() {
            break;
        }
        k_max = k;
        let is_terminal = events
            .iter()
            .any(|e| e.kind == "workflow_completed" || e.kind == "workflow_failed");
        last_session_is_terminal = is_terminal;
        if let Some(s) = snap {
            if let Ok(state) = serde_json::from_str::<SingleWorkerState>(&s.state_json) {
                last_snap = Some(state);
            }
        }
    }

    if k_max == 0 {
        let base_id = format!("worker:{name}");
        if let Ok(Some(s)) = journal.load_latest_snapshot(&base_id) {
            if let Ok(state) = serde_json::from_str::<SingleWorkerState>(&s.state_json) {
                last_snap = Some(state);
            }
        }
    }

    let fallback_kind = if srt.is_file() {
        if let Ok(c) = std::fs::read_to_string(&srt) {
            serde_json::from_str::<serde_json::Value>(&c)
                .ok()
                .and_then(|j| {
                    j.get("kind")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| name.split('-').next().unwrap_or("tieude").to_string())
        } else {
            name.split('-').next().unwrap_or("tieude").to_string()
        }
    } else {
        name.split('-').next().unwrap_or("tieude").to_string()
    };

    let fallback_repo = {
        let (_ok, get_info) = herdr.agent_get(name).unwrap_or((false, String::new()));
        let mut cwd = String::new();
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&get_info) {
            if let Some(agent) = json.get("result").and_then(|r| r.get("agent")) {
                cwd = agent
                    .get("cwd")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
            }
        }
        if !cwd.is_empty() {
            cwd
        } else {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        }
    };

    enum ExecAction {
        Run {
            wf_id: String,
            state: SingleWorkerState,
            budget: u64,
        },
        Resume {
            wf_id: String,
            budget: u64,
        },
    }

    let exec_action: ExecAction;

    if !resume {
        let task_str = task.unwrap_or("");
        let budget = resolve_budget_sec(task_str).0;

        if out.exists() {
            let stale_path = format!("{}.stale.{}", out.display(), now_sec());
            let _ = std::fs::rename(&out, &stale_path);
            eprintln!(
                "archived pre-existing {} — a new task needs a fresh report",
                out.display()
            );
        }
        let _ = std::fs::remove_file(WorkerMetadata::provider_retry_path(&runs_dir, name));

        // Pre-prompt grid check
        if std::env::var("GRID_SKIP").as_deref() != Ok("1")
            || (std::env::var("PIPELINE_TASK").is_err()
                && std::env::var("QUAQUA_TEST").as_deref() != Ok("1"))
        {
            let grid_tab =
                std::fs::read_to_string(runs_dir.join(format!("{name}.grid"))).unwrap_or_default();
            let tab = {
                let t = grid_tab.trim();
                if t.is_empty() {
                    "workers"
                } else {
                    t
                }
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

        let next_k = k_max + 1;
        let wf_id = format!("worker:{name}#{next_k}");
        let repo = fallback_repo.clone();
        let kind = fallback_kind.clone();
        let input = SingleWorkerInput::new(name, kind, repo, task_str, budget, cost_budget);
        let state = SingleWorkerState::new(input)
            .with_report_path(&out)
            .with_config(config.clone());
        exec_action = ExecAction::Run {
            wf_id,
            state,
            budget,
        };
    } else if let Some(ext) = extend {
        let next_k = k_max + 1;
        let wf_id = format!("worker:{name}#{next_k}");
        let (
            repo,
            kind,
            brief,
            old_budget,
            prev_elapsed,
            overrun_granted,
            extension_total,
            nudged,
            last_progress,
        ) = if let Some(s) = &last_snap {
            (
                s.input.repo.clone(),
                s.input.kind.clone(),
                s.input.brief.clone(),
                s.input.time_budget_sec,
                s.prev_elapsed_ms / 1000,
                s.overrun_granted,
                s.extension_total_ms / 1000,
                s.nudged,
                s.last_progress_ms / 1000,
            )
        } else {
            (
                fallback_repo.clone(),
                fallback_kind.clone(),
                String::new(),
                resolve_budget_sec("").0,
                0,
                false,
                0,
                false,
                0,
            )
        };
        let new_budget = old_budget + ext;
        eprintln!("WATCHDOG extend +{ext}s (allowance {new_budget}s, cap {max}s)");
        let input = SingleWorkerInput::new(name, kind, repo, brief, new_budget, cost_budget);
        let state = SingleWorkerState::new(input)
            .with_report_path(&out)
            .with_config(config.clone())
            .resumed(
                prev_elapsed,
                overrun_granted,
                extension_total,
                nudged,
                last_progress,
            );
        exec_action = ExecAction::Run {
            wf_id,
            state,
            budget: new_budget,
        };
    } else {
        if k_max > 0 && !last_session_is_terminal {
            let wf_id = format!("worker:{name}#{k_max}");
            let budget = last_snap
                .as_ref()
                .map(|s| s.input.time_budget_sec)
                .unwrap_or_else(|| resolve_budget_sec("").0);
            exec_action = ExecAction::Resume { wf_id, budget };
        } else {
            let next_k = k_max + 1;
            let wf_id = format!("worker:{name}#{next_k}");
            let (
                repo,
                kind,
                brief,
                budget,
                prev_elapsed,
                overrun_granted,
                extension_total,
                nudged,
                last_progress,
            ) = if let Some(s) = &last_snap {
                (
                    s.input.repo.clone(),
                    s.input.kind.clone(),
                    s.input.brief.clone(),
                    s.input.time_budget_sec,
                    s.prev_elapsed_ms / 1000,
                    s.overrun_granted,
                    s.extension_total_ms / 1000,
                    s.nudged,
                    s.last_progress_ms / 1000,
                )
            } else {
                (
                    fallback_repo.clone(),
                    fallback_kind.clone(),
                    String::new(),
                    resolve_budget_sec("").0,
                    0,
                    false,
                    0,
                    false,
                    0,
                )
            };
            let input = SingleWorkerInput::new(name, kind, repo, brief, budget, cost_budget);
            let state = SingleWorkerState::new(input)
                .with_report_path(&out)
                .with_config(config.clone())
                .resumed(
                    prev_elapsed,
                    overrun_granted,
                    extension_total,
                    nudged,
                    last_progress,
                );
            exec_action = ExecAction::Run {
                wf_id,
                state,
                budget,
            };
        }
    }

    let (active_wf_id, active_budget, repo_dir) = match &exec_action {
        ExecAction::Run {
            wf_id,
            state,
            budget,
        } => (wf_id.clone(), *budget, PathBuf::from(&state.input.repo)),
        ExecAction::Resume { wf_id, budget } => {
            (wf_id.clone(), *budget, PathBuf::from(&fallback_repo))
        }
    };

    eprintln!(
        "waiting for {} (poll {}s, stall {}s, idle {}s, budget {}s, cap {}s)",
        out.display(),
        config.poll_sec,
        config.stall_sec,
        config.idle_sec,
        active_budget,
        max
    );

    let start_sec = now_sec();
    let sys_acts = SystemActivities::with_repo_dir(repo_dir);
    let prompt_acts = PromptActivities {
        inner: sys_acts,
        runs_dir: runs_dir.clone(),
        time_budget_sec: active_budget,
        nudge_pct,
        heartbeat_sec,
        land_sec,
        is_resume: resume,
        start_sec,
        cost_tracker: Mutex::new(CostTracker::new(&runs_dir, name, cost_budget)),
        last_obs: Mutex::new(None),
        last_change: Mutex::new(start_sec),
        last_heartbeat: Mutex::new(start_sec),
        prev_fp: Mutex::new(String::new()),
        fp_seen: Mutex::new(false),
        prompt_landed: Mutex::new(resume || land_sec == 0),
        prompt_recovered: Mutex::new(false),
        recover_at: Mutex::new(0),
        recover_deadline: Mutex::new(0),
        prompt_not_landed: Mutex::new(false),
        provider_retried: Mutex::new(if resume {
            WorkerMetadata::has_provider_retry(&runs_dir, name)
        } else {
            false
        }),
    };
    let mut acts = prompt_acts;
    let mut clock = PromptClock::new();
    let workflow = SingleWorkerWorkflow::new();

    let outcome = match exec_action {
        ExecAction::Run { wf_id, state, .. } => crate::engine::runner::run(
            &workflow,
            &wf_id,
            state,
            &mut journal,
            &mut acts,
            &mut clock,
        ),
        ExecAction::Resume { wf_id, .. } => crate::engine::runner::resume(
            &workflow,
            &wf_id,
            &mut journal,
            &mut acts,
            &mut clock,
            Vec::new(),
        ),
    };

    let single_output = match outcome {
        Ok(RunnerOutcome::Completed(o)) => o,
        Ok(RunnerOutcome::Blocked(reason)) => {
            return Err((1, format!("workflow blocked: {reason}\n")))
        }
        Ok(RunnerOutcome::Failed { code, reason }) => {
            return Err((code, format!("workflow failed: {reason}\n")))
        }
        Err(e) => return Err((1, format!("runner error: {e}\n"))),
    };

    // Check if grace or autoext was granted during this execution
    if let Ok(Some(snap)) = journal.load_latest_snapshot(&active_wf_id) {
        if let Ok(state) = serde_json::from_str::<SingleWorkerState>(&snap.state_json) {
            let prior_grace = last_snap
                .as_ref()
                .map(|s| s.overrun_granted)
                .unwrap_or(false);
            if state.overrun_granted && !prior_grace {
                let grace_sec = (active_budget / 4).max(1);
                eprintln!("WATCHDOG grace +{grace_sec}s (progressing)");
            }
            let prior_autoext = last_snap
                .as_ref()
                .map(|s| s.extension_total_ms / 1000)
                .unwrap_or(0);
            let cur_autoext = state.extension_total_ms / 1000;
            if cur_autoext > prior_autoext {
                let diff = cur_autoext - prior_autoext;
                let max_autoext = active_budget / 2;
                eprintln!("WATCHDOG auto-extend +{diff}s (diff progressing, cap {max}s, total +{cur_autoext}s/+{max_autoext}s)");
            }
        }
    }

    let final_obs = acts.last_obs.lock().unwrap().clone().or_else(|| {
        observe(&acts.inner.herdr, &acts.inner.git, name)
            .as_ref()
            .map(ObsSnapshot::from_obs)
    });
    let change_ts = *acts.last_change.lock().unwrap();

    let watchdog_exit = |code: i32, reason: &str| -> (i32, String) {
        let now = now_sec();
        let status = final_obs
            .as_ref()
            .map(|o| o.status.as_str())
            .unwrap_or("unknown");
        let ctx = final_obs.as_ref().map(|o| o.ctx.as_str()).unwrap_or("?");
        let quiet = now.saturating_sub(change_ts);
        let last_tool = final_obs
            .as_ref()
            .map(|o| o.last_tool.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("none");
        let diffstat = final_obs
            .as_ref()
            .map(|o| o.diffstat.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("n/a");
        let elapsed = now.saturating_sub(start_sec);
        eprintln!(
            "WATCHDOG exit={code} {reason} | name={name} elapsed={elapsed}s status={status} ctx={ctx} quiet={quiet}s last={last_tool} | diff={diffstat}"
        );
        (code, String::new())
    };

    match single_output.exit {
        0 => {
            let report_content = std::fs::read_to_string(&out).unwrap_or_default();
            if let Err(missing) = validate_report_content(&report_content, expect_verdict) {
                eprintln!(
                    "WATCHDOG malformed-report | name={name} missing={missing} report={}",
                    out.display()
                );
                return Err((6, String::new()));
            }

            if let Some(line) = fallback_line {
                if let Some(stamped) = prepend_header(&report_content, &line) {
                    let tmp = runs_dir.join(format!(".{name}.tmp"));
                    if std::fs::write(&tmp, stamped).is_ok() {
                        let _ = std::fs::rename(&tmp, &out);
                    }
                }
            }

            let read_marker = runs_dir.join(format!(".{name}.read"));
            let _ = std::fs::write(&read_marker, format!("{}\n", now_sec()));

            println!("{}", out.display());
            Ok(())
        }
        2 => {
            let reason = format!("error: no {} after {}s", out.display(), max);
            Err(watchdog_exit(2, &reason))
        }
        3 => {
            let now = now_sec();
            let has_report = out.is_file();
            let is_pnl = !has_report
                && (*acts.prompt_not_landed.lock().unwrap()
                    || *acts.prompt_recovered.lock().unwrap());
            let st = final_obs
                .as_ref()
                .map(|o| o.status.as_str())
                .unwrap_or("unknown");
            let quiet = now.saturating_sub(change_ts);
            let reason = if is_pnl {
                let rec_at = *acts.recover_at.lock().unwrap();
                format!(
                    "prompt not landed: agent {st} for {}s after a send-keys enter",
                    now.saturating_sub(rec_at)
                )
            } else if st == "gone"
                || (st == "unknown"
                    && final_obs
                        .as_ref()
                        .map(|o| o.pane_text.is_empty())
                        .unwrap_or(false))
            {
                format!("agent gone: status={st} for {quiet}s")
            } else if st == "idle" || st == "done" {
                format!("agent {st} for {quiet}s without run file")
            } else {
                format!("stall: no pane/worktree change for {quiet}s")
            };
            Err(watchdog_exit(3, &reason))
        }
        4 => {
            let reason = format!("time budget {}s exceeded", active_budget);
            Err(watchdog_exit(4, &reason))
        }
        5 => {
            let st = final_obs
                .as_ref()
                .map(|o| o.status.as_str())
                .unwrap_or("unknown");
            let reason = if st == "blocked" {
                "agent blocked (question/permission pending)"
            } else {
                "pane shows a pending question/permission prompt"
            };
            Err(watchdog_exit(5, reason))
        }
        6 => {
            let report_content = std::fs::read_to_string(&out).unwrap_or_default();
            let missing = validate_report_content(&report_content, expect_verdict)
                .err()
                .unwrap_or("unknown section");
            eprintln!(
                "WATCHDOG malformed-report | name={name} missing={missing} report={}",
                out.display()
            );
            Err((6, String::new()))
        }
        other => Err((other, format!("unexpected exit code {other}\n"))),
    }
}
