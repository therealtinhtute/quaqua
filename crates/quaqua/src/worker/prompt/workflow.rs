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
    ActivityRequest, ActivityResult, HerdrRequest, HerdrResult, ReportFileRequest, ReportFileResult,
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
    pub last_progress_hash: String,
    pub last_autoext_hash: String,
    pub latest_pane_text: String,
    pub agent_status: String,
    pub idle_since_ms: u64,
    pub gone_since_ms: u64,
    pub grace_used: bool,
    pub autoext_total_ms: u64,
    pub poll_count: u64,
    pub nudged: bool,
    pub initialized: bool,
}

impl SingleWorkerState {
    pub fn new(input: SingleWorkerInput) -> Self {
        let report_path = PathBuf::from(format!("runs/{}.md", input.name));
        let expect_verdict = input.brief.contains("Pipeline phase: REVIEW")
            || input.brief.contains("Verdict: clean")
            || input.brief.contains("Verdict:");
        let is_fallback_idle = input.kind.contains("opencode")
            || input.kind.contains("omp")
            || input.kind == "suhuynh"
            || input.kind == "sude";

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
            last_progress_hash: String::new(),
            last_autoext_hash: String::new(),
            latest_pane_text: String::new(),
            agent_status: "working".to_string(),
            idle_since_ms: 0,
            gone_since_ms: 0,
            grace_used: false,
            autoext_total_ms: 0,
            poll_count: 0,
            nudged: false,
            initialized: false,
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
    ) -> Self {
        self.prev_elapsed_ms = prev_elapsed_sec * 1000;
        self.grace_used = grace_used;
        self.autoext_total_ms = autoext_total_sec * 1000;
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

/// Parses agent status from text output or structured lines.
fn parse_agent_status(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed == "gone"
        || trimmed == "idle"
        || trimmed == "done"
        || trimmed == "blocked"
        || trimmed == "working"
    {
        return trimmed.to_string();
    }
    for line in content.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("status:") {
            return rest.trim().to_string();
        }
        if let Some(rest) = l.strip_prefix("agent_status:") {
            return rest.trim().to_string();
        }
    }
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(s) = val.get("agent_status").and_then(|v| v.as_str()) {
                return s.to_string();
            }
            if let Some(s) = val.get("status").and_then(|v| v.as_str()) {
                return s.to_string();
            }
        }
    }
    "working".to_string()
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
            state.last_progress_ms = state.start_ms;

            // Schedule initial observation activities and poll timer
            ctx.schedule_activity(
                format!("check_report_{}", state.poll_count),
                ActivityRequest::ReportFile(ReportFileRequest::Exists {
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
            ctx.schedule_timer("poll_timer", Duration::from_secs(state.config.poll_sec));
            return StepDecision::Continue;
        }

        match event {
            None => StepDecision::Continue,

            Some(WorkflowEvent::ActivityCompleted { id, result }) => {
                if id.starts_with("check_report_") {
                    match result {
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
                        ActivityResult::ReportFile(ReportFileResult::MtimeMs(mtime)) => {
                            if mtime > 0 {
                                ctx.schedule_activity(
                                    format!("read_report_{}", state.poll_count),
                                    ActivityRequest::ReportFile(ReportFileRequest::Read {
                                        path: state.report_path.to_string_lossy().to_string(),
                                    }),
                                );
                            }
                        }
                        ActivityResult::ReportFile(ReportFileResult::Content(content)) => {
                            let valid =
                                validate_report_content(&content, state.config.expect_verdict);
                            let exit = if valid.is_ok() { 0 } else { 6 };
                            return StepDecision::Complete(
                                SingleWorkerOutput::new(exit, state.report_path.clone())
                                    .expect("valid single worker exit code"),
                            );
                        }
                        _ => {}
                    }
                    StepDecision::Continue
                } else if id.starts_with("read_report_") {
                    if let ActivityResult::ReportFile(ReportFileResult::Content(content)) = result {
                        let valid = validate_report_content(&content, state.config.expect_verdict);
                        let exit = if valid.is_ok() { 0 } else { 6 };
                        return StepDecision::Complete(
                            SingleWorkerOutput::new(exit, state.report_path.clone())
                                .expect("valid single worker exit code"),
                        );
                    }
                    StepDecision::Continue
                } else if id.starts_with("read_agent_") {
                    if let ActivityResult::Herdr(HerdrResult::AgentOutput { content }) = result {
                        state.latest_pane_text = content.clone();

                        // Immediate check: question / interactive prompt pending
                        if question_seen(&content) {
                            return StepDecision::Complete(
                                SingleWorkerOutput::new(5, state.report_path.clone())
                                    .expect("valid single worker exit code"),
                            );
                        }

                        // Immediate check: status blocked
                        let st = parse_agent_status(&content);
                        state.agent_status = st.clone();
                        if st == "blocked" {
                            return StepDecision::Complete(
                                SingleWorkerOutput::new(5, state.report_path.clone())
                                    .expect("valid single worker exit code"),
                            );
                        }

                        // Progress hash update
                        let h = hash_pane_content(&content);
                        if !h.is_empty() {
                            if state.last_autoext_hash.is_empty() {
                                state.last_autoext_hash = h.clone();
                            }
                            if h != state.last_progress_hash {
                                state.last_progress_hash = h;
                                state.last_progress_ms = ctx.now_ms();
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

                    // 3. Agent idle check (exit 3)
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

                    // 4. Stall check (exit 3)
                    if now.saturating_sub(state.last_progress_ms) >= state.config.stall_sec * 1000 {
                        return StepDecision::Complete(
                            SingleWorkerOutput::new(3, state.report_path.clone())
                                .expect("valid single worker exit code"),
                        );
                    }

                    // 5. Budget deadline check (exit 4, with grace and autoext)
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

                        // Auto-extensions: +25% steps up to +50% total while progress signal advances
                        let max_autoext_ms = (state.input.budget_sec / 2) * 1000;
                        if state.autoext_total_ms < max_autoext_ms
                            && !state.last_progress_hash.is_empty()
                            && state.last_progress_hash != state.last_autoext_hash
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
                                state.last_autoext_hash = state.last_progress_hash.clone();
                            }
                        }

                        if now >= state.budget_deadline_ms {
                            return StepDecision::Complete(
                                SingleWorkerOutput::new(4, state.report_path.clone())
                                    .expect("valid single worker exit code"),
                            );
                        }
                    }

                    // 6. Nudge check (non-terminal)
                    let nudge_threshold_ms = state.start_ms
                        + (state.input.budget_sec * state.config.nudge_pct / 100) * 1000;
                    if !state.nudged && now >= nudge_threshold_ms {
                        state.nudged = true;
                    }

                    // 7. Schedule next observation activities and timer
                    ctx.schedule_activity(
                        format!("check_report_{}", state.poll_count),
                        ActivityRequest::ReportFile(ReportFileRequest::Exists {
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
        report_exists: bool,
        agent_content: &str,
    ) {
        activities.register(
            format!("check_report_{poll}"),
            ActivityResult::ReportFile(ReportFileResult::Exists(report_exists)),
        );
        activities.register(
            format!("read_agent_{poll}"),
            ActivityResult::Herdr(HerdrResult::AgentOutput {
                content: agent_content.to_string(),
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

        // Poll 0: report not ready yet, worker is working
        mock_poll(&mut activities, 0, false, "Compiling crates/quaqua...");
        // Poll 1: report exists!
        mock_poll(&mut activities, 1, true, "Finished compiling");
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
        mock_poll(&mut activities, 0, false, "step 0");
        mock_poll(&mut activities, 1, false, "step 1");
        mock_poll(&mut activities, 2, false, "step 2");
        mock_poll(&mut activities, 3, false, "step 3");

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
        mock_poll(&mut activities, 0, false, "frozen output");
        mock_poll(&mut activities, 1, false, "frozen output");
        mock_poll(&mut activities, 2, false, "frozen output");

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

        mock_poll(&mut activities, 0, false, "status: working");
        mock_poll(&mut activities, 1, false, "status: idle");
        mock_poll(&mut activities, 2, false, "status: idle");
        mock_poll(&mut activities, 3, false, "status: idle");

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

        mock_poll(&mut activities, 0, false, "status: working");
        mock_poll(&mut activities, 1, false, "status: idle");
        mock_poll(&mut activities, 2, false, "status: idle"); // 30s idle - not reached 90s yet
        mock_poll(&mut activities, 3, false, "status: idle"); // 60s idle - not reached 90s yet
        mock_poll(&mut activities, 4, false, "status: idle"); // 90s idle - reached!

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

        mock_poll(&mut activities, 0, false, "status: working");
        mock_poll(&mut activities, 1, false, "status: gone");
        mock_poll(&mut activities, 2, false, "status: gone");
        mock_poll(&mut activities, 3, false, "status: gone");

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

        // 60s budget with 30s poll. No progress after start -> grace is not eligible
        let input = sample_input("worker-budget", 60);
        let report_path = PathBuf::from("runs/worker-budget.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 30;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Same output -> no progress hash update -> at 60s deadline exits 4
        mock_poll(&mut activities, 0, false, "no progress");
        mock_poll(&mut activities, 1, false, "no progress");
        mock_poll(&mut activities, 2, false, "no progress");

        let out = run_to_completion(
            &workflow,
            "test-exit-4",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        )
        .expect("workflow should complete");

        assert_eq!(out.exit, 4);
        assert_eq!(out.report_path, report_path);
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

        mock_poll(&mut activities, 0, false, "status: blocked");

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

        mock_poll(&mut activities, 0, false, "Do you want to proceed? [y/N]");

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

        mock_poll(&mut activities, 0, true, "working");
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
        mock_poll(&mut activities, 0, false, "step 0");
        mock_poll(&mut activities, 1, false, "step 1");
        mock_poll(&mut activities, 2, false, "step 2");
        mock_poll(&mut activities, 3, false, "step 3"); // Progress at 60s, which is within 2 polls (40s) of 80s!

        // Poll 4 (t=80s): reaches budget deadline, but grace (+20s = 80s/4) extends deadline to 100s!
        // During grace, report appears at Poll 4:
        mock_poll(&mut activities, 4, true, "step 4");
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
        // Cap is 200s so cap does not clamp.
        let input = sample_input("worker-autoext", 40);
        let report_path = PathBuf::from("runs/worker-autoext.md");
        let mut config = WatchdogConfig::default();
        config.poll_sec = 20;
        config.hard_cap_sec = 200;
        config.stall_sec = 600;

        let initial_state = SingleWorkerState::new(input).with_config(config);

        // Progress continues to advance on every poll
        mock_poll(&mut activities, 0, false, "diff A");
        mock_poll(&mut activities, 1, false, "diff B");
        mock_poll(&mut activities, 2, false, "diff C"); // t=40s: budget reached -> grace (+10s -> 50s) + autoext 1 (+10s -> 60s)
        mock_poll(&mut activities, 3, false, "diff D"); // t=60s: autoext 2 (+10s -> 70s, max 20s reached)
        mock_poll(&mut activities, 4, false, "diff E"); // t=80s: past 70s, max autoext reached -> exits 4!

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

        mock_poll(&mut activities, 0, false, "diff 0");
        mock_poll(&mut activities, 1, false, "diff 1");
        mock_poll(&mut activities, 2, false, "diff 2");
        mock_poll(&mut activities, 3, false, "diff 3");
        mock_poll(&mut activities, 4, false, "diff 4"); // t=80s: budget reaches 80s, grace clamped to 10s (deadline 90s)
        mock_poll(&mut activities, 5, false, "diff 5"); // t=100s: now >= 90s hard cap -> exit 2!

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

            mock_poll(&mut activities, 0, false, "step 1");
            mock_poll(&mut activities, 1, true, "step 2");
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
}
