//! SingleWorkerWorkflow implementation over the frozen W0 engine.
//!
//! Evaluates worker watchdog rules deterministically:
//! - Exit 0: Success (report ready and validates `## Lessons` or `Verdict:`)
//! - Exit 2: Hard timeout / cap exceeded
//! - Exit 3: Stall (no progress for 600 s) / idle (120 s, 300 s fallback) / agent gone (60 s)
//! - Exit 4: Time budget exceeded (after one +25% grace and bounded auto-extensions up to +50% total)
//! - Exit 5: Blocked on question / permission
//! - Exit 6: Malformed report (missing `## Lessons` or `Verdict:`)
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
    pub budget_deadline_ms: u64,
    pub cap_deadline_ms: u64,
    pub last_progress_ms: u64,
    pub first_progress_seen: bool,
    pub last_progress_hash: String,
    pub current_sha: String,
    pub last_autoext_sha: String,
    pub latest_pane_text: String,
    pub agent_status: String,
    pub idle_since_ms: u64,
    pub gone_since_ms: u64,
    pub grace_used: bool,
    pub autoext_total_ms: u64,
    pub poll_count: u64,
    pub nudged: bool,
    pub initialized: bool,
    pub report_mtime_ms: u64,
}

impl SingleWorkerState {
    pub fn new(input: SingleWorkerInput) -> Self {
        let report_path = PathBuf::from(format!("runs/{}.md", input.name));
        // Only actual pipeline review phases expect Verdict: lines (Finding 6)
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
            budget_deadline_ms: 0,
            cap_deadline_ms: 0,
            last_progress_ms: 0,
            first_progress_seen: false,
            last_progress_hash: String::new(),
            current_sha: String::new(),
            last_autoext_sha: String::new(),
            latest_pane_text: String::new(),
            agent_status: "working".to_string(),
            idle_since_ms: 0,
            gone_since_ms: 0,
            grace_used: false,
            autoext_total_ms: 0,
            poll_count: 0,
            nudged: false,
            initialized: false,
            report_mtime_ms: 0,
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
        grace_used: bool,
        autoext_total_sec: u64,
        nudged: bool,
        last_progress_sec: u64,
    ) -> Self {
        self.prev_elapsed_ms = prev_elapsed_sec * 1000;
        self.grace_used = grace_used;
        self.autoext_total_ms = autoext_total_sec * 1000;
        self.nudged = nudged;
        if last_progress_sec > 0 {
            self.last_progress_ms = last_progress_sec * 1000;
            self.first_progress_seen = true;
        }
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

/// Computes a deterministic hash of pane output lines to detect progress.
fn hash_pane_content(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            trimmed.hash(&mut hasher);
        }
    }
    format!("{:016x}", hasher.finish())
}

/// Parses an agent status event string into a normalized status.
fn parse_status_event(event: &str) -> Option<String> {
    let trimmed = event.trim();
    if trimmed == "idle"
        || trimmed == "done"
        || trimmed == "blocked"
        || trimmed == "gone"
        || trimmed == "working"
    {
        return Some(trimmed.to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("status:") {
        return Some(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("agent_status:") {
        return Some(rest.trim().to_string());
    }
    if trimmed == "agent_idle" {
        return Some("idle".to_string());
    }
    if trimmed == "agent_blocked" {
        return Some("blocked".to_string());
    }
    if trimmed == "agent_gone" {
        return Some("gone".to_string());
    }
    if trimmed == "agent_working" {
        return Some("working".to_string());
    }
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(s) = val.get("agent_status").and_then(|v| v.as_str()) {
                return Some(s.to_string());
            }
            if let Some(s) = val.get("status").and_then(|v| v.as_str()) {
                return Some(s.to_string());
            }
        }
    }
    None
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
            state.budget_deadline_ms = state.start_ms + rem_budget * 1000;
            state.cap_deadline_ms = state.start_ms + rem_cap * 1000;

            // Schedule initial observation activities and poll timer
            ctx.schedule_activity(
                format!("check_report_{}", state.poll_count),
                ActivityRequest::ReportFile(ReportFileRequest::CheckMtime {
                    path: state.report_path.to_string_lossy().to_string(),
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
                ActivityRequest::HerdrEvents(HerdrEventRequest::PollEvents),
            );
            ctx.schedule_activity(
                format!("get_sha_{}", state.poll_count),
                ActivityRequest::Git(GitRequest::CurrentSha),
            );
            ctx.schedule_timer("poll_timer", Duration::from_secs(state.config.poll_sec));
            return StepDecision::Continue;
        }

        match event {
            None => StepDecision::Continue,

            Some(WorkflowEvent::ActivityCompleted { id, result }) => {
                if id.starts_with("check_report_") {
                    match result {
                        ActivityResult::ReportFile(ReportFileResult::MtimeMs(mtime)) => {
                            state.report_mtime_ms = mtime;
                            if mtime > 0 {
                                ctx.schedule_activity(
                                    format!("read_report_{}", state.poll_count),
                                    ActivityRequest::ReportFile(ReportFileRequest::Read {
                                        path: state.report_path.to_string_lossy().to_string(),
                                    }),
                                );
                            }
                        }
                        ActivityResult::ReportFile(ReportFileResult::Exists(exists)) => {
                            if exists {
                                ctx.schedule_activity(
                                    format!("read_report_{}", state.poll_count),
                                    ActivityRequest::ReportFile(ReportFileRequest::Read {
                                        path: state.report_path.to_string_lossy().to_string(),
                                    }),
                                );
                            }
                        }
                        _ => {}
                    }
                    StepDecision::Continue
                } else if id.starts_with("read_report_") {
                    if let ActivityResult::ReportFile(ReportFileResult::Content(content)) = result {
                        // Finding 1: An empty report is not ready yet; do not exit 6 immediately, keep polling
                        if !content.trim().is_empty() {
                            let valid =
                                validate_report_content(&content, state.config.expect_verdict);
                            let exit = if valid.is_ok() { 0 } else { 6 };
                            return StepDecision::Complete(
                                SingleWorkerOutput::new(exit, state.report_path.clone())
                                    .expect("valid single worker exit code"),
                            );
                        }
                    }
                    StepDecision::Continue
                } else if id.starts_with("read_agent_") {
                    if let ActivityResult::Herdr(HerdrResult::AgentOutput { content }) = result {
                        state.latest_pane_text = content.clone();

                        // Legacy watchdog: empty pane text indicates agent gone / unobservable
                        if content.trim().is_empty() {
                            state.agent_status = "gone".to_string();
                        } else {
                            if state.agent_status == "gone" {
                                state.agent_status = "working".to_string();
                            }

                            // Interactive prompt pending
                            if question_seen(&content) {
                                return StepDecision::Complete(
                                    SingleWorkerOutput::new(5, state.report_path.clone())
                                        .expect("valid single worker exit code"),
                                );
                            }

                            // Progress hash tracking (Finding 9: first observation is baseline, does not update progress timestamp)
                            let h = hash_pane_content(&content);
                            if !h.is_empty() {
                                if !state.first_progress_seen {
                                    state.first_progress_seen = true;
                                    state.last_progress_hash = h;
                                } else if h != state.last_progress_hash {
                                    state.last_progress_hash = h;
                                    state.last_progress_ms = ctx.now_ms();
                                }
                            }
                        }
                    }
                    StepDecision::Continue
                } else if id.starts_with("poll_events_") {
                    // Finding 3: Agent status from HerdrEvents
                    if let ActivityResult::HerdrEvents(HerdrEventResult::Events(events)) = result {
                        for ev in events {
                            if let Some(st) = parse_status_event(&ev) {
                                state.agent_status = st.clone();
                                if st == "blocked" {
                                    return StepDecision::Complete(
                                        SingleWorkerOutput::new(5, state.report_path.clone())
                                            .expect("valid single worker exit code"),
                                    );
                                }
                            }
                        }
                    }
                    StepDecision::Continue
                } else if id.starts_with("get_sha_") {
                    // Finding 2: Worktree Git SHA tracking for auto-extension
                    if let ActivityResult::Git(GitResult::Sha(sha)) = result {
                        state.current_sha = sha.clone();
                        if state.last_autoext_sha.is_empty() && !sha.is_empty() {
                            state.last_autoext_sha = sha;
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

                    // Finding 4: Reorder checks to match legacy watchdog.rs:267-396:
                    // 1. Hard cap (exit 2)
                    if now >= state.cap_deadline_ms {
                        return StepDecision::Complete(
                            SingleWorkerOutput::new(2, state.report_path.clone())
                                .expect("valid single worker exit code"),
                        );
                    }

                    // 2. Agent gone check (exit 3)
                    if state.agent_status == "gone" {
                        if state.gone_since_ms == 0 {
                            state.gone_since_ms = now;
                        }
                        if now.saturating_sub(state.gone_since_ms) >= state.config.gone_sec * 1000 {
                            return StepDecision::Complete(
                                SingleWorkerOutput::new(3, state.report_path.clone())
                                    .expect("valid single worker exit code"),
                            );
                        }
                    } else {
                        state.gone_since_ms = 0;
                    }

                    // 3. Blocked / question check (exit 5)
                    if state.agent_status == "blocked" || question_seen(&state.latest_pane_text) {
                        return StepDecision::Complete(
                            SingleWorkerOutput::new(5, state.report_path.clone())
                                .expect("valid single worker exit code"),
                        );
                    }

                    // 4. Budget deadline check (exit 4, with grace and autoext)
                    // Evaluated BEFORE idle and stall, matching legacy watchdog!
                    if now >= state.budget_deadline_ms {
                        // One +25% grace if progress was seen within 2 polls
                        if !state.grace_used
                            && state.last_progress_ms > 0
                            && now.saturating_sub(state.last_progress_ms)
                                <= state.config.poll_sec * 2 * 1000
                        {
                            let mut grace_ms = (state.input.budget_sec / 4).max(1) * 1000;
                            if state.budget_deadline_ms + grace_ms > state.cap_deadline_ms {
                                grace_ms = state
                                    .cap_deadline_ms
                                    .saturating_sub(state.budget_deadline_ms);
                            }
                            if grace_ms > 0 {
                                state.budget_deadline_ms += grace_ms;
                                state.grace_used = true;
                            }
                        }

                        // Finding 2: Auto-extensions require Git::CurrentSha advancement, not pane churn alone
                        let max_autoext_ms = (state.input.budget_sec / 2) * 1000;
                        if state.autoext_total_ms < max_autoext_ms
                            && !state.current_sha.is_empty()
                            && state.current_sha != state.last_autoext_sha
                        {
                            let rem = max_autoext_ms.saturating_sub(state.autoext_total_ms);
                            let mut ext_ms = ((state.input.budget_sec / 4).max(1) * 1000).min(rem);
                            if state.budget_deadline_ms + ext_ms > state.cap_deadline_ms {
                                ext_ms = state
                                    .cap_deadline_ms
                                    .saturating_sub(state.budget_deadline_ms);
                            }
                            if ext_ms > 0 {
                                state.budget_deadline_ms += ext_ms;
                                state.autoext_total_ms += ext_ms;
                                state.last_autoext_sha = state.current_sha.clone();
                            }
                        }

                        if now >= state.budget_deadline_ms {
                            return StepDecision::Complete(
                                SingleWorkerOutput::new(4, state.report_path.clone())
                                    .expect("valid single worker exit code"),
                            );
                        }
                    }

                    // 5. Agent idle check (exit 3) - evaluated after budget
                    if state.agent_status == "idle" || state.agent_status == "done" {
                        if state.idle_since_ms == 0 {
                            state.idle_since_ms = now;
                        }
                        let effective_idle_sec = if state.config.is_fallback_idle {
                            state.config.idle_fallback_sec
                        } else {
                            state.config.idle_sec
                        };
                        if now.saturating_sub(state.idle_since_ms) >= effective_idle_sec * 1000 {
                            return StepDecision::Complete(
                                SingleWorkerOutput::new(3, state.report_path.clone())
                                    .expect("valid single worker exit code"),
                            );
                        }
                    } else {
                        state.idle_since_ms = 0;
                    }

                    // 6. Stall check (exit 3) - evaluated after budget
                    let stall_baseline = if state.last_progress_ms > 0 {
                        state.last_progress_ms
                    } else {
                        state.start_ms
                    };
                    if now.saturating_sub(stall_baseline) >= state.config.stall_sec * 1000 {
                        return StepDecision::Complete(
                            SingleWorkerOutput::new(3, state.report_path.clone())
                                .expect("valid single worker exit code"),
                        );
                    }

                    // 7. Nudge check (non-terminal, Finding 5: subtracts prev_elapsed_ms and schedules Prompt)
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

                    // 8. Schedule next observation activities and timer
                    ctx.schedule_activity(
                        format!("check_report_{}", state.poll_count),
                        ActivityRequest::ReportFile(ReportFileRequest::CheckMtime {
                            path: state.report_path.to_string_lossy().to_string(),
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
                        ActivityRequest::HerdrEvents(HerdrEventRequest::PollEvents),
                    );
                    ctx.schedule_activity(
                        format!("get_sha_{}", state.poll_count),
                        ActivityRequest::Git(GitRequest::CurrentSha),
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
        poll: u64,
        report_mtime: u64,
        agent_content: &str,
    ) {
        activities.register(
            format!("check_report_{poll}"),
            ActivityResult::ReportFile(ReportFileResult::MtimeMs(report_mtime)),
        );
        activities.register(
            format!("read_agent_{poll}"),
            ActivityResult::Herdr(HerdrResult::AgentOutput {
                content: agent_content.to_string(),
            }),
        );
        activities.register(
            format!("poll_events_{poll}"),
            ActivityResult::HerdrEvents(HerdrEventResult::Events(vec![])),
        );
        activities.register(
            format!("get_sha_{poll}"),
            ActivityResult::Git(GitResult::Sha(String::new())),
        );
        activities.register(
            format!("nudge_prompt_{poll}"),
            ActivityResult::Herdr(HerdrResult::Prompted {
                name: "worker".to_string(),
            }),
        );
    }

    #[test]
    fn test_single_worker_workflow_exit_0_success() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-ok", 1800);
        let report_path = PathBuf::from("runs/worker-ok.md");
        let initial_state = SingleWorkerState::new(input);

        // Poll 0: report mtime is 0 (not ready yet), worker is working
        mock_poll(&mut activities, 0, 0, "Compiling crates/quaqua...");
        // Poll 1: report exists with mtime > 0
        mock_poll(&mut activities, 1, 1000, "Finished compiling");
        activities.register(
            "read_report_1",
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
        config.hard_cap_sec = 90; // Exceeds at poll 3 (90s >= 90s)
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Advance through polls while making progress so stall doesn't trigger
        mock_poll(&mut activities, 0, 0, "step 0");
        mock_poll(&mut activities, 1, 0, "step 1");
        mock_poll(&mut activities, 2, 0, "step 2");
        mock_poll(&mut activities, 3, 0, "step 3");

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
    fn test_single_worker_workflow_exit_3_stall() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-stall", 1800);
        let report_path = PathBuf::from("runs/worker-stall.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.stall_sec = 60; // Stalls after 60s (2 polls of unchanged output)

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Same output on all polls -> last_progress_ms stays at start_ms (100_000)
        mock_poll(&mut activities, 0, 0, "frozen output");
        mock_poll(&mut activities, 1, 0, "frozen output");
        mock_poll(&mut activities, 2, 0, "frozen output");

        let out = run_to_completion(
            &workflow,
            "test-exit-3-stall",
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
    fn test_single_worker_workflow_exit_3_idle() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-idle", 1800);
        let report_path = PathBuf::from("runs/worker-idle.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.idle_sec = 60; // Exits idle after 60s

        let initial_state = SingleWorkerState::new(input).with_config(config);

        mock_poll(&mut activities, 0, 0, "working");
        mock_poll(&mut activities, 1, 0, "working");
        activities.register(
            "poll_events_1",
            ActivityResult::HerdrEvents(HerdrEventResult::Events(vec!["status: idle".to_string()])),
        );
        mock_poll(&mut activities, 2, 0, "working");
        mock_poll(&mut activities, 3, 0, "working");

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
    fn test_single_worker_workflow_exit_3_idle_fallback() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let mut input = sample_input("worker-idle-fallback", 1800);
        input.kind = "opencode".to_string(); // triggers is_fallback_idle
        let report_path = PathBuf::from("runs/worker-idle-fallback.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.idle_sec = 30;
        config.idle_fallback_sec = 90; // Fallback idle is 90s instead of 30s
        config.is_fallback_idle = true;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        mock_poll(&mut activities, 0, 0, "working");
        mock_poll(&mut activities, 1, 0, "working");
        activities.register(
            "poll_events_1",
            ActivityResult::HerdrEvents(HerdrEventResult::Events(vec!["status: idle".to_string()])),
        );
        mock_poll(&mut activities, 2, 0, "working"); // 30s idle - not reached 90s yet
        mock_poll(&mut activities, 3, 0, "working"); // 60s idle - not reached 90s yet
        mock_poll(&mut activities, 4, 0, "working"); // 90s idle - reached!

        let out = run_to_completion(
            &workflow,
            "test-exit-3-idle-fb",
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
    fn test_single_worker_workflow_exit_3_agent_gone() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-gone", 1800);
        let report_path = PathBuf::from("runs/worker-gone.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.gone_sec = 60;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Empty pane text marks agent as gone (matching legacy watchdog.rs:267)
        mock_poll(&mut activities, 0, 0, "working");
        mock_poll(&mut activities, 1, 0, "");
        mock_poll(&mut activities, 2, 0, "");
        mock_poll(&mut activities, 3, 0, "");

        let out = run_to_completion(
            &workflow,
            "test-exit-3-gone",
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
    fn test_single_worker_workflow_exit_4_budget_exceeded() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        // 60s budget with 30s poll. Output never changes after start -> first_progress_seen is baseline,
        // last_progress_ms stays 0, so grace is NOT eligible! Exits at t=60s.
        let input = sample_input("worker-budget", 60);
        let report_path = PathBuf::from("runs/worker-budget.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        mock_poll(&mut activities, 0, 0, "no progress");
        mock_poll(&mut activities, 1, 0, "no progress");
        mock_poll(&mut activities, 2, 0, "no progress");

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
        // Assert virtual clock advanced exactly to budget deadline (60s = 60_000ms), NOT 90s
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

        mock_poll(&mut activities, 0, 0, "working");
        activities.register(
            "poll_events_0",
            ActivityResult::HerdrEvents(HerdrEventResult::Events(
                vec!["agent_blocked".to_string()],
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-exit-5-st",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete");

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

        mock_poll(&mut activities, 0, 0, "Do you want to proceed? [y/N]");

        let out = run_to_completion(
            &workflow,
            "test-exit-5-q",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete");

        assert_eq!(out.exit, 5);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_single_worker_workflow_exit_6_malformed_report() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-bad-report", 1800);
        let report_path = PathBuf::from("runs/worker-bad-report.md");
        let initial_state = SingleWorkerState::new(input);

        mock_poll(&mut activities, 0, 1000, "working");
        // Malformed report: completely missing ## Lessons heading
        activities.register(
            "read_report_0",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Report\nEverything completed without lessons.\n".to_string(),
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-exit-6",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete");

        assert_eq!(out.exit, 6);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_single_worker_workflow_grace_extension() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        // Budget is 80s with 20s poll interval
        let input = sample_input("worker-grace", 80);
        let report_path = PathBuf::from("runs/worker-grace.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 20;
        config.hard_cap_sec = 200;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Poll 0 (t=0s), Poll 1 (t=20s), Poll 2 (t=40s), Poll 3 (t=60s)
        mock_poll(&mut activities, 0, 0, "step 0");
        mock_poll(&mut activities, 1, 0, "step 1");
        mock_poll(&mut activities, 2, 0, "step 2");
        mock_poll(&mut activities, 3, 0, "step 3"); // Progress at 60s, within 2 polls of 80s!

        // Poll 4 (t=80s): reaches budget deadline, but grace (+20s = 80s/4) extends deadline to 100s!
        // During grace, report appears at Poll 4:
        mock_poll(&mut activities, 4, 1000, "step 4");
        activities.register(
            "read_report_4",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Report\nLanded during grace!\n\n## Lessons\n- none\n".to_string(),
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-grace",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete successfully during grace");

        assert_eq!(out.exit, 0);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_single_worker_workflow_autoext_bounded() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        // Budget is 40s (poll 20s). Grace +25% = 10s, autoext max +50% = 20s.
        let input = sample_input("worker-autoext", 40);
        let report_path = PathBuf::from("runs/worker-autoext.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 20;
        config.hard_cap_sec = 200;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Progress continues to advance with new git commits on polls
        mock_poll(&mut activities, 0, 0, "output A");
        activities.register(
            "get_sha_0",
            ActivityResult::Git(GitResult::Sha("sha1".to_string())),
        );

        mock_poll(&mut activities, 1, 0, "output B");
        activities.register(
            "get_sha_1",
            ActivityResult::Git(GitResult::Sha("sha2".to_string())),
        );

        // Poll 2 (t=40s): budget reached -> grace (+10s -> 50s) + autoext 1 on sha3 (+10s -> 60s)
        mock_poll(&mut activities, 2, 0, "output C");
        activities.register(
            "get_sha_2",
            ActivityResult::Git(GitResult::Sha("sha3".to_string())),
        );

        // Poll 3 (t=60s): autoext 2 on sha4 (+10s -> 70s, max 20s reached)
        mock_poll(&mut activities, 3, 0, "output D");
        activities.register(
            "get_sha_3",
            ActivityResult::Git(GitResult::Sha("sha4".to_string())),
        );

        // Poll 4 (t=80s): past 70s, max autoext reached -> exits 4!
        mock_poll(&mut activities, 4, 0, "output E");
        activities.register(
            "get_sha_4",
            ActivityResult::Git(GitResult::Sha("sha5".to_string())),
        );

        let out = run_to_completion(
            &workflow,
            "test-autoext",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 4 after autoext bounds");

        assert_eq!(out.exit, 4);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_single_worker_workflow_autoext_cap_clamped() {
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        // Budget is 80s with 20s poll, but hard cap is 90s!
        let input = sample_input("worker-clamped", 80);
        let report_path = PathBuf::from("runs/worker-clamped.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 20;
        config.hard_cap_sec = 90; // Only allows +10s max extension
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        mock_poll(&mut activities, 0, 0, "diff 0");
        mock_poll(&mut activities, 1, 0, "diff 1");
        mock_poll(&mut activities, 2, 0, "diff 2");
        mock_poll(&mut activities, 3, 0, "diff 3");
        // Poll 4 (t=80s): budget reached, grace clamped to 10s (deadline 90s)
        mock_poll(&mut activities, 4, 0, "diff 4");
        // Poll 5 (t=100s): now >= 90s hard cap -> exit 2!
        mock_poll(&mut activities, 5, 0, "diff 5");

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

            mock_poll(&mut activities, 0, 0, "step 1");
            mock_poll(&mut activities, 1, 1000, "step 2");
            activities.register(
                "read_report_1",
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

    // ------------------------------------------------------------------------
    // Tests specifically addressing the 9 review findings
    // ------------------------------------------------------------------------

    #[test]
    fn test_report_not_ready_when_empty_content_waits_for_completion() {
        // Finding 1: An empty report file must not exit 6; workflow keeps polling until content arrives
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-empty-first", 1800);
        let report_path = PathBuf::from("runs/worker-empty-first.md");
        let initial_state = SingleWorkerState::new(input);

        // Poll 0: report mtime > 0, but content is empty!
        mock_poll(&mut activities, 0, 1000, "writing report...");
        activities.register(
            "read_report_0",
            ActivityResult::ReportFile(ReportFileResult::Content("   \n".to_string())),
        );

        // Poll 1: report now has valid non-empty content
        mock_poll(&mut activities, 1, 1000, "done");
        activities.register(
            "read_report_1",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Report\nReady!\n\n## Lessons\n- none\n".to_string(),
            )),
        );

        let out = run_to_completion(
            &workflow,
            "test-empty-wait",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 0 after waiting for non-empty content");

        assert_eq!(out.exit, 0);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_pane_churn_does_not_autoextend_without_git_sha() {
        // Finding 2: Pane churn (spinners, clock) alone does not extend budget past grace
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        // 40s budget, 20s poll. Grace is +10s (50s).
        let input = sample_input("worker-churn", 40);
        let report_path = PathBuf::from("runs/worker-churn.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 20;
        config.hard_cap_sec = 200;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Pane text is changing on every poll (pane churn), but Git SHA remains the same!
        mock_poll(&mut activities, 0, 0, "spinner [|]");
        activities.register(
            "get_sha_0",
            ActivityResult::Git(GitResult::Sha("same-sha".to_string())),
        );

        mock_poll(&mut activities, 1, 0, "spinner [/]");
        activities.register(
            "get_sha_1",
            ActivityResult::Git(GitResult::Sha("same-sha".to_string())),
        );

        mock_poll(&mut activities, 2, 0, "spinner [-]");
        activities.register(
            "get_sha_2",
            ActivityResult::Git(GitResult::Sha("same-sha".to_string())),
        );

        // Poll 3 (t=60s): Grace expired at 50s; autoext was NOT granted despite pane churn -> exits 4!
        mock_poll(&mut activities, 3, 0, "spinner [\\]");
        activities.register(
            "get_sha_3",
            ActivityResult::Git(GitResult::Sha("same-sha".to_string())),
        );

        let out = run_to_completion(
            &workflow,
            "test-churn-exit-4",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 4 without autoext");

        assert_eq!(out.exit, 4);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_status_from_herdr_events_poll() {
        // Finding 3: Agent status received via HerdrEventRequest::PollEvents
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        let input = sample_input("worker-events", 1800);
        let report_path = PathBuf::from("runs/worker-events.md");
        let initial_state = SingleWorkerState::new(input);

        mock_poll(&mut activities, 0, 0, "working");
        activities.register(
            "poll_events_0",
            ActivityResult::HerdrEvents(HerdrEventResult::Events(vec![
                "{\"status\":\"blocked\"}".to_string()
            ])),
        );

        let out = run_to_completion(
            &workflow,
            "test-events-blocked",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete with exit 5 on event blocked");

        assert_eq!(out.exit, 5);
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_budget_precedence_over_idle_and_stall() {
        // Finding 4: When budget expires while worker is idle/stalled, budget (exit 4) takes precedence
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        // 60s budget, 30s poll, idle threshold 30s
        let input = sample_input("worker-precedence", 60);
        let report_path = PathBuf::from("runs/worker-precedence.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.idle_sec = 30;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        mock_poll(&mut activities, 0, 0, "working");
        // Poll 1 (t=30s): becomes idle
        mock_poll(&mut activities, 1, 0, "working");
        activities.register(
            "poll_events_1",
            ActivityResult::HerdrEvents(HerdrEventResult::Events(vec!["agent_idle".to_string()])),
        );
        // Poll 2 (t=60s): both budget (60s) and idle (30s) are expired!
        // Budget check (exit 4) must take precedence over idle (exit 3)
        mock_poll(&mut activities, 2, 0, "working");

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
        assert_eq!(out.report_path, report_path);
    }

    #[test]
    fn test_nudge_prompt_activity_scheduled_at_80_percent() {
        // Finding 5: Nudge prompt activity is scheduled at 80% of budget
        let workflow = SingleWorkerWorkflow::new();
        let mut clock = VirtualClock::new(100_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        // 100s budget with 20s poll -> 80% is 80s (poll 4)
        let input = sample_input("worker-nudge", 100);
        let report_path = PathBuf::from("runs/worker-nudge.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 20;
        config.nudge_pct = 80;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        mock_poll(&mut activities, 0, 0, "step 0");
        mock_poll(&mut activities, 1, 0, "step 1");
        mock_poll(&mut activities, 2, 0, "step 2");
        mock_poll(&mut activities, 3, 0, "step 3");
        // Poll 4 (t=80s): nudge threshold reached!
        mock_poll(&mut activities, 4, 1000, "step 4");
        activities.register(
            "nudge_prompt_4",
            ActivityResult::Herdr(HerdrResult::Prompted {
                name: "worker-nudge".to_string(),
            }),
        );
        activities.register(
            "read_report_4",
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
        assert_eq!(out.report_path, report_path);

        // Verify nudge prompt was actually scheduled and executed in journal
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
        // Finding 6: A builder brief that mentions "Verdict:" does not trigger review mode
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
        let report_path = PathBuf::from("runs/builder-mentions-verdict.md");
        let initial_state = SingleWorkerState::new(input);

        // Report has ## Lessons (builder report), no Verdict:
        mock_poll(&mut activities, 0, 1000, "working");
        activities.register(
            "read_report_0",
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
        assert_eq!(out.report_path, report_path);
    }
}
