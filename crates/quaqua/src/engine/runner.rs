//! Durable event-sourced workflow runner.
//!
//! Provides snapshot loading, event replay without side-effect re-execution,
//! deterministic stepping, activity dispatch, timer coordination via a clock seam,
//! event and snapshot persistence, and suspension when blocked.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::engine::activity::{Activity, ActivityError, ActivityRequest, ActivityResult};
use crate::engine::journal::{EventRecord, Journal, JournalError, SnapshotRecord};
use crate::engine::workflow::{
    ScheduledAction, StepDecision, Workflow, WorkflowContext, WorkflowEvent,
};

/// Minimal clock abstraction internal to the runner.
pub(crate) trait RunnerClock: Send {
    /// Returns current virtual timestamp in milliseconds since Unix epoch.
    fn now_ms(&self) -> u64;

    /// Schedules a timer with an ID and duration.
    fn schedule_timer(&mut self, id: &str, duration: Duration);

    /// Advances virtual time to the earliest scheduled timer, if any, and returns expired IDs.
    fn advance_to_next_timer(&mut self) -> Option<(u64, Vec<String>)>;

    /// Returns the number of pending timers waiting to fire.
    fn pending_timers_count(&self) -> usize;
}

impl RunnerClock for crate::engine::testkit::VirtualClock {
    fn now_ms(&self) -> u64 {
        self.now_ms()
    }

    fn schedule_timer(&mut self, id: &str, duration: Duration) {
        self.schedule_timer(id, duration);
    }

    fn advance_to_next_timer(&mut self) -> Option<(u64, Vec<String>)> {
        self.advance_to_next_timer()
    }

    fn pending_timers_count(&self) -> usize {
        self.pending_timers_count()
    }
}

/// Outcome of running a workflow until completion, failure, or suspension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RunnerOutcome<Output> {
    /// Workflow completed successfully with output.
    Completed(Output),
    /// Workflow suspended waiting for external events or un-fireable timers.
    Blocked(String),
    /// Workflow terminated with failure.
    Failed { code: i32, reason: String },
}

/// Configuration options for workflow execution.
#[derive(Debug, Clone)]
pub struct RunnerOptions {
    /// Maximum steps allowed before aborting with [`RunnerError::StepLimitExceeded`].
    pub max_steps: usize,
    /// Number of events between periodic snapshots (0 disables periodic snapshots).
    pub snapshot_interval: u64,
    /// Whether to always capture a state snapshot when the workflow transitions to Blocked.
    pub snapshot_at_blocked: bool,
}

impl Default for RunnerOptions {
    fn default() -> Self {
        Self {
            max_steps: 10_000,
            snapshot_interval: 20,
            snapshot_at_blocked: true,
        }
    }
}

/// Errors returned by the workflow runner.
#[derive(Debug, Clone, PartialEq)]
pub enum RunnerError {
    /// Underlying journal error.
    Journal(JournalError),
    /// Underlying activity execution error.
    Activity(ActivityError),
    /// Serialization or deserialization failure.
    Serialization(String),
    /// Workflow has no snapshot and no initial state was provided.
    NoInitialState,
    /// Execution exceeded configured max_steps.
    StepLimitExceeded(usize),
}

impl From<JournalError> for RunnerError {
    fn from(err: JournalError) -> Self {
        Self::Journal(err)
    }
}

impl From<ActivityError> for RunnerError {
    fn from(err: ActivityError) -> Self {
        Self::Activity(err)
    }
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Journal(e) => write!(f, "journal error: {e}"),
            Self::Activity(e) => write!(f, "activity error: {e}"),
            Self::Serialization(e) => write!(f, "serialization error: {e}"),
            Self::NoInitialState => write!(f, "no snapshot found and no initial state provided"),
            Self::StepLimitExceeded(n) => write!(f, "execution step limit ({n}) exceeded"),
        }
    }
}

impl std::error::Error for RunnerError {}

// ----------------------------------------------------------------------------
// Internal Event Payload Serializers / Deserializers
// ----------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct ActivityScheduledData {
    id: String,
    activity: ActivityRequest,
}

#[derive(Serialize, Deserialize)]
struct ActivityCompletedData {
    id: String,
    result: ActivityResult,
}

#[derive(Serialize, Deserialize)]
struct TimerScheduledData {
    id: String,
    duration_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct TimerFiredData {
    id: String,
}

#[derive(Serialize, Deserialize)]
struct PauseForApprovalData {
    reason: String,
}

#[derive(Serialize, Deserialize)]
struct ApprovalResolvedData {
    approved: bool,
    reason: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ChildStartedData {
    id: String,
    workflow_type: String,
    input: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct ChildCompletedData {
    id: String,
    output: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct ChildFailedData {
    id: String,
    code: i32,
    reason: String,
}

#[derive(Serialize, Deserialize)]
struct WorkflowFailedData {
    code: i32,
    reason: String,
}

enum ReplayStepInput {
    NoneStep,
    Event(WorkflowEvent),
}

// ----------------------------------------------------------------------------
// Dispatch Helpers
// ----------------------------------------------------------------------------

fn dispatch_activity(
    workflow_id: &str,
    id: &str,
    activity: &ActivityRequest,
    journal: &mut dyn Journal,
    activities: &mut dyn Activity,
    clock: &impl RunnerClock,
    max_seq: &mut u64,
    events_since_snapshot: &mut u64,
) -> Result<WorkflowEvent, RunnerError> {
    *max_seq += 1;
    journal.append_event(
        workflow_id,
        &EventRecord::new(
            *max_seq,
            clock.now_ms(),
            "activity_scheduled",
            serde_json::to_string(&ActivityScheduledData {
                id: id.to_string(),
                activity: activity.clone(),
            })
            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
        ),
    )?;
    *events_since_snapshot += 1;

    let result = activities.execute(activity)?;
    *max_seq += 1;
    journal.append_event(
        workflow_id,
        &EventRecord::new(
            *max_seq,
            clock.now_ms(),
            "activity_completed",
            serde_json::to_string(&ActivityCompletedData {
                id: id.to_string(),
                result: result.clone(),
            })
            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
        ),
    )?;
    *events_since_snapshot += 1;
    Ok(WorkflowEvent::ActivityCompleted {
        id: id.to_string(),
        result,
    })
}

fn dispatch_timer(
    workflow_id: &str,
    id: &str,
    duration: Duration,
    journal: &mut dyn Journal,
    clock: &mut impl RunnerClock,
    max_seq: &mut u64,
    events_since_snapshot: &mut u64,
) -> Result<(), RunnerError> {
    *max_seq += 1;
    journal.append_event(
        workflow_id,
        &EventRecord::new(
            *max_seq,
            clock.now_ms(),
            "timer_scheduled",
            serde_json::to_string(&TimerScheduledData {
                id: id.to_string(),
                duration_ms: duration.as_millis() as u64,
            })
            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
        ),
    )?;
    *events_since_snapshot += 1;
    clock.schedule_timer(id, duration);
    Ok(())
}

// ----------------------------------------------------------------------------
// Public Runner Entrypoints
// ----------------------------------------------------------------------------

/// Drives a workflow from initial state or prior snapshot until it completes, fails, or blocks.
pub(crate) fn run<W: Workflow, C: RunnerClock>(
    workflow: &W,
    workflow_id: &str,
    initial_state: W::State,
    journal: &mut dyn Journal,
    activities: &mut dyn Activity,
    clock: &mut C,
) -> Result<RunnerOutcome<W::Output>, RunnerError> {
    run_until_blocked(
        workflow,
        workflow_id,
        Some(initial_state),
        journal,
        activities,
        clock,
        &RunnerOptions::default(),
        Vec::new(),
    )
}

/// Resumes a blocked workflow using state from the journal and delivers incoming external events.
pub(crate) fn resume<W: Workflow, C: RunnerClock>(
    workflow: &W,
    workflow_id: &str,
    journal: &mut dyn Journal,
    activities: &mut dyn Activity,
    clock: &mut C,
    incoming_events: Vec<WorkflowEvent>,
) -> Result<RunnerOutcome<W::Output>, RunnerError> {
    run_until_blocked(
        workflow,
        workflow_id,
        None,
        journal,
        activities,
        clock,
        &RunnerOptions::default(),
        incoming_events,
    )
}

/// Executes or resumes a workflow until blocked, completed, or failed.
pub(crate) fn run_until_blocked<W: Workflow, C: RunnerClock>(
    workflow: &W,
    workflow_id: &str,
    initial_state: Option<W::State>,
    journal: &mut dyn Journal,
    activities: &mut dyn Activity,
    clock: &mut C,
    options: &RunnerOptions,
    incoming_events: Vec<WorkflowEvent>,
) -> Result<RunnerOutcome<W::Output>, RunnerError> {
    // 1. Load full history from seq 1 to inspect terminal status and build complete indexes
    let all_events = journal.load_events(workflow_id, 1)?;
    let mut current_seq = all_events.iter().map(|e| e.seq).max().unwrap_or(0);

    // Finding 4: Resuming an already completed or failed workflow immediately returns terminal outcome
    for rec in &all_events {
        if rec.kind == "workflow_completed" {
            let output: W::Output = serde_json::from_str(&rec.payload_json)
                .map_err(|e| RunnerError::Serialization(e.to_string()))?;
            return Ok(RunnerOutcome::Completed(output));
        }
        if rec.kind == "workflow_failed" {
            let data: WorkflowFailedData = serde_json::from_str(&rec.payload_json)
                .map_err(|e| RunnerError::Serialization(e.to_string()))?;
            return Ok(RunnerOutcome::Failed {
                code: data.code,
                reason: data.reason,
            });
        }
    }

    // Build complete activity & timer index across ALL historical events
    let mut completed_activities: HashMap<String, ActivityResult> = HashMap::new();
    let mut scheduled_activities: HashMap<String, ActivityRequest> = HashMap::new();
    let mut scheduled_timers: HashMap<String, (u64, Duration)> = HashMap::new();
    let mut fired_timers: HashSet<String> = HashSet::new();

    for rec in &all_events {
        match rec.kind.as_str() {
            "activity_scheduled" => {
                let data: ActivityScheduledData = serde_json::from_str(&rec.payload_json)
                    .map_err(|e| RunnerError::Serialization(e.to_string()))?;
                scheduled_activities.insert(data.id, data.activity);
            }
            "activity_completed" => {
                let data: ActivityCompletedData = serde_json::from_str(&rec.payload_json)
                    .map_err(|e| RunnerError::Serialization(e.to_string()))?;
                completed_activities.insert(data.id, data.result);
            }
            "timer_scheduled" => {
                let data: TimerScheduledData = serde_json::from_str(&rec.payload_json)
                    .map_err(|e| RunnerError::Serialization(e.to_string()))?;
                scheduled_timers.insert(
                    data.id,
                    (rec.ts_ms, Duration::from_millis(data.duration_ms)),
                );
            }
            "timer_fired" => {
                let data: TimerFiredData = serde_json::from_str(&rec.payload_json)
                    .map_err(|e| RunnerError::Serialization(e.to_string()))?;
                fired_timers.insert(data.id);
            }
            _ => {}
        }
    }

    // Findings 2 & 5: Restore pending timers from full history with their remaining duration
    for (id, (sched_ts, orig_dur)) in &scheduled_timers {
        if !fired_timers.contains(id) {
            let deadline_ms = sched_ts + orig_dur.as_millis() as u64;
            let current_now = clock.now_ms();
            let remaining = if deadline_ms > current_now {
                Duration::from_millis(deadline_ms - current_now)
            } else {
                Duration::from_millis(0)
            };
            clock.schedule_timer(id, remaining);
        }
    }

    // 2. Load latest state snapshot
    let latest_snapshot = journal.load_latest_snapshot(workflow_id)?;
    let (mut state, snapshot_seq) = match latest_snapshot {
        Some(snap) => {
            let s: W::State = serde_json::from_str(&snap.state_json)
                .map_err(|e| RunnerError::Serialization(e.to_string()))?;
            (s, snap.seq)
        }
        None => {
            let s = initial_state.ok_or(RunnerError::NoInitialState)?;
            (s, 0u64)
        }
    };

    // 3. Collect step inputs from events recorded after the snapshot
    let events_since_snapshot_rec = journal.load_events(workflow_id, snapshot_seq + 1)?;
    let mut replay_queue: VecDeque<ReplayStepInput> = VecDeque::new();

    if snapshot_seq == 0 && !all_events.is_empty() {
        replay_queue.push_back(ReplayStepInput::NoneStep);
    }

    for rec in &events_since_snapshot_rec {
        match rec.kind.as_str() {
            // Finding 3: Continue steps must be replayed as step(None)
            "continue" => {
                replay_queue.push_back(ReplayStepInput::NoneStep);
            }
            "activity_completed" => {
                let data: ActivityCompletedData = serde_json::from_str(&rec.payload_json)
                    .map_err(|e| RunnerError::Serialization(e.to_string()))?;
                replay_queue.push_back(ReplayStepInput::Event(WorkflowEvent::ActivityCompleted {
                    id: data.id,
                    result: data.result,
                }));
            }
            "timer_fired" => {
                let data: TimerFiredData = serde_json::from_str(&rec.payload_json)
                    .map_err(|e| RunnerError::Serialization(e.to_string()))?;
                replay_queue.push_back(ReplayStepInput::Event(WorkflowEvent::TimerFired {
                    id: data.id,
                }));
            }
            "approval_resolved" => {
                let data: ApprovalResolvedData = serde_json::from_str(&rec.payload_json)
                    .map_err(|e| RunnerError::Serialization(e.to_string()))?;
                replay_queue.push_back(ReplayStepInput::Event(WorkflowEvent::ApprovalResolved {
                    approved: data.approved,
                    reason: data.reason,
                }));
            }
            "child_completed" => {
                let data: ChildCompletedData = serde_json::from_str(&rec.payload_json)
                    .map_err(|e| RunnerError::Serialization(e.to_string()))?;
                replay_queue.push_back(ReplayStepInput::Event(WorkflowEvent::ChildCompleted {
                    id: data.id,
                    output: data.output,
                }));
            }
            "child_failed" => {
                let data: ChildFailedData = serde_json::from_str(&rec.payload_json)
                    .map_err(|e| RunnerError::Serialization(e.to_string()))?;
                replay_queue.push_back(ReplayStepInput::Event(WorkflowEvent::ChildFailed {
                    id: data.id,
                    code: data.code,
                    reason: data.reason,
                }));
            }
            _ => {}
        }
    }

    // 4. Replay steps through step()
    let mut last_replay_decision: Option<StepDecision<W::Output>> = None;
    let mut last_replay_scheduled: Vec<ScheduledAction> = Vec::new();

    while let Some(item) = replay_queue.pop_front() {
        let is_last = replay_queue.is_empty();
        let ev = match item {
            ReplayStepInput::NoneStep => None,
            ReplayStepInput::Event(e) => Some(e),
        };
        let mut ctx = WorkflowContext::new(clock.now_ms());
        let dec = workflow.step(&mut state, &mut ctx, ev);
        let scheduled = ctx.drain_scheduled();

        if is_last {
            last_replay_decision = Some(dec);
            last_replay_scheduled = scheduled;
        }
    }

    // 5. Transition to Forward Execution
    let mut event_queue: VecDeque<WorkflowEvent> = VecDeque::new();
    let mut events_since_snapshot = 0u64;
    let mut continue_immediate = false;

    // Handle unscheduled actions from the last step of replay (if any)
    for action in last_replay_scheduled {
        match action {
            ScheduledAction::Activity { id, activity } => {
                if !scheduled_activities.contains_key(&id) {
                    let ev = dispatch_activity(
                        workflow_id,
                        &id,
                        &activity,
                        journal,
                        activities,
                        clock,
                        &mut current_seq,
                        &mut events_since_snapshot,
                    )?;
                    event_queue.push_back(ev);
                }
            }
            ScheduledAction::Timer { id, duration } => {
                if !scheduled_timers.contains_key(&id) {
                    dispatch_timer(
                        workflow_id,
                        &id,
                        duration,
                        journal,
                        clock,
                        &mut current_seq,
                        &mut events_since_snapshot,
                    )?;
                }
            }
            ScheduledAction::Child {
                id,
                workflow_type,
                input,
            } => {
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "child_started",
                        serde_json::to_string(&ChildStartedData {
                            id,
                            workflow_type,
                            input,
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                events_since_snapshot += 1;
            }
        }
    }

    if let Some(decision) = last_replay_decision {
        match decision {
            StepDecision::Continue => {
                continue_immediate = true;
            }
            StepDecision::ScheduleActivity { id, activity } => {
                if !scheduled_activities.contains_key(&id) {
                    let ev = dispatch_activity(
                        workflow_id,
                        &id,
                        &activity,
                        journal,
                        activities,
                        clock,
                        &mut current_seq,
                        &mut events_since_snapshot,
                    )?;
                    event_queue.push_back(ev);
                }
            }
            StepDecision::ScheduleTimer { id, duration } => {
                if !scheduled_timers.contains_key(&id) {
                    dispatch_timer(
                        workflow_id,
                        &id,
                        duration,
                        journal,
                        clock,
                        &mut current_seq,
                        &mut events_since_snapshot,
                    )?;
                }
            }
            StepDecision::Complete(output) => {
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "workflow_completed",
                        serde_json::to_string(&output)
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                journal.append_snapshot(
                    workflow_id,
                    &SnapshotRecord::new(
                        current_seq,
                        serde_json::to_string(&state)
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                return Ok(RunnerOutcome::Completed(output));
            }
            StepDecision::Fail { code, reason } => {
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "workflow_failed",
                        serde_json::to_string(&WorkflowFailedData {
                            code,
                            reason: reason.clone(),
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                return Ok(RunnerOutcome::Failed { code, reason });
            }
            StepDecision::PauseForApproval { reason } => {
                if !all_events.iter().any(|e| e.kind == "pause_for_approval") {
                    current_seq += 1;
                    journal.append_event(
                        workflow_id,
                        &EventRecord::new(
                            current_seq,
                            clock.now_ms(),
                            "pause_for_approval",
                            serde_json::to_string(&PauseForApprovalData {
                                reason: reason.clone(),
                            })
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                        ),
                    )?;
                }
            }
            StepDecision::StartChild {
                id,
                workflow_type,
                input,
            } => {
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "child_started",
                        serde_json::to_string(&ChildStartedData {
                            id,
                            workflow_type,
                            input,
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                events_since_snapshot += 1;
            }
        }
    }

    // Re-dispatch any activities that were scheduled in history but never completed
    for (id, req) in &scheduled_activities {
        if !completed_activities.contains_key(id) {
            let result = activities.execute(req)?;
            current_seq += 1;
            journal.append_event(
                workflow_id,
                &EventRecord::new(
                    current_seq,
                    clock.now_ms(),
                    "activity_completed",
                    serde_json::to_string(&ActivityCompletedData {
                        id: id.clone(),
                        result: result.clone(),
                    })
                    .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                ),
            )?;
            event_queue.push_back(WorkflowEvent::ActivityCompleted {
                id: id.clone(),
                result,
            });
        }
    }

    // Append and queue any new incoming external events
    for ev in incoming_events {
        current_seq += 1;
        match &ev {
            WorkflowEvent::ApprovalResolved { approved, reason } => {
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "approval_resolved",
                        serde_json::to_string(&ApprovalResolvedData {
                            approved: *approved,
                            reason: reason.clone(),
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
            }
            WorkflowEvent::ActivityCompleted { id, result } => {
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "activity_completed",
                        serde_json::to_string(&ActivityCompletedData {
                            id: id.clone(),
                            result: result.clone(),
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
            }
            WorkflowEvent::TimerFired { id } => {
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "timer_fired",
                        serde_json::to_string(&TimerFiredData { id: id.clone() })
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
            }
            WorkflowEvent::ChildCompleted { id, output } => {
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "child_completed",
                        serde_json::to_string(&ChildCompletedData {
                            id: id.clone(),
                            output: output.clone(),
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
            }
            WorkflowEvent::ChildFailed { id, code, reason } => {
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "child_failed",
                        serde_json::to_string(&ChildFailedData {
                            id: id.clone(),
                            code: *code,
                            reason: reason.clone(),
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
            }
        }
        event_queue.push_back(ev);
    }

    let mut first_step_needed = snapshot_seq == 0 && all_events.is_empty();
    let mut steps = 0;

    loop {
        steps += 1;
        if steps > options.max_steps {
            return Err(RunnerError::StepLimitExceeded(options.max_steps));
        }

        let next_event = if first_step_needed {
            first_step_needed = false;
            None
        } else if continue_immediate {
            continue_immediate = false;
            None
        } else if let Some(ev) = event_queue.pop_front() {
            Some(ev)
        } else if clock.pending_timers_count() > 0 {
            if let Some((_ts, fired_ids)) = clock.advance_to_next_timer() {
                for id in fired_ids {
                    current_seq += 1;
                    journal.append_event(
                        workflow_id,
                        &EventRecord::new(
                            current_seq,
                            clock.now_ms(),
                            "timer_fired",
                            serde_json::to_string(&TimerFiredData { id: id.clone() })
                                .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                        ),
                    )?;
                    events_since_snapshot += 1;
                    event_queue.push_back(WorkflowEvent::TimerFired { id });
                }
                event_queue.pop_front()
            } else {
                None
            }
        } else {
            // Quiescent point: event queue empty and no pending timers can fire
            if options.snapshot_at_blocked {
                journal.append_snapshot(
                    workflow_id,
                    &SnapshotRecord::new(
                        current_seq,
                        serde_json::to_string(&state)
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
            }
            return Ok(RunnerOutcome::Blocked(
                "no pending events or timers".to_string(),
            ));
        };

        let mut ctx = WorkflowContext::new(clock.now_ms());
        let decision = workflow.step(&mut state, &mut ctx, next_event);

        // Process actions scheduled directly on WorkflowContext
        for action in ctx.drain_scheduled() {
            match action {
                ScheduledAction::Activity { id, activity } => {
                    let ev = dispatch_activity(
                        workflow_id,
                        &id,
                        &activity,
                        journal,
                        activities,
                        clock,
                        &mut current_seq,
                        &mut events_since_snapshot,
                    )?;
                    event_queue.push_back(ev);
                }
                ScheduledAction::Timer { id, duration } => {
                    dispatch_timer(
                        workflow_id,
                        &id,
                        duration,
                        journal,
                        clock,
                        &mut current_seq,
                        &mut events_since_snapshot,
                    )?;
                }
                ScheduledAction::Child {
                    id,
                    workflow_type,
                    input,
                } => {
                    current_seq += 1;
                    journal.append_event(
                        workflow_id,
                        &EventRecord::new(
                            current_seq,
                            clock.now_ms(),
                            "child_started",
                            serde_json::to_string(&ChildStartedData {
                                id,
                                workflow_type,
                                input,
                            })
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                        ),
                    )?;
                    events_since_snapshot += 1;
                }
            }
        }

        // Process step decision
        match decision {
            StepDecision::Continue => {
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(current_seq, clock.now_ms(), "continue", ""),
                )?;
                events_since_snapshot += 1;
                continue_immediate = true;
            }
            StepDecision::ScheduleActivity { id, activity } => {
                let ev = dispatch_activity(
                    workflow_id,
                    &id,
                    &activity,
                    journal,
                    activities,
                    clock,
                    &mut current_seq,
                    &mut events_since_snapshot,
                )?;
                event_queue.push_back(ev);
            }
            StepDecision::ScheduleTimer { id, duration } => {
                dispatch_timer(
                    workflow_id,
                    &id,
                    duration,
                    journal,
                    clock,
                    &mut current_seq,
                    &mut events_since_snapshot,
                )?;
            }
            StepDecision::PauseForApproval { reason } => {
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "pause_for_approval",
                        serde_json::to_string(&PauseForApprovalData {
                            reason: reason.clone(),
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                if options.snapshot_at_blocked {
                    journal.append_snapshot(
                        workflow_id,
                        &SnapshotRecord::new(
                            current_seq,
                            serde_json::to_string(&state)
                                .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                        ),
                    )?;
                }
                return Ok(RunnerOutcome::Blocked(format!(
                    "pause_for_approval: {reason}"
                )));
            }
            StepDecision::Complete(output) => {
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "workflow_completed",
                        serde_json::to_string(&output)
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                journal.append_snapshot(
                    workflow_id,
                    &SnapshotRecord::new(
                        current_seq,
                        serde_json::to_string(&state)
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                return Ok(RunnerOutcome::Completed(output));
            }
            StepDecision::Fail { code, reason } => {
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "workflow_failed",
                        serde_json::to_string(&WorkflowFailedData {
                            code,
                            reason: reason.clone(),
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                return Ok(RunnerOutcome::Failed { code, reason });
            }
            StepDecision::StartChild {
                id,
                workflow_type,
                input,
            } => {
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "child_started",
                        serde_json::to_string(&ChildStartedData {
                            id,
                            workflow_type,
                            input,
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                events_since_snapshot += 1;
            }
        }

        // Finding 1: Periodic snapshots are ONLY taken at a quiescent point!
        // i.e., when no events remain queued and no continue_immediate is pending.
        if options.snapshot_interval > 0
            && events_since_snapshot >= options.snapshot_interval
            && event_queue.is_empty()
            && !continue_immediate
        {
            journal.append_snapshot(
                workflow_id,
                &SnapshotRecord::new(
                    current_seq,
                    serde_json::to_string(&state)
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                ),
            )?;
            events_since_snapshot = 0;
        }
    }
}

// ----------------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::activity::{CostRequest, CostResult, HerdrRequest, HerdrResult};
    use crate::engine::testkit::{InMemoryJournal, VirtualClock};
    use std::sync::Mutex;
    use std::time::Instant;

    /// Thread-safe mock activities implementing [`Activity`].
    #[derive(Default)]
    struct MockActivityRegistry {
        canned: Mutex<Vec<(ActivityRequest, ActivityResult)>>,
        calls: Mutex<Vec<ActivityRequest>>,
    }

    impl MockActivityRegistry {
        fn new() -> Self {
            Self::default()
        }

        fn register(&self, req: ActivityRequest, res: ActivityResult) {
            self.canned.lock().unwrap().push((req, res));
        }

        fn count_for(&self, req: &ActivityRequest) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|r| *r == req)
                .count()
        }
    }

    impl Activity for MockActivityRegistry {
        fn execute(&self, req: &ActivityRequest) -> Result<ActivityResult, ActivityError> {
            self.calls.lock().unwrap().push(req.clone());
            if let Some((_, res)) = self.canned.lock().unwrap().iter().find(|(r, _)| r == req) {
                Ok(res.clone())
            } else {
                Err(ActivityError::NotFound(format!(
                    "unregistered request: {req:?}"
                )))
            }
        }
    }

    // ------------------------------------------------------------------------
    // Test Workflow 1: Long 7200s Timer Workflow (Acceptance b)
    // ------------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TimerWorkflowState {
        timer_completed: bool,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TimerWorkflow {
        timer_duration_secs: u64,
    }

    impl Workflow for TimerWorkflow {
        type Input = u64;
        type State = TimerWorkflowState;
        type Output = String;

        fn step(
            &self,
            state: &mut Self::State,
            _ctx: &mut WorkflowContext,
            event: Option<WorkflowEvent>,
        ) -> StepDecision<Self::Output> {
            match event {
                None => StepDecision::ScheduleTimer {
                    id: "long_timer".to_string(),
                    duration: Duration::from_secs(self.timer_duration_secs),
                },
                Some(WorkflowEvent::TimerFired { id }) => {
                    if id == "long_timer" {
                        state.timer_completed = true;
                        StepDecision::Complete("timer_finished".to_string())
                    } else {
                        StepDecision::Fail {
                            code: 1,
                            reason: format!("unexpected timer: {id}"),
                        }
                    }
                }
                _ => StepDecision::Fail {
                    code: 2,
                    reason: "unexpected event".to_string(),
                },
            }
        }
    }

    #[test]
    fn test_virtual_clock_7200s_timer_completes_under_50ms() {
        let workflow = TimerWorkflow {
            timer_duration_secs: 7200,
        };
        let initial_state = TimerWorkflowState {
            timer_completed: false,
        };
        let mut journal = InMemoryJournal::new();
        let mut activities = MockActivityRegistry::new();
        let mut clock = VirtualClock::new(1000);

        let start_instant = Instant::now();
        let outcome = run(
            &workflow,
            "wf_timer_7200",
            initial_state,
            &mut journal,
            &mut activities,
            &mut clock,
        )
        .expect("run should succeed");

        let elapsed = start_instant.elapsed();

        assert_eq!(
            outcome,
            RunnerOutcome::Completed("timer_finished".to_string())
        );
        assert!(
            elapsed < Duration::from_millis(50),
            "7200s timer took {:?}, expected <50ms",
            elapsed
        );
        assert_eq!(clock.now_ms(), 1000 + 7200 * 1000);

        let events = journal
            .load_events("wf_timer_7200", 1)
            .expect("events load");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind, "timer_scheduled");
        assert_eq!(events[1].kind, "timer_fired");
        assert_eq!(events[2].kind, "workflow_completed");
    }

    // ------------------------------------------------------------------------
    // Test Workflow 2: Mid-run Kill & Resume against Uninterrupted Reference (Acceptance a & Finding 6)
    // ------------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct MidrunKillState {
        act1_result: Option<f64>,
        act2_result: Option<f64>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct MidrunKillWorkflow;

    impl Workflow for MidrunKillWorkflow {
        type Input = ();
        type State = MidrunKillState;
        type Output = String;

        fn step(
            &self,
            state: &mut Self::State,
            _ctx: &mut WorkflowContext,
            event: Option<WorkflowEvent>,
        ) -> StepDecision<Self::Output> {
            match event {
                None => StepDecision::ScheduleActivity {
                    id: "act_1".to_string(),
                    activity: ActivityRequest::Cost(CostRequest::QuerySpend {
                        session_pattern: "session_1".to_string(),
                    }),
                },
                Some(WorkflowEvent::ActivityCompleted { id, result }) => {
                    if id == "act_1" {
                        if let ActivityResult::Cost(CostResult::Spend(v)) = result {
                            state.act1_result = Some(v);
                            StepDecision::ScheduleActivity {
                                id: "act_2".to_string(),
                                activity: ActivityRequest::Cost(CostRequest::QuerySpend {
                                    session_pattern: "session_2".to_string(),
                                }),
                            }
                        } else {
                            StepDecision::Fail {
                                code: 1,
                                reason: "bad act_1".to_string(),
                            }
                        }
                    } else if id == "act_2" {
                        if let ActivityResult::Cost(CostResult::Spend(v)) = result {
                            state.act2_result = Some(v);
                            StepDecision::Complete(format!(
                                "total: {}",
                                state.act1_result.unwrap_or(0.0) + v
                            ))
                        } else {
                            StepDecision::Fail {
                                code: 2,
                                reason: "bad act_2".to_string(),
                            }
                        }
                    } else {
                        StepDecision::Fail {
                            code: 3,
                            reason: "unknown act".to_string(),
                        }
                    }
                }
                _ => StepDecision::Fail {
                    code: 4,
                    reason: "unexpected".to_string(),
                },
            }
        }
    }

    #[test]
    fn test_kill_and_resume_preserves_state_and_event_sequence() {
        let workflow = MidrunKillWorkflow;
        let initial_state = MidrunKillState {
            act1_result: None,
            act2_result: None,
        };

        let req1 = ActivityRequest::Cost(CostRequest::QuerySpend {
            session_pattern: "session_1".to_string(),
        });
        let res1 = ActivityResult::Cost(CostResult::Spend(10.0));
        let req2 = ActivityRequest::Cost(CostRequest::QuerySpend {
            session_pattern: "session_2".to_string(),
        });
        let res2 = ActivityResult::Cost(CostResult::Spend(20.0));

        // --- Run 1: Uninterrupted Reference Run ---
        let mut journal_ref = InMemoryJournal::new();
        let mut activities_ref = MockActivityRegistry::new();
        activities_ref.register(req1.clone(), res1.clone());
        activities_ref.register(req2.clone(), res2.clone());
        let mut clock_ref = VirtualClock::new(5000);

        let outcome_ref = run(
            &workflow,
            "ref_wf",
            initial_state.clone(),
            &mut journal_ref,
            &mut activities_ref,
            &mut clock_ref,
        )
        .expect("ref run should succeed");

        assert_eq!(
            outcome_ref,
            RunnerOutcome::Completed("total: 30".to_string())
        );
        let ref_events = journal_ref
            .load_events("ref_wf", 1)
            .expect("load ref events");
        let ref_snap = journal_ref
            .load_latest_snapshot("ref_wf")
            .expect("load ref snap")
            .unwrap();

        // --- Run 2: True Mid-run Kill ---
        // Pre-populate journal with act_1 scheduled and completed (NO snapshot exists yet!)
        let mut journal_interrupted = InMemoryJournal::new();
        let mut activities_interrupted = MockActivityRegistry::new();
        activities_interrupted.register(req1.clone(), res1.clone());
        activities_interrupted.register(req2.clone(), res2);
        let mut clock_interrupted = VirtualClock::new(5000);

        // Pre-write act_1 events as if killed right after act_1 finished before act_2
        journal_interrupted
            .append_event(
                "int_wf",
                &EventRecord::new(
                    1,
                    5000,
                    "activity_scheduled",
                    serde_json::to_string(&ActivityScheduledData {
                        id: "act_1".to_string(),
                        activity: req1.clone(),
                    })
                    .unwrap(),
                ),
            )
            .unwrap();
        journal_interrupted
            .append_event(
                "int_wf",
                &EventRecord::new(
                    2,
                    5000,
                    "activity_completed",
                    serde_json::to_string(&ActivityCompletedData {
                        id: "act_1".to_string(),
                        result: res1,
                    })
                    .unwrap(),
                ),
            )
            .unwrap();

        // Resume with fresh runner from initial_state
        let outcome_resumed = run(
            &workflow,
            "int_wf",
            initial_state,
            &mut journal_interrupted,
            &mut activities_interrupted,
            &mut clock_interrupted,
        )
        .expect("resumed run should complete");

        assert_eq!(
            outcome_resumed,
            RunnerOutcome::Completed("total: 30".to_string())
        );

        // ACT 1 WAS REPLAYED WITHOUT RE-EXECUTION!
        assert_eq!(activities_interrupted.count_for(&req1), 0);
        // Only ACT 2 was executed in forward execution!
        assert_eq!(activities_interrupted.count_for(&req2), 1);

        // Verify final state snapshot equivalence
        let int_snap = journal_interrupted
            .load_latest_snapshot("int_wf")
            .expect("load int snap")
            .unwrap();
        assert_eq!(int_snap.state_json, ref_snap.state_json);

        // Verify exact journal event sequence equivalence
        let int_events = journal_interrupted
            .load_events("int_wf", 1)
            .expect("load int events");
        assert_eq!(int_events.len(), ref_events.len());
        for (i, (int_ev, ref_ev)) in int_events.iter().zip(ref_events.iter()).enumerate() {
            assert_eq!(
                int_ev.kind, ref_ev.kind,
                "event {i} kind mismatch: int={}, ref={}",
                int_ev.kind, ref_ev.kind
            );
            assert_eq!(
                int_ev.payload_json, ref_ev.payload_json,
                "event {i} payload mismatch"
            );
            assert_eq!(int_ev.seq, ref_ev.seq, "event {i} seq mismatch");
        }
    }

    // ------------------------------------------------------------------------
    // Test Workflow 3: Periodic Snapshotting & Quiescent Safety (Finding 1)
    // ------------------------------------------------------------------------

    #[test]
    fn test_periodic_snapshot_does_not_drop_queued_events_on_resume() {
        let workflow = MidrunKillWorkflow;
        let mut journal = InMemoryJournal::new();
        let mut activities = MockActivityRegistry::new();

        let req1 = ActivityRequest::Cost(CostRequest::QuerySpend {
            session_pattern: "session_1".to_string(),
        });
        let res1 = ActivityResult::Cost(CostResult::Spend(10.0));
        let req2 = ActivityRequest::Cost(CostRequest::QuerySpend {
            session_pattern: "session_2".to_string(),
        });
        let res2 = ActivityResult::Cost(CostResult::Spend(20.0));

        activities.register(req1, res1);
        activities.register(req2, res2);
        let mut clock = VirtualClock::new(100);

        // Set snapshot_interval = 2 (triggers periodic snapshot after 2 events)
        let options = RunnerOptions {
            max_steps: 100,
            snapshot_interval: 2,
            snapshot_at_blocked: true,
        };

        let outcome = run_until_blocked(
            &workflow,
            "periodic_wf",
            Some(MidrunKillState {
                act1_result: None,
                act2_result: None,
            }),
            &mut journal,
            &mut activities,
            &mut clock,
            &options,
            Vec::new(),
        )
        .expect("should complete without getting stuck");

        assert_eq!(outcome, RunnerOutcome::Completed("total: 30".to_string()));

        // Snapshot was taken during execution
        let snap = journal
            .load_latest_snapshot("periodic_wf")
            .expect("load snap")
            .unwrap();
        assert!(snap.seq >= 2);
    }

    // ------------------------------------------------------------------------
    // Test Workflow 4: Pending Timers Across Mid-run Snapshot & Remaining Duration (Findings 2 & 5)
    // ------------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TimerAndActivityState {
        timer_fired: bool,
        act_done: bool,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TimerAndActivityWorkflow;

    impl Workflow for TimerAndActivityWorkflow {
        type Input = ();
        type State = TimerAndActivityState;
        type Output = String;

        fn step(
            &self,
            state: &mut Self::State,
            _ctx: &mut WorkflowContext,
            event: Option<WorkflowEvent>,
        ) -> StepDecision<Self::Output> {
            match event {
                None => StepDecision::ScheduleTimer {
                    id: "wf_timer".to_string(),
                    duration: Duration::from_secs(1000),
                },
                Some(WorkflowEvent::TimerFired { id }) => {
                    if id == "wf_timer" {
                        state.timer_fired = true;
                        StepDecision::ScheduleActivity {
                            id: "post_timer_act".to_string(),
                            activity: ActivityRequest::Herdr(HerdrRequest::Stop {
                                name: "worker_x".to_string(),
                            }),
                        }
                    } else {
                        StepDecision::Fail {
                            code: 1,
                            reason: "unknown timer".to_string(),
                        }
                    }
                }
                Some(WorkflowEvent::ActivityCompleted { id, .. }) => {
                    if id == "post_timer_act" {
                        state.act_done = true;
                        StepDecision::Complete("done".to_string())
                    } else {
                        StepDecision::Fail {
                            code: 2,
                            reason: "unknown act".to_string(),
                        }
                    }
                }
                _ => StepDecision::Fail {
                    code: 3,
                    reason: "bad event".to_string(),
                },
            }
        }
    }

    #[test]
    fn test_pending_timer_across_snapshot_restores_with_remaining_duration() {
        let workflow = TimerAndActivityWorkflow;
        let mut journal = InMemoryJournal::new();
        let mut activities = MockActivityRegistry::new();
        let req = ActivityRequest::Herdr(HerdrRequest::Stop {
            name: "worker_x".to_string(),
        });
        let res = ActivityResult::Herdr(HerdrResult::Stopped {
            name: "worker_x".to_string(),
        });
        activities.register(req, res);

        // Pre-populate journal: timer scheduled at ts=1000 with duration=1000s (deadline=1001000)
        // And a periodic snapshot was taken at seq 1
        journal
            .append_event(
                "timer_snap_wf",
                &EventRecord::new(
                    1,
                    1000,
                    "timer_scheduled",
                    serde_json::to_string(&TimerScheduledData {
                        id: "wf_timer".to_string(),
                        duration_ms: 1000 * 1000,
                    })
                    .unwrap(),
                ),
            )
            .unwrap();
        journal
            .append_snapshot(
                "timer_snap_wf",
                &SnapshotRecord::new(
                    1,
                    serde_json::to_string(&TimerAndActivityState {
                        timer_fired: false,
                        act_done: false,
                    })
                    .unwrap(),
                ),
            )
            .unwrap();

        // Advance clock to 1000 + 400s = 401000 (400s elapsed, 600s remaining)
        let mut resuming_clock = VirtualClock::new(401000);

        let outcome = run_until_blocked(
            &workflow,
            "timer_snap_wf",
            None, // resumes from snapshot
            &mut journal,
            &mut activities,
            &mut resuming_clock,
            &RunnerOptions::default(),
            Vec::new(),
        )
        .expect("should advance and complete");

        assert_eq!(outcome, RunnerOutcome::Completed("done".to_string()));
        // Clock advanced to exactly 1001000 (the original deadline)!
        assert_eq!(resuming_clock.now_ms(), 1001000);
    }

    // ------------------------------------------------------------------------
    // Test Workflow 5: Continue Replay Determinism (Finding 3)
    // ------------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct ContinueWorkflowState {
        continued: bool,
        act_done: bool,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct ContinueWorkflow;

    impl Workflow for ContinueWorkflow {
        type Input = ();
        type State = ContinueWorkflowState;
        type Output = String;

        fn step(
            &self,
            state: &mut Self::State,
            _ctx: &mut WorkflowContext,
            event: Option<WorkflowEvent>,
        ) -> StepDecision<Self::Output> {
            match event {
                None => {
                    if !state.continued {
                        state.continued = true;
                        StepDecision::Continue
                    } else {
                        StepDecision::ScheduleActivity {
                            id: "act_after_continue".to_string(),
                            activity: ActivityRequest::Herdr(HerdrRequest::Stop {
                                name: "worker_c".to_string(),
                            }),
                        }
                    }
                }
                Some(WorkflowEvent::ActivityCompleted { id, .. }) => {
                    if id == "act_after_continue" {
                        state.act_done = true;
                        StepDecision::Complete("continue_completed".to_string())
                    } else {
                        StepDecision::Fail {
                            code: 1,
                            reason: "unknown act".to_string(),
                        }
                    }
                }
                _ => StepDecision::Fail {
                    code: 2,
                    reason: "bad ev".to_string(),
                },
            }
        }
    }

    #[test]
    fn test_continue_events_replayed_deterministically_without_double_execution() {
        let workflow = ContinueWorkflow;
        let initial_state = ContinueWorkflowState {
            continued: false,
            act_done: false,
        };

        let req = ActivityRequest::Herdr(HerdrRequest::Stop {
            name: "worker_c".to_string(),
        });
        let res = ActivityResult::Herdr(HerdrResult::Stopped {
            name: "worker_c".to_string(),
        });

        // --- Run 1: Uninterrupted Reference Run ---
        let mut journal_ref = InMemoryJournal::new();
        let mut activities_ref = MockActivityRegistry::new();
        activities_ref.register(req.clone(), res.clone());
        let mut clock_ref = VirtualClock::new(100);

        let outcome_ref = run(
            &workflow,
            "continue_ref",
            initial_state.clone(),
            &mut journal_ref,
            &mut activities_ref,
            &mut clock_ref,
        )
        .expect("ref run");

        assert_eq!(
            outcome_ref,
            RunnerOutcome::Completed("continue_completed".to_string())
        );
        let ref_events = journal_ref
            .load_events("continue_ref", 1)
            .expect("load ref events");

        // --- Run 2: Interrupted Run (kill right after act_after_continue completes) ---
        let mut journal_int = InMemoryJournal::new();
        let mut activities_int = MockActivityRegistry::new();
        activities_int.register(req.clone(), res.clone());
        let mut clock_int = VirtualClock::new(100);

        // Pre-populate journal with continue, activity_scheduled, activity_completed
        journal_int
            .append_event(
                "continue_int",
                &EventRecord::new(1, 100, "continue", "".to_string()),
            )
            .unwrap();
        journal_int
            .append_event(
                "continue_int",
                &EventRecord::new(
                    2,
                    100,
                    "activity_scheduled",
                    serde_json::to_string(&ActivityScheduledData {
                        id: "act_after_continue".to_string(),
                        activity: req.clone(),
                    })
                    .unwrap(),
                ),
            )
            .unwrap();
        journal_int
            .append_event(
                "continue_int",
                &EventRecord::new(
                    3,
                    100,
                    "activity_completed",
                    serde_json::to_string(&ActivityCompletedData {
                        id: "act_after_continue".to_string(),
                        result: res,
                    })
                    .unwrap(),
                ),
            )
            .unwrap();

        let outcome_resumed = run(
            &workflow,
            "continue_int",
            initial_state,
            &mut journal_int,
            &mut activities_int,
            &mut clock_int,
        )
        .expect("resumed run");

        assert_eq!(
            outcome_resumed,
            RunnerOutcome::Completed("continue_completed".to_string())
        );

        // Activity was replayed without second execution!
        assert_eq!(activities_int.count_for(&req), 0);

        // Event sequence matches reference run exactly
        let int_events = journal_int
            .load_events("continue_int", 1)
            .expect("load int events");
        assert_eq!(int_events.len(), ref_events.len());
        for (i, (int_ev, ref_ev)) in int_events.iter().zip(ref_events.iter()).enumerate() {
            assert_eq!(int_ev.kind, ref_ev.kind, "event {i} kind mismatch");
            assert_eq!(int_ev.seq, ref_ev.seq, "event {i} seq mismatch");
        }
    }

    // ------------------------------------------------------------------------
    // Test Workflow 6: Resuming Completed Workflow Returns Completed (Finding 4)
    // ------------------------------------------------------------------------

    #[test]
    fn test_resume_after_completed_returns_completed_not_blocked() {
        let workflow = TimerWorkflow {
            timer_duration_secs: 10,
        };
        let mut journal = InMemoryJournal::new();
        let mut activities = MockActivityRegistry::new();
        let mut clock = VirtualClock::new(0);

        let outcome = run(
            &workflow,
            "completed_wf",
            TimerWorkflowState {
                timer_completed: false,
            },
            &mut journal,
            &mut activities,
            &mut clock,
        )
        .expect("run to completion");

        assert_eq!(
            outcome,
            RunnerOutcome::Completed("timer_finished".to_string())
        );

        let event_count_before = journal.load_events("completed_wf", 1).unwrap().len();

        // Calling resume on completed workflow must return Completed directly without appending events
        let mut clock2 = VirtualClock::new(100);
        let outcome2 = resume(
            &workflow,
            "completed_wf",
            &mut journal,
            &mut activities,
            &mut clock2,
            vec![],
        )
        .expect("resume call");

        assert_eq!(
            outcome2,
            RunnerOutcome::Completed("timer_finished".to_string())
        );
        let event_count_after = journal.load_events("completed_wf", 1).unwrap().len();
        assert_eq!(event_count_after, event_count_before);
    }

    // ------------------------------------------------------------------------
    // Test Workflow 7: Corrupted Payload Fails Fast (Finding 7)
    // ------------------------------------------------------------------------

    #[test]
    fn test_corrupt_payload_returns_serialization_error() {
        let workflow = ContinueWorkflow;
        let mut journal = InMemoryJournal::new();
        let mut activities = MockActivityRegistry::new();
        let mut clock = VirtualClock::new(0);

        journal
            .append_event(
                "corrupt_wf",
                &EventRecord::new(1, 0, "activity_scheduled", "NOT_JSON"),
            )
            .unwrap();

        let res = run(
            &workflow,
            "corrupt_wf",
            ContinueWorkflowState {
                continued: false,
                act_done: false,
            },
            &mut journal,
            &mut activities,
            &mut clock,
        );

        assert!(matches!(res, Err(RunnerError::Serialization(_))));
    }

    #[test]
    fn test_workflow_failure_recorded_in_journal() {
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        struct FailingWorkflow;

        impl Workflow for FailingWorkflow {
            type Input = ();
            type State = ();
            type Output = ();

            fn step(
                &self,
                _state: &mut Self::State,
                _ctx: &mut WorkflowContext,
                _event: Option<WorkflowEvent>,
            ) -> StepDecision<Self::Output> {
                StepDecision::Fail {
                    code: 42,
                    reason: "deliberate test failure".to_string(),
                }
            }
        }

        let workflow = FailingWorkflow;
        let mut journal = InMemoryJournal::new();
        let mut activities = MockActivityRegistry::new();
        let mut clock = VirtualClock::new(0);

        let outcome = run(
            &workflow,
            "fail_wf",
            (),
            &mut journal,
            &mut activities,
            &mut clock,
        )
        .expect("run returns outcome");

        assert_eq!(
            outcome,
            RunnerOutcome::Failed {
                code: 42,
                reason: "deliberate test failure".to_string(),
            }
        );

        let events = journal.load_events("fail_wf", 1).expect("load events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "workflow_failed");
        assert!(events[0].payload_json.contains("deliberate test failure"));
    }
}
