//! SingleWorkerWorkflow implementation over the extended W0 engine contract.
//!
//! Evaluates worker watchdog rules deterministically:
//! - Exit 0: Success (report ready with stable mtime, non-empty content, and valid `## Lessons` or `Verdict:`)
//! - Exit 2: Hard timeout / cap exceeded
//! - Exit 3: Stall (no pane or diff progress for 600 s) / idle (120 s, 300 s fallback) / agent gone (60 s)
//! - Exit 4: Time budget exceeded (after one +25% grace and bounded auto-extensions up to +50% total)
//! - Exit 5: Blocked on question / permission
//! - Exit 6: Malformed report (stable, non-empty report missing `## Lessons` or `Verdict:`)
//!
//! Signals and contracts:
//! - Auto-extension keys exclusively to `GitRequest::DiffHash { path }` -> `GitResult::DiffHash { hash, stat }`
//! - Progress fingerprint `fp` combines cleaned pane body hash (dropping spinners/separators and collapsing digit runs) + diff hash
//! - Grace keys to progress fingerprint `fp` advancement within 2 polls of budget deadline
//! - Agent status, harness, and hook activity arrive via `HerdrRequest::AgentInfo { name }` -> `HerdrResult::AgentInfo`
//! - Scoped events arrive via `HerdrEventRequest::PollEventsFor { agent }` -> `HerdrEventResult::ScopedEvents` (filtering `pane.exited`)
//! - Report readiness requires two consecutive matching `ReportFileRequest::CheckMtime` samples + non-empty content
//!
//! Determinism contract: No `std::fs`, `std::process`, or `std::net` is used.
//! All external observation and time progression occur through scheduled activities, timers,
//! and virtual time arithmetic.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;

use crate::engine::activity::{
    ActivityRequest, ActivityResult, GitRequest, GitResult, HerdrEventRequest, HerdrEventResult,
    HerdrRequest, HerdrResult, ReportFileRequest, ReportFileResult,
};
use crate::engine::child::{SingleWorkerInput, SingleWorkerOutput};
use crate::engine::workflow::{StepDecision, Workflow, WorkflowContext, WorkflowEvent};
use crate::worker::prompt::observer::question_seen;
use crate::worker::prompt::watchdog::validate_report_content;

/// Pure configuration parameters for watchdog execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchdogConfig {
    pub poll_sec: u64,
    pub stall_sec: u64,
    pub idle_sec: u64,
    pub idle_fallback_sec: u64,
    pub gone_sec: u64,
    pub nudge_pct: u64,
    pub hard_cap_sec: u64,
    pub is_fallback_idle: bool,
    pub expect_verdict: bool,
    #[serde(default)]
    pub land_sec: u64,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            poll_sec: 30,
            stall_sec: 600,
            idle_sec: 120,
            idle_fallback_sec: 300,
            gone_sec: 60,
            nudge_pct: 80,
            hard_cap_sec: 7200,
            is_fallback_idle: false,
            expect_verdict: false,
            land_sec: 0,
        }
    }
}

/// State of an active single worker workflow execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SingleWorkerState {
    pub input: SingleWorkerInput,
    pub report_path: PathBuf,
    pub config: WatchdogConfig,
    pub start_ms: u64,
    pub prev_elapsed_ms: u64,
    #[serde(default, alias = "budget_deadline_ms")]
    pub allowance_deadline_ms: u64,
    pub cap_deadline_ms: u64,
    pub last_progress_ms: u64,
    pub first_progress_seen: bool,
    pub last_progress_fp: String,
    pub current_diff_hash: String,
    pub last_autoext_diff_hash: String,
    pub latest_pane_text: String,
    pub agent_status: String,
    pub harness: Option<String>,
    pub hook_active: bool,
    pub idle_since_ms: u64,
    pub gone_since_ms: u64,
    #[serde(default, alias = "grace_used")]
    pub overrun_granted: bool,
    #[serde(default, alias = "autoext_total_ms")]
    pub extension_total_ms: u64,
    pub poll_count: u64,
    pub nudged: bool,
    pub initialized: bool,
    pub report_mtime_ms: u64,
    pub report_mtime_stable: bool,
    #[serde(default)]
    pub prompt_landed: bool,
    #[serde(default)]
    pub prompt_recovered: bool,
    #[serde(default)]
    pub recover_at_ms: u64,
    #[serde(default)]
    pub recover_deadline_ms: u64,
}

impl SingleWorkerState {
    pub fn new(input: SingleWorkerInput) -> Self {
        let report_path = PathBuf::from(format!("runs/{}.md", input.name));
        let expect_verdict = input.brief.contains("Pipeline phase: REVIEW");
        let is_fallback_idle = input.kind.contains("opencode") || input.kind.contains("omp");

        let mut config = WatchdogConfig::default();
        config.expect_verdict = expect_verdict;
        config.is_fallback_idle = is_fallback_idle;

        Self {
            input,
            report_path,
            config,
            start_ms: 0,
            prev_elapsed_ms: 0,
            allowance_deadline_ms: 0,
            cap_deadline_ms: 0,
            last_progress_ms: 0,
            first_progress_seen: false,
            last_progress_fp: String::new(),
            current_diff_hash: String::new(),
            last_autoext_diff_hash: String::new(),
            latest_pane_text: String::new(),
            agent_status: "working".to_string(),
            harness: None,
            hook_active: true,
            idle_since_ms: 0,
            gone_since_ms: 0,
            overrun_granted: false,
            extension_total_ms: 0,
            poll_count: 0,
            nudged: false,
            initialized: false,
            report_mtime_ms: 0,
            report_mtime_stable: false,
            prompt_landed: false,
            prompt_recovered: false,
            recover_at_ms: 0,
            recover_deadline_ms: 0,
        }
    }

    pub fn with_config(mut self, config: WatchdogConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_report_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.report_path = path.into();
        self
    }

    pub fn resumed(
        mut self,
        prev_elapsed_sec: u64,
        overrun_granted: bool,
        extension_total_sec: u64,
        nudged: bool,
        last_progress_sec: u64,
    ) -> Self {
        self.prev_elapsed_ms = prev_elapsed_sec * 1000;
        self.overrun_granted = overrun_granted;
        self.extension_total_ms = extension_total_sec * 1000;
        self.nudged = nudged;
        if last_progress_sec > 0 {
            self.last_progress_ms = last_progress_sec * 1000;
            self.first_progress_seen = true;
        }
        self.initialized = false;
        self.prompt_landed = true;
        self.initialized = false;
        self.start_ms = 0;
        self
    }
}

/// Deterministic SingleWorkerWorkflow state machine.
#[derive(Debug, Clone, Default)]
pub struct SingleWorkerWorkflow;

impl SingleWorkerWorkflow {
    pub fn new() -> Self {
        Self
    }
}
fn complete_step(
    state: &mut SingleWorkerState,
    now_ms: u64,
    exit: i32,
) -> StepDecision<SingleWorkerOutput> {
    if state.start_ms > 0 && now_ms >= state.start_ms {
        let added = now_ms - state.start_ms;
        state.prev_elapsed_ms = (state.prev_elapsed_ms + added).min(state.input.budget_sec * 1000);
        state.start_ms = now_ms;
    }
    StepDecision::Complete(
        SingleWorkerOutput::new(exit, state.report_path.clone())
            .expect("valid single worker exit code"),
    )
}

/// Cleans pane text body by dropping spinners/separators and collapsing digit sequences to 'N'.
fn clean_pane_body(pane_text: &str) -> String {
    let mut body_lines = Vec::new();
    for line in pane_text.lines() {
        let t = line.trim();
        let is_sep = t.chars().all(|c| "─│| ".contains(c));
        if is_sep
            || t.contains('👾')
            || t.contains('✦')
            || t.chars().any(|c| "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".contains(c))
        {
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
    body_lines.join("\n")
}

/// Computes the composite progress fingerprint: clean pane body hash + diff hash.
fn compute_fingerprint(pane_text: &str, diff_hash: &str) -> String {
    let cleaned = clean_pane_body(pane_text);
    let mut hasher = DefaultHasher::new();
    cleaned.hash(&mut hasher);
    let body_hash = format!("{:016x}", hasher.finish());
    format!("{body_hash}|{diff_hash}")
}

impl Workflow for SingleWorkerWorkflow {
    type Input = SingleWorkerInput;
    type State = SingleWorkerState;
    type Output = SingleWorkerOutput;

    fn step(
        &self,
        state: &mut Self::State,
        ctx: &mut WorkflowContext,
        event: Option<WorkflowEvent>,
    ) -> StepDecision<Self::Output> {
        if !state.initialized {
            state.initialized = true;
            state.start_ms = ctx.now_ms();
            let rem_budget = state
                .input
                .budget_sec
                .saturating_sub(state.prev_elapsed_ms / 1000)
                .max(1);
            let rem_cap = state
                .config
                .hard_cap_sec
                .saturating_sub(state.prev_elapsed_ms / 1000)
                .max(1);
            state.allowance_deadline_ms = state.start_ms + rem_budget * 1000;
            state.cap_deadline_ms = state.start_ms + rem_cap * 1000;

            // Subscribe once on the first step for scoped events (pane.exited)
            ctx.schedule_activity(
                "sub_events_0",
                ActivityRequest::HerdrEvents(HerdrEventRequest::Subscribe {
                    agent: state.input.name.clone(),
                    event_types: vec!["pane.exited".to_string()],
                }),
            );

            // Schedule initial tick observations and poll timer
            ctx.schedule_activity(
                format!("act_agent_info_{}", state.poll_count),
                ActivityRequest::Herdr(HerdrRequest::AgentInfo {
                    name: state.input.name.clone(),
                }),
            );
            ctx.schedule_activity(
                format!("check_report_{}", state.poll_count),
                ActivityRequest::ReportFile(ReportFileRequest::CheckMtime {
                    path: state.report_path.to_string_lossy().to_string(),
                }),
            );
            ctx.schedule_activity(
                format!("get_diff_hash_{}", state.poll_count),
                ActivityRequest::Git(GitRequest::DiffHash {
                    path: state.input.repo.clone(),
                }),
            );
            ctx.schedule_activity(
                format!("read_agent_{}", state.poll_count),
                ActivityRequest::Herdr(HerdrRequest::ReadAgent {
                    name: state.input.name.clone(),
                    lines: 80,
                }),
            );
            ctx.schedule_activity(
                format!("poll_events_{}", state.poll_count),
                ActivityRequest::HerdrEvents(HerdrEventRequest::PollEventsFor {
                    agent: state.input.name.clone(),
                }),
            );
            ctx.schedule_timer("poll_timer", Duration::from_secs(state.config.poll_sec));
            return StepDecision::Continue;
        }

        match event {
            None => StepDecision::Continue,

            Some(WorkflowEvent::ActivityCompleted { id, result }) => {
                if id.starts_with("act_agent_info_") {
                    if let ActivityResult::Herdr(HerdrResult::AgentInfo {
                        status,
                        harness,
                        hook_active,
                        ..
                    }) = result
                    {
                        state.agent_status = status.clone();
                        state.harness = harness;
                        state.hook_active = hook_active;
                        let is_gone = status == "gone" || status == "not_found";
                        if is_gone {
                            if state.gone_since_ms == 0 {
                                state.gone_since_ms = ctx.now_ms();
                            }
                        } else {
                            state.gone_since_ms = 0;
                        }

                        if status == "idle" || status == "done" {
                            if state.idle_since_ms == 0 {
                                state.idle_since_ms = ctx.now_ms();
                            }
                        } else {
                            state.idle_since_ms = 0;
                        }

                        if status == "blocked" {
                            return complete_step(state, ctx.now_ms(), 5);
                        }
                    }
                    StepDecision::Continue
                } else if id.starts_with("check_report_") {
                    let mtime_opt = match result {
                        ActivityResult::ReportFile(ReportFileResult::MtimeMs(mtime)) => Some(mtime),
                        ActivityResult::ReportFile(ReportFileResult::Exists(exists)) => {
                            Some(if exists { 1 } else { 0 })
                        }
                        _ => None,
                    };
                    if let Some(mtime) = mtime_opt {
                        if mtime == 0 {
                            state.report_mtime_ms = 0;
                            state.report_mtime_stable = false;
                        } else if state.report_mtime_ms == mtime {
                            // Two consecutive matching mtime samples -> stable
                            state.report_mtime_stable = true;
                            state.prompt_landed = true;
                            state.prompt_recovered = false;
                            ctx.schedule_activity(
                                format!("read_report_{}", state.poll_count),
                                ActivityRequest::ReportFile(ReportFileRequest::Read {
                                    path: state.report_path.to_string_lossy().to_string(),
                                }),
                            );
                        } else {
                            // First sample or modified mtime
                            state.report_mtime_ms = mtime;
                            state.report_mtime_stable = false;
                            state.prompt_landed = true;
                            state.prompt_recovered = false;
                        }
                    }
                    StepDecision::Continue
                } else if id.starts_with("read_report_") {
                    if let ActivityResult::ReportFile(ReportFileResult::Content(content)) = result {
                        // Exit 6 only on a stable, non-empty, malformed report; empty continues polling
                        if !content.trim().is_empty() && state.report_mtime_stable {
                            let valid =
                                validate_report_content(&content, state.config.expect_verdict);
                            let exit = if valid.is_ok() { 0 } else { 6 };
                            return complete_step(state, ctx.now_ms(), exit);
                        }
                    }
                    StepDecision::Continue
                } else if id.starts_with("get_diff_hash_") {
                    if let ActivityResult::Git(GitResult::DiffHash { hash, stat: _ }) = result {
                        state.current_diff_hash = hash.clone();
                        if state.last_autoext_diff_hash.is_empty() && !hash.is_empty() {
                            state.last_autoext_diff_hash = hash;
                        }

                        // Recompute progress fingerprint with updated diff hash
                        let fp =
                            compute_fingerprint(&state.latest_pane_text, &state.current_diff_hash);
                        if state.poll_count == 0 {
                            state.last_progress_fp = fp;
                        } else if fp != state.last_progress_fp {
                            state.last_progress_fp = fp;
                            state.last_progress_ms = ctx.now_ms();
                            if state.agent_status != "idle" && state.agent_status != "done" {
                                state.idle_since_ms = 0;
                            }
                        }
                    }
                    StepDecision::Continue
                } else if id.starts_with("read_agent_") {
                    if let ActivityResult::Herdr(HerdrResult::AgentOutput { content }) = result {
                        state.latest_pane_text = content.clone();

                        // Legacy watchdog: unknown status with empty pane indicates agent gone
                        if content.trim().is_empty() && state.agent_status == "unknown" {
                            state.agent_status = "gone".to_string();
                        }

                        // Interactive question / permission prompt pending
                        if question_seen(&content) {
                            return complete_step(state, ctx.now_ms(), 5);
                        }

                        // Recompute progress fingerprint with updated pane text
                        let fp =
                            compute_fingerprint(&state.latest_pane_text, &state.current_diff_hash);
                        if state.poll_count == 0 {
                            state.last_progress_fp = fp;
                        } else if fp != state.last_progress_fp {
                            state.last_progress_fp = fp;
                            state.last_progress_ms = ctx.now_ms();
                            if state.agent_status != "idle" && state.agent_status != "done" {
                                state.idle_since_ms = 0;
                            }
                        }

                        let now = ctx.now_ms();
                        let is_waiting = {
                            let runs_dir = state
                                .report_path
                                .parent()
                                .unwrap_or_else(|| std::path::Path::new("."));
                            crate::worker::prompt::observer::lead_waiting(
                                runs_dir,
                                &state.input.name,
                                "",
                            )
                        };
                        let stall_baseline = if state.last_progress_ms > 0 {
                            state.last_progress_ms
                        } else {
                            state.start_ms
                        };
                        if !is_waiting
                            && state.poll_count > 0
                            && state.report_mtime_ms == 0
                            && now.saturating_sub(stall_baseline) >= state.config.stall_sec * 1000
                        {
                            return complete_step(state, now, 3);
                        }
                    }
                    StepDecision::Continue
                } else if id.starts_with("poll_events_") {
                    if let ActivityResult::HerdrEvents(HerdrEventResult::ScopedEvents {
                        agent,
                        events,
                    }) = result
                    {
                        // Apply scoped events only for this agent; check for pane.exited
                        if agent == state.input.name {
                            for ev in events {
                                if ev.contains("pane.exited") {
                                    state.agent_status = "gone".to_string();
                                }
                            }
                        }
                    }
                    StepDecision::Continue
                } else {
                    StepDecision::Continue
                }
            }

            Some(WorkflowEvent::TimerFired { id }) => {
                if id == "poll_timer" {
                    state.poll_count += 1;
                    let now = ctx.now_ms();

                    // 1. Hard cap check (exit 2)
                    if now >= state.cap_deadline_ms {
                        return complete_step(state, now, 2);
                    }

                    // 2. Agent gone check (exit 3) - includes NotFound or unknown with empty pane
                    let is_gone = state.agent_status == "gone"
                        || state.agent_status == "not_found"
                        || (state.agent_status == "unknown"
                            && state.latest_pane_text.trim().is_empty());
                    if is_gone {
                        if state.gone_since_ms == 0 {
                            state.gone_since_ms = now;
                        }
                        if now.saturating_sub(state.gone_since_ms) >= state.config.gone_sec * 1000 {
                            return complete_step(state, now, 3);
                        }
                    } else {
                        state.gone_since_ms = 0;
                    }

                    // 3. Blocked / question check (exit 5)
                    if state.agent_status == "blocked" || question_seen(&state.latest_pane_text) {
                        return complete_step(state, now, 5);
                    }

                    // 4. Budget deadline check (exit 4, with grace and autoext) - checked BEFORE idle and stall
                    if now >= state.allowance_deadline_ms {
                        // Grace (+25% once) keys to progress fingerprint advancement within 2 polls
                        if !state.overrun_granted
                            && state.last_progress_ms > 0
                            && now.saturating_sub(state.last_progress_ms)
                                <= state.config.poll_sec * 2 * 1000
                        {
                            let mut grace_ms = (state.input.budget_sec / 4).max(1) * 1000;
                            if state.allowance_deadline_ms + grace_ms > state.cap_deadline_ms {
                                grace_ms = state
                                    .cap_deadline_ms
                                    .saturating_sub(state.allowance_deadline_ms);
                            }
                            if grace_ms > 0 {
                                state.allowance_deadline_ms += grace_ms;
                                state.overrun_granted = true;
                            }
                        }

                        // Auto-extension (+25% steps up to +50% total) keys strictly to diff hash changes
                        let max_autoext_ms = (state.input.budget_sec / 2) * 1000;
                        if state.extension_total_ms < max_autoext_ms
                            && !state.current_diff_hash.is_empty()
                            && state.current_diff_hash != state.last_autoext_diff_hash
                        {
                            let rem = max_autoext_ms.saturating_sub(state.extension_total_ms);
                            let mut ext_ms = ((state.input.budget_sec / 4).max(1) * 1000).min(rem);
                            if state.allowance_deadline_ms + ext_ms > state.cap_deadline_ms {
                                ext_ms = state
                                    .cap_deadline_ms
                                    .saturating_sub(state.allowance_deadline_ms);
                            }
                            if ext_ms > 0 {
                                state.allowance_deadline_ms += ext_ms;
                                state.extension_total_ms += ext_ms;
                                state.last_autoext_diff_hash = state.current_diff_hash.clone();
                            }
                        }

                        if now >= state.allowance_deadline_ms {
                            return complete_step(state, now, 4);
                        }
                    }

                    let is_waiting = {
                        let runs_dir = state
                            .report_path
                            .parent()
                            .unwrap_or_else(|| std::path::Path::new("."));
                        crate::worker::prompt::observer::lead_waiting(
                            runs_dir,
                            &state.input.name,
                            "",
                        )
                    };

                    if state.report_mtime_ms > 0 {
                        state.prompt_landed = true;
                        state.prompt_recovered = false;
                    }

                    if !state.prompt_landed && state.config.land_sec > 0 {
                        if state.agent_status == "working"
                            || state.agent_status == "blocked"
                            || is_waiting
                            || state.report_mtime_ms > 0
                        {
                            state.prompt_landed = true;
                            state.prompt_recovered = false;
                        } else if state.agent_status == "idle" || state.agent_status == "done" {
                            if state.prompt_recovered {
                                if now >= state.recover_deadline_ms && state.report_mtime_ms == 0 {
                                    return complete_step(state, now, 3);
                                }
                            } else if now >= state.start_ms + state.config.land_sec * 1000 {
                                state.prompt_recovered = true;
                                state.recover_at_ms = now;
                                state.recover_deadline_ms = now + state.config.land_sec * 1000;
                            }
                        }
                    }

                    // 5. Agent idle check (exit 3) - 120s standard, 300s fallback for opencode/omp without hook
                    if is_waiting {
                        state.idle_since_ms = 0;
                    } else if state.agent_status == "idle" || state.agent_status == "done" {
                        if state.idle_since_ms == 0 {
                            state.idle_since_ms = now;
                        }
                        let is_fallback_idle = (!state.hook_active)
                            && state
                                .harness
                                .as_deref()
                                .map(|h| h == "opencode" || h == "omp")
                                .unwrap_or(false);
                        let effective_idle_sec =
                            if is_fallback_idle || state.config.is_fallback_idle {
                                state.config.idle_fallback_sec
                            } else {
                                state.config.idle_sec
                            };
                        if state.report_mtime_ms == 0
                            && now.saturating_sub(state.idle_since_ms) >= effective_idle_sec * 1000
                        {
                            return complete_step(state, now, 3);
                        }
                    } else {
                        state.idle_since_ms = 0;
                    }

                    // 7. Nudge check (non-terminal)
                    let nudge_thresh_ms =
                        (state.input.budget_sec * state.config.nudge_pct / 100) * 1000;
                    let nudge_at_ms =
                        state.start_ms + nudge_thresh_ms.saturating_sub(state.prev_elapsed_ms);
                    if !state.nudged && now >= nudge_at_ms {
                        state.nudged = true;
                        let nudge_msg = format!(
                            "Time check: you are past {}% of this task's time budget. Wrap up now: finish the current step, commit your work, and write {} (via a temp file + rename) with what is done plus a Blockers section for the rest. Reply only the path.",
                            state.config.nudge_pct,
                            state.report_path.display()
                        );
                        ctx.schedule_activity(
                            format!("nudge_prompt_{}", state.poll_count),
                            ActivityRequest::Herdr(HerdrRequest::Prompt {
                                name: state.input.name.clone(),
                                task: nudge_msg,
                            }),
                        );
                    }

                    // 8. Schedule next tick observations and poll timer
                    ctx.schedule_activity(
                        format!("act_agent_info_{}", state.poll_count),
                        ActivityRequest::Herdr(HerdrRequest::AgentInfo {
                            name: state.input.name.clone(),
                        }),
                    );
                    ctx.schedule_activity(
                        format!("check_report_{}", state.poll_count),
                        ActivityRequest::ReportFile(ReportFileRequest::CheckMtime {
                            path: state.report_path.to_string_lossy().to_string(),
                        }),
                    );
                    ctx.schedule_activity(
                        format!("get_diff_hash_{}", state.poll_count),
                        ActivityRequest::Git(GitRequest::DiffHash {
                            path: state.input.repo.clone(),
                        }),
                    );
                    ctx.schedule_activity(
                        format!("read_agent_{}", state.poll_count),
                        ActivityRequest::Herdr(HerdrRequest::ReadAgent {
                            name: state.input.name.clone(),
                            lines: 80,
                        }),
                    );
                    ctx.schedule_activity(
                        format!("poll_events_{}", state.poll_count),
                        ActivityRequest::HerdrEvents(HerdrEventRequest::PollEventsFor {
                            agent: state.input.name.clone(),
                        }),
                    );
                    ctx.schedule_timer("poll_timer", Duration::from_secs(state.config.poll_sec));
                    StepDecision::Continue
                } else {
                    StepDecision::Fail {
                        code: 1,
                        reason: format!("unexpected timer fired: {id}"),
                    }
                }
            }

            _ => StepDecision::Fail {
                code: 1,
                reason: format!("unexpected workflow event: {event:?}"),
            },
        }
    }
}

// ----------------------------------------------------------------------------
// Deterministic Testkit Test Suite
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testkit::{
        run_to_completion, InMemoryJournal, MockActivities, VirtualClock,
    };

    fn sample_input(name: &str, budget_sec: u64) -> SingleWorkerInput {
        SingleWorkerInput::new(
            name,
            "sude",
            "quaqua",
            "Goal: implement worker workflow\nHard constraints: none\nEvidence: tests pass",
            budget_sec,
            None,
        )
    }

    fn mock_poll(
        activities: &mut MockActivities,
        agent: &str,
        poll: u64,
        mtime: u64,
        status: &str,
        pane_text: &str,
        diff_hash: &str,
        harness: Option<&str>,
        hook_active: bool,
        scoped_events: Vec<String>,
    ) {
        if poll == 0 {
            activities.register(
                "sub_events_0",
                ActivityResult::HerdrEvents(HerdrEventResult::Subscribed {
                    agent: agent.to_string(),
                }),
            );
        }
        activities.register(
            format!("act_agent_info_{poll}"),
            ActivityResult::Herdr(HerdrResult::AgentInfo {
                name: agent.to_string(),
                status: status.to_string(),
                harness: harness.map(|s| s.to_string()),
                hook_active,
            }),
        );
        activities.register(
            format!("check_report_{poll}"),
            ActivityResult::ReportFile(ReportFileResult::MtimeMs(mtime)),
        );
        activities.register(
            format!("get_diff_hash_{poll}"),
            ActivityResult::Git(GitResult::DiffHash {
                hash: diff_hash.to_string(),
                stat: if diff_hash.is_empty() {
                    "clean".to_string()
                } else {
                    "modified".to_string()
                },
            }),
        );
        activities.register(
            format!("read_agent_{poll}"),
            ActivityResult::Herdr(HerdrResult::AgentOutput {
                content: pane_text.to_string(),
            }),
        );
        activities.register(
            format!("poll_events_{poll}"),
            ActivityResult::HerdrEvents(HerdrEventResult::ScopedEvents {
                agent: agent.to_string(),
                events: scoped_events,
            }),
        );
        activities.register(
            format!("nudge_prompt_{poll}"),
            ActivityResult::Herdr(HerdrResult::Prompted {
                name: agent.to_string(),
            }),
        );
    }

    fn mock_poll_standard(
        activities: &mut MockActivities,
        agent: &str,
        poll: u64,
        mtime: u64,
        pane_text: &str,
        diff_hash: &str,
    ) {
        mock_poll(
            activities,
            agent,
            poll,
            mtime,
            "working",
            pane_text,
            diff_hash,
            Some("omp"),
            true,
            vec![],
        );
    }

    // ------------------------------------------------------------------------
    // Acceptance Test Cases
    // ------------------------------------------------------------------------

    #[test]
    fn test_single_worker_workflow_exit_0_success() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-ok", 1800);
        let report_path = PathBuf::from("runs/worker-ok.md");
        let initial_state = SingleWorkerState::new(input);

        // Poll 0: mtime 0
        mock_poll_standard(&mut activities, "worker-ok", 0, 0, "building", "dh0");
        // Poll 1: mtime 1000 (sample 1)
        mock_poll_standard(&mut activities, "worker-ok", 1, 1000, "building", "dh0");
        // Poll 2: mtime 1000 (sample 2 -> stable!)
        mock_poll_standard(&mut activities, "worker-ok", 2, 1000, "done", "dh0");
        activities.register(
            "read_report_2",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Report\nDone!\n\n## Lessons\n- none\n".to_string(),
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-exit-0",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete successfully");

        assert_eq!(out.exit, 0);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_single_worker_workflow_exit_2_hard_cap() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-cap", 10_000);
        let report_path = PathBuf::from("runs/worker-cap.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.hard_cap_sec = 90;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Diff moves on each poll so stall does not trigger
        mock_poll_standard(&mut activities, "worker-cap", 0, 0, "step 0", "dh0");
        mock_poll_standard(&mut activities, "worker-cap", 1, 0, "step 1", "dh1");
        mock_poll_standard(&mut activities, "worker-cap", 2, 0, "step 2", "dh2");
        mock_poll_standard(&mut activities, "worker-cap", 3, 0, "step 3", "dh3");

        let out = run_to_completion(
            &workflow,
            "test-exit-2",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete");

        assert_eq!(out.exit, 2);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_stall_fires_when_neither_pane_nor_diff_moves() {
        // Acceptance test: stall (exit 3) still fires when neither pane nor diff moves
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-stall", 1800);
        let report_path = PathBuf::from("runs/worker-stall.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.stall_sec = 60; // Stalls after 60s of static pane + static diff

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Same pane and same diff on all polls
        mock_poll_standard(
            &mut activities,
            "worker-stall",
            0,
            0,
            "frozen output",
            "static_dh",
        );
        mock_poll_standard(
            &mut activities,
            "worker-stall",
            1,
            0,
            "frozen output",
            "static_dh",
        );
        mock_poll_standard(
            &mut activities,
            "worker-stall",
            2,
            0,
            "frozen output",
            "static_dh",
        );

        let out = run_to_completion(
            &workflow,
            "test-stall-both-static",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete");

        assert_eq!(out.exit, 3);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_single_worker_workflow_exit_3_stall() {
        // Legacy test alias
        test_stall_fires_when_neither_pane_nor_diff_moves();
    }

    #[test]
    fn test_idle_fallback_300s_for_opencode_omp_without_hook_and_120s_otherwise() {
        // Acceptance test: idle fallback 300 s for opencode/omp without hook and 120 s otherwise

        // Case A: opencode without hook -> 300s fallback (does not exit at 120s, exits at 300s)
        {
            let workflow = SingleWorkerWorkflow::new();
            let mut clock = VirtualClock::new(100_000);
            let mut activities = MockActivities::new();
            let mut journal = InMemoryJournal::new();

            let input = sample_input("worker-fb", 1800);
            let initial_state = SingleWorkerState::new(input);

            mock_poll(
                &mut activities,
                "worker-fb",
                0,
                0,
                "working",
                "running",
                "dh0",
                Some("opencode"),
                false,
                vec![],
            );
            // Becomes idle at poll 1 (t=30s)
            for p in 1..=11 {
                mock_poll(
                    &mut activities,
                    "worker-fb",
                    p,
                    0,
                    "idle",
                    "idle",
                    "dh0",
                    Some("opencode"),
                    false, // !hook_active
                    vec![],
                );
            }

            let out = run_to_completion(
                &workflow,
                "test-idle-fb-300",
                initial_state,
                &mut clock,
                &mut activities,
                &mut journal,
            )
            .expect("workflow should complete with exit 3 after 300s fallback");

            assert_eq!(out.exit, 3);
            // 300s idle from t=30s is t=330s (100_000 + 330_000 = 430_000)
            assert_eq!(clock.now_ms(), 430_000);
        }

        // Case B: claude with hook_active=true -> 120s standard idle
        {
            let workflow = SingleWorkerWorkflow::new();
            let mut clock = VirtualClock::new(100_000);
            let mut activities = MockActivities::new();
            let mut journal = InMemoryJournal::new();

            let input = sample_input("worker-standard-idle", 1800);
            let initial_state = SingleWorkerState::new(input);

            mock_poll(
                &mut activities,
                "worker-standard-idle",
                0,
                0,
                "working",
                "running",
                "dh0",
                Some("claude"),
                true,
                vec![],
            );
            // Becomes idle at poll 1 (t=30s)
            for p in 1..=5 {
                mock_poll(
                    &mut activities,
                    "worker-standard-idle",
                    p,
                    0,
                    "idle",
                    "idle",
                    "dh0",
                    Some("claude"),
                    true,
                    vec![],
                );
            }

            let out = run_to_completion(
                &workflow,
                "test-idle-standard-120",
                initial_state,
                &mut clock,
                &mut activities,
                &mut journal,
            )
            .expect("workflow should complete with exit 3 after 120s");

            assert_eq!(out.exit, 3);
            // 120s idle from t=30s is t=150s (100_000 + 150_000 = 250_000)
            assert_eq!(clock.now_ms(), 250_000);
        }
    }

    #[test]
    fn test_single_worker_workflow_exit_3_idle() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-idle", 1800);
        let report_path = PathBuf::from("runs/worker-idle.md");
        let initial_state = SingleWorkerState::new(input);

        mock_poll(
            &mut activities,
            "worker-idle",
            0,
            0,
            "working",
            "running",
            "dh0",
            Some("claude"),
            true,
            vec![],
        );
        for p in 1..=5 {
            mock_poll(
                &mut activities,
                "worker-idle",
                p,
                0,
                "idle",
                "idle",
                "dh0",
                Some("claude"),
                true,
                vec![],
            );
        }

        let out = run_to_completion(
            &workflow,
            "test-exit-3-idle",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete");

        assert_eq!(out.exit, 3);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_gone_via_agent_info_not_found() {
        // Acceptance test: gone via AgentInfo NotFound / gone status
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-gone", 1800);
        let report_path = PathBuf::from("runs/worker-gone.md");
        let initial_state = SingleWorkerState::new(input);

        mock_poll_standard(&mut activities, "worker-gone", 0, 0, "working", "dh0");
        // Status reports "not_found" (legacy gone clock trigger)
        mock_poll(
            &mut activities,
            "worker-gone",
            1,
            0,
            "not_found",
            "",
            "dh0",
            None,
            false,
            vec![],
        );
        mock_poll(
            &mut activities,
            "worker-gone",
            2,
            0,
            "not_found",
            "",
            "dh0",
            None,
            false,
            vec![],
        );
        mock_poll(
            &mut activities,
            "worker-gone",
            3,
            0,
            "not_found",
            "",
            "dh0",
            None,
            false,
            vec![],
        );

        let out = run_to_completion(
            &workflow,
            "test-gone-not-found",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 3");

        assert_eq!(out.exit, 3);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_single_worker_workflow_exit_3_agent_gone() {
        test_gone_via_agent_info_not_found();
    }

    #[test]
    fn test_single_worker_workflow_exit_4_budget_exceeded() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-budget", 60);
        let report_path = PathBuf::from("runs/worker-budget.md");
        let initial_state = SingleWorkerState::new(input);

        mock_poll_standard(&mut activities, "worker-budget", 0, 0, "no progress", "dh0");
        mock_poll_standard(&mut activities, "worker-budget", 1, 0, "no progress", "dh0");
        mock_poll_standard(&mut activities, "worker-budget", 2, 0, "no progress", "dh0");

        let out = run_to_completion(
            &workflow,
            "test-exit-4",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 4");

        assert_eq!(out.exit, 4);
        assert_eq!(out.report_path, report_path);
        assert_eq!(clock.now_ms(), 100_000 + 60_000);
    }

    #[test]
    fn test_single_worker_workflow_exit_5_blocked_status() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-blocked", 1800);
        let report_path = PathBuf::from("runs/worker-blocked.md");
        let initial_state = SingleWorkerState::new(input);

        mock_poll(
            &mut activities,
            "worker-blocked",
            0,
            0,
            "blocked",
            "waiting on permission",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );

        let out = run_to_completion(
            &workflow,
            "test-exit-5-st",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 5");

        assert_eq!(out.exit, 5);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_single_worker_workflow_exit_5_question_detected() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-q", 1800);
        let report_path = PathBuf::from("runs/worker-q.md");
        let initial_state = SingleWorkerState::new(input);

        mock_poll_standard(
            &mut activities,
            "worker-q",
            0,
            0,
            "Do you want to proceed? [y/N]",
            "dh0",
        );

        let out = run_to_completion(
            &workflow,
            "test-exit-5-q",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 5");

        assert_eq!(out.exit, 5);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_stable_mtime_ready_vs_malformed_exit_6() {
        // Acceptance test: stable-mtime Ready vs malformed exit 6

        // Case A: malformed report exits 6 ONLY after stable mtime and non-empty content
        {
            let workflow = SingleWorkerWorkflow::new();
            let mut clock = VirtualClock::new(100_000);
            let mut activities = MockActivities::new();
            let mut journal = InMemoryJournal::new();

            let input = sample_input("worker-bad", 1800);
            let report_path = PathBuf::from("runs/worker-bad.md");
            let initial_state = SingleWorkerState::new(input);

            // Poll 0: sample 1 of mtime 1000 (not yet stable)
            mock_poll_standard(&mut activities, "worker-bad", 0, 1000, "working", "dh0");
            // Poll 1: sample 2 of mtime 1000 (now stable!)
            mock_poll_standard(&mut activities, "worker-bad", 1, 1000, "working", "dh0");
            activities.register(
                "read_report_1",
                ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Bad Report\nMissing lessons\n".to_string(),
                )),
            );

            let out = run_to_completion(
                &workflow,
                "test-bad-report",
                initial_state,
                &mut clock,
                &mut activities,
                &mut journal,
            )
            .expect("workflow should complete with exit 6 on stable malformed report");

            assert_eq!(out.exit, 6);
            assert_eq!(out.report_path, report_path);
        }

        // Case B: empty content does NOT exit 6; waits for non-empty report
        {
            let workflow = SingleWorkerWorkflow::new();
            let mut clock = VirtualClock::new(100_000);
            let mut activities = MockActivities::new();
            let mut journal = InMemoryJournal::new();

            let input = sample_input("worker-empty-wait", 1800);
            let initial_state = SingleWorkerState::new(input);

            mock_poll_standard(
                &mut activities,
                "worker-empty-wait",
                0,
                1000,
                "writing",
                "dh0",
            );
            mock_poll_standard(
                &mut activities,
                "worker-empty-wait",
                1,
                1000,
                "writing",
                "dh0",
            );
            // Stable mtime, but content is empty!
            activities.register(
                "read_report_1",
                ActivityResult::ReportFile(ReportFileResult::Content("   \n".to_string())),
            );

            // Poll 2: still stable, now content arrives!
            mock_poll_standard(&mut activities, "worker-empty-wait", 2, 1000, "done", "dh0");
            activities.register(
                "read_report_2",
                ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Done\n\n## Lessons\n- none\n".to_string(),
                )),
            );

            let out = run_to_completion(
                &workflow,
                "test-empty-content-wait",
                initial_state,
                &mut clock,
                &mut activities,
                &mut journal,
            )
            .expect("workflow should wait for non-empty content and exit 0");

            assert_eq!(out.exit, 0);
        }
    }

    #[test]
    fn test_single_worker_workflow_exit_6_malformed_report() {
        test_stable_mtime_ready_vs_malformed_exit_6();
    }

    #[test]
    fn test_autoext_fires_on_diff_hash_change_and_not_on_pane_churn_alone() {
        // Acceptance test: autoext fires on diff-hash change and NOT on pane churn alone

        // Case A: pane churn alone does NOT autoextend -> exits 4
        {
            let workflow = SingleWorkerWorkflow::new();
            let mut clock = VirtualClock::new(100_000);
            let mut activities = MockActivities::new();
            let mut journal = InMemoryJournal::new();

            // Budget 40s (poll 20s). Grace +10s (50s).
            let input = sample_input("worker-churn-no-ext", 40);
            let mut config = WatchdogConfig::default();
            config.poll_sec = 20;
            config.hard_cap_sec = 200;
            config.stall_sec = 600;

            let initial_state = SingleWorkerState::new(input).with_config(config);

            // Pane text churns on every poll, but diff hash is static
            mock_poll_standard(
                &mut activities,
                "worker-churn-no-ext",
                0,
                0,
                "spin |",
                "dh_static",
            );
            mock_poll_standard(
                &mut activities,
                "worker-churn-no-ext",
                1,
                0,
                "spin /",
                "dh_static",
            );
            mock_poll_standard(
                &mut activities,
                "worker-churn-no-ext",
                2,
                0,
                "spin -",
                "dh_static",
            );
            mock_poll_standard(
                &mut activities,
                "worker-churn-no-ext",
                3,
                0,
                "spin \\",
                "dh_static",
            );

            let out = run_to_completion(
                &workflow,
                "test-churn-fails",
                initial_state,
                &mut clock,
                &mut activities,
                &mut journal,
            )
            .expect("workflow should complete with exit 4 without autoext");

            assert_eq!(out.exit, 4);
        }

        // Case B: diff hash changes -> auto-extends up to +50%
        {
            let workflow = SingleWorkerWorkflow::new();
            let mut clock = VirtualClock::new(100_000);
            let mut activities = MockActivities::new();
            let mut journal = InMemoryJournal::new();

            let input = sample_input("worker-diff-ext", 40);
            let mut config = WatchdogConfig::default();
            config.poll_sec = 20;
            config.hard_cap_sec = 200;
            config.stall_sec = 600;

            let initial_state = SingleWorkerState::new(input).with_config(config);

            mock_poll_standard(&mut activities, "worker-diff-ext", 0, 0, "step 0", "dh1");
            mock_poll_standard(&mut activities, "worker-diff-ext", 1, 0, "step 1", "dh2");
            // Poll 2 (t=40s): budget reached -> grace (+10s -> 50s) + autoext on dh3 (+10s -> 60s)
            mock_poll_standard(&mut activities, "worker-diff-ext", 2, 0, "step 2", "dh3");
            // Poll 3 (t=60s): autoext on dh4 (+10s -> 70s, max +50% reached)
            mock_poll_standard(&mut activities, "worker-diff-ext", 3, 0, "step 3", "dh4");
            // Poll 4 (t=80s): past 70s max -> exits 4
            mock_poll_standard(&mut activities, "worker-diff-ext", 4, 0, "step 4", "dh5");

            let out = run_to_completion(
                &workflow,
                "test-diff-autoext",
                initial_state,
                &mut clock,
                &mut activities,
                &mut journal,
            )
            .expect("workflow should complete with exit 4 after reaching max autoext");

            assert_eq!(out.exit, 4);
        }
    }

    #[test]
    fn test_single_worker_workflow_autoext_bounded() {
        test_autoext_fires_on_diff_hash_change_and_not_on_pane_churn_alone();
    }

    #[test]
    fn test_grace_after_a_diff_move() {
        // Acceptance test: grace after a diff move
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        // 80s budget with 20s poll
        let input = sample_input("worker-grace-diff", 80);
        let mut config = WatchdogConfig::default();
        config.poll_sec = 20;
        config.hard_cap_sec = 200;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Diff moves at poll 3 (t=60s), which is within 2 polls of 80s budget!
        mock_poll_standard(&mut activities, "worker-grace-diff", 0, 0, "same", "dh0");
        mock_poll_standard(&mut activities, "worker-grace-diff", 1, 0, "same", "dh0");
        mock_poll_standard(&mut activities, "worker-grace-diff", 2, 0, "same", "dh0");
        mock_poll_standard(
            &mut activities,
            "worker-grace-diff",
            3,
            0,
            "same",
            "dh1_moved",
        );

        // Poll 4 (t=80s): reaches budget, grace (+20s = 80/4) extends deadline to 100s!
        // Report lands in grace window: sample 1 at poll 4, sample 2 at poll 5
        mock_poll_standard(
            &mut activities,
            "worker-grace-diff",
            4,
            1000,
            "same",
            "dh1_moved",
        );
        mock_poll_standard(
            &mut activities,
            "worker-grace-diff",
            5,
            1000,
            "same",
            "dh1_moved",
        );
        activities.register(
            "read_report_5",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Report\nLanded in grace\n\n## Lessons\n- none\n".to_string(),
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-grace-diff-run",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 0 during grace");

        assert_eq!(out.exit, 0);
    }

    #[test]
    fn test_single_worker_workflow_grace_extension() {
        test_grace_after_a_diff_move();
    }

    #[test]
    fn test_single_worker_workflow_autoext_cap_clamped() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        // 80s budget, 90s hard cap
        let input = sample_input("worker-clamped", 80);
        let report_path = PathBuf::from("runs/worker-clamped.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 20;
        config.hard_cap_sec = 90;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        mock_poll_standard(&mut activities, "worker-clamped", 0, 0, "diff 0", "dh0");
        mock_poll_standard(&mut activities, "worker-clamped", 1, 0, "diff 1", "dh1");
        mock_poll_standard(&mut activities, "worker-clamped", 2, 0, "diff 2", "dh2");
        mock_poll_standard(&mut activities, "worker-clamped", 3, 0, "diff 3", "dh3");
        // Poll 4 (t=80s): budget reached, grace clamped to 10s (deadline 90s)
        mock_poll_standard(&mut activities, "worker-clamped", 4, 0, "diff 4", "dh4");
        // Poll 5 (t=100s): now >= 90s hard cap -> exit 2!
        mock_poll_standard(&mut activities, "worker-clamped", 5, 0, "diff 5", "dh5");

        let out = run_to_completion(
            &workflow,
            "test-clamped",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 2 when hard cap reached");

        assert_eq!(out.exit, 2);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_single_worker_workflow_determinism_two_runs() {
        let run = || {
            let workflow = SingleWorkerWorkflow::new();
            let mut clock = VirtualClock::new(100_000);
            let mut activities = MockActivities::new();
            let mut journal = InMemoryJournal::new();

            let input = sample_input("worker-det", 1800);
            let initial_state = SingleWorkerState::new(input);

            mock_poll_standard(&mut activities, "worker-det", 0, 0, "step 1", "dh0");
            mock_poll_standard(&mut activities, "worker-det", 1, 1000, "step 2", "dh1");
            mock_poll_standard(&mut activities, "worker-det", 2, 1000, "step 3", "dh1");
            activities.register(
                "read_report_2",
                ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Report\nDeterministic!\n\n## Lessons\n- none\n".to_string(),
                )),
            );

            let out = run_to_completion(
                &workflow,
                "det-run",
                initial_state,
                &mut clock,
                &mut activities,
                &mut journal,
            )
            .expect("workflow should complete");

            (out, journal.all_events("det-run").to_vec())
        };

        let (out1, events1) = run();
        let (out2, events2) = run();

        assert_eq!(out1, out2);
        assert_eq!(events1, events2);
    }

    #[test]
    fn test_budget_precedence_over_idle_and_stall() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-precedence", 60);
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.idle_sec = 30;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        mock_poll_standard(&mut activities, "worker-precedence", 0, 0, "idle", "dh0");
        // Poll 1 (t=30s): becomes idle
        mock_poll(
            &mut activities,
            "worker-precedence",
            1,
            0,
            "idle",
            "idle",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );
        // Poll 2 (t=60s): budget reached while idle -> exit 4 takes precedence over exit 3
        mock_poll(
            &mut activities,
            "worker-precedence",
            2,
            0,
            "idle",
            "idle",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );

        let out = run_to_completion(
            &workflow,
            "test-precedence",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 4");

        assert_eq!(out.exit, 4);
    }

    #[test]
    fn test_nudge_prompt_activity_scheduled_at_80_percent() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-nudge", 100);
        let mut config = WatchdogConfig::default();
        config.poll_sec = 20;
        config.nudge_pct = 80;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        mock_poll_standard(&mut activities, "worker-nudge", 0, 0, "step 0", "dh0");
        mock_poll_standard(&mut activities, "worker-nudge", 1, 0, "step 1", "dh1");
        mock_poll_standard(&mut activities, "worker-nudge", 2, 0, "step 2", "dh2");
        mock_poll_standard(&mut activities, "worker-nudge", 3, 0, "step 3", "dh3");
        // Poll 4 (t=80s): nudge threshold reached!
        mock_poll_standard(&mut activities, "worker-nudge", 4, 1000, "step 4", "dh4");
        // Poll 5 (t=100s): stable mtime
        mock_poll_standard(&mut activities, "worker-nudge", 5, 1000, "step 5", "dh4");
        activities.register(
            "read_report_5",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Report\nLanded after nudge!\n\n## Lessons\n- none\n".to_string(),
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-nudge",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete");

        assert_eq!(out.exit, 0);

        let events = journal.all_events("test-nudge");
        assert!(
            events
                .iter()
                .any(|e| e.payload_json.contains("Time check: you are past 80%")),
            "nudge prompt was not recorded in journal"
        );
    }

    #[test]
    fn test_builder_brief_mentioning_verdict_does_not_require_verdict() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = SingleWorkerInput::new(
            "builder-mentions-verdict",
            "sude",
            "quaqua",
            "Goal: build something. Note that the reviewer will check for Verdict: clean.",
            1800,
            None,
        );
        let initial_state = SingleWorkerState::new(input);

        mock_poll_standard(
            &mut activities,
            "builder-mentions-verdict",
            0,
            1000,
            "working",
            "dh0",
        );
        mock_poll_standard(
            &mut activities,
            "builder-mentions-verdict",
            1,
            1000,
            "working",
            "dh0",
        );
        activities.register(
            "read_report_1",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Builder Report\nSuccess!\n\n## Lessons\n- none\n".to_string(),
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-builder-verdict",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 0 for builder report");

        assert_eq!(out.exit, 0);
    }

    #[test]
    fn test_scoped_events_pane_exited_starts_gone() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-pane-exited", 1800);
        let initial_state = SingleWorkerState::new(input);

        mock_poll_standard(
            &mut activities,
            "worker-pane-exited",
            0,
            0,
            "working",
            "dh0",
        );
        // Poll 1: scoped event pane.exited arrives
        mock_poll(
            &mut activities,
            "worker-pane-exited",
            1,
            0,
            "working",
            "crashed",
            "dh0",
            Some("omp"),
            true,
            vec!["pane.exited".to_string()],
        );
        // Gone for 60s
        mock_poll(
            &mut activities,
            "worker-pane-exited",
            2,
            0,
            "gone",
            "",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );
        mock_poll(
            &mut activities,
            "worker-pane-exited",
            3,
            0,
            "gone",
            "",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );

        let out = run_to_completion(
            &workflow,
            "test-scoped-exited",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 3 after pane.exited");

        assert_eq!(out.exit, 3);
    }
    #[test]
    fn test_fast_finish_report_valid_before_second_poll_exits_0() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-fast-0", 1800);
        let report_path = PathBuf::from("runs/worker-fast-0.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.land_sec = 15;
        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Poll 0 (t=0): report already on disk with mtime 1000, agent done
        mock_poll(
            &mut activities,
            "worker-fast-0",
            0,
            1000,
            "done",
            "done",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );
        // Poll 1 (t=30s): matching mtime 1000 -> stable before second timer poll (poll 2)!
        mock_poll(
            &mut activities,
            "worker-fast-0",
            1,
            1000,
            "done",
            "done",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );
        activities.register(
            "read_report_1",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Report\nFast done!\n\n## Lessons\n- none\n".to_string(),
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-fast-exit-0",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 0");

        assert_eq!(out.exit, 0);
        assert_eq!(out.report_path, report_path);
        // Completed at poll 1 (t=30s), before poll 2!
        assert_eq!(clock.now_ms(), 100_000 + 30_000);
    }

    #[test]
    fn test_fast_finish_landing_timer_expiry_with_valid_report_exits_0() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-fast-land", 1800);
        let report_path = PathBuf::from("runs/worker-fast-land.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.land_sec = 15;
        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Poll 0 (t=0): report not yet on disk (mtime 0), agent done
        mock_poll(
            &mut activities,
            "worker-fast-land",
            0,
            0,
            "done",
            "done",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );
        // Poll 1 (t=30s): report appears on disk (mtime 1000, sample 1), agent done
        mock_poll(
            &mut activities,
            "worker-fast-land",
            1,
            1000,
            "done",
            "done",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );
        // Poll 2 (t=60s): recover_deadline_ms (45s) is expired!
        // But report is on disk (mtime 1000, sample 2 -> stable!)
        mock_poll(
            &mut activities,
            "worker-fast-land",
            2,
            1000,
            "done",
            "done",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );
        activities.register(
            "read_report_2",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Report\nFast done after poll 0!\n\n## Lessons\n- none\n".to_string(),
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-fast-land-exit-0",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 0");

        assert_eq!(out.exit, 0);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_fast_finish_malformed_report_exits_6() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-fast-bad", 1800);
        let report_path = PathBuf::from("runs/worker-fast-bad.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.land_sec = 15;
        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Poll 0 (t=0): report already on disk (mtime 1000), agent done
        mock_poll(
            &mut activities,
            "worker-fast-bad",
            0,
            1000,
            "done",
            "done",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );
        // Poll 1 (t=30s): matching mtime 1000 -> stable!
        mock_poll(
            &mut activities,
            "worker-fast-bad",
            1,
            1000,
            "done",
            "done",
            "dh0",
            Some("omp"),
            true,
            vec![],
        );
        // Malformed content: missing `## Lessons`
        activities.register(
            "read_report_1",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Report\nFast done but malformed!\n".to_string(),
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-fast-exit-6",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 6");

        assert_eq!(out.exit, 6);
        assert_eq!(out.report_path, report_path);
    }
}
