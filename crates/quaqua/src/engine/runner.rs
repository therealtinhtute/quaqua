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
pub trait RunnerClock: Send {
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

// ----------------------------------------------------------------------------
// Public Runner Entrypoints
// ----------------------------------------------------------------------------

/// Drives a workflow from initial state or prior snapshot until it completes, fails, or blocks.
pub fn run<W: Workflow, C: RunnerClock>(
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
pub fn resume<W: Workflow, C: RunnerClock>(
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
pub fn run_until_blocked<W: Workflow, C: RunnerClock>(
    workflow: &W,
    workflow_id: &str,
    initial_state: Option<W::State>,
    journal: &mut dyn Journal,
    activities: &mut dyn Activity,
    clock: &mut C,
    options: &RunnerOptions,
    incoming_events: Vec<WorkflowEvent>,
) -> Result<RunnerOutcome<W::Output>, RunnerError> {
    // 1. Load latest state snapshot
    let latest_snapshot = journal.load_latest_snapshot(workflow_id)?;
    let (mut state, mut current_seq, snapshot_seq) = match latest_snapshot {
        Some(snap) => {
            let s: W::State = serde_json::from_str(&snap.state_json)
                .map_err(|e| RunnerError::Serialization(e.to_string()))?;
            (s, snap.seq, snap.seq)
        }
        None => {
            let s = initial_state.ok_or(RunnerError::NoInitialState)?;
            (s, 0u64, 0u64)
        }
    };

    // 2. Load historical events since snapshot
    let history = journal.load_events(workflow_id, snapshot_seq + 1)?;
    for rec in &history {
        if rec.seq > current_seq {
            current_seq = rec.seq;
        }
    }

    // 3. Inspect history to categorize actions and identify terminal outcomes
    let mut completed_activities: HashMap<String, ActivityResult> = HashMap::new();
    let mut scheduled_activities: HashMap<String, ActivityRequest> = HashMap::new();
    let mut fired_timers: HashSet<String> = HashSet::new();
    let mut scheduled_timers: HashMap<String, Duration> = HashMap::new();
    let mut replay_inputs: VecDeque<WorkflowEvent> = VecDeque::new();
    let mut terminal_outcome: Option<RunnerOutcome<W::Output>> = None;

    for rec in &history {
        match rec.kind.as_str() {
            "activity_scheduled" => {
                if let Ok(data) = serde_json::from_str::<ActivityScheduledData>(&rec.payload_json) {
                    scheduled_activities.insert(data.id, data.activity);
                }
            }
            "activity_completed" => {
                if let Ok(data) = serde_json::from_str::<ActivityCompletedData>(&rec.payload_json) {
                    completed_activities.insert(data.id.clone(), data.result.clone());
                    replay_inputs.push_back(WorkflowEvent::ActivityCompleted {
                        id: data.id,
                        result: data.result,
                    });
                }
            }
            "timer_scheduled" => {
                if let Ok(data) = serde_json::from_str::<TimerScheduledData>(&rec.payload_json) {
                    scheduled_timers.insert(data.id, Duration::from_millis(data.duration_ms));
                }
            }
            "timer_fired" => {
                let id = if let Ok(data) = serde_json::from_str::<TimerFiredData>(&rec.payload_json)
                {
                    data.id
                } else if rec.payload_json.starts_with("id=") {
                    rec.payload_json.trim_start_matches("id=").to_string()
                } else {
                    rec.payload_json.clone()
                };
                fired_timers.insert(id.clone());
                replay_inputs.push_back(WorkflowEvent::TimerFired { id });
            }
            "child_started" => {}
            "child_completed" => {
                if let Ok(data) = serde_json::from_str::<ChildCompletedData>(&rec.payload_json) {
                    replay_inputs.push_back(WorkflowEvent::ChildCompleted {
                        id: data.id,
                        output: data.output,
                    });
                }
            }
            "child_failed" => {
                if let Ok(data) = serde_json::from_str::<ChildFailedData>(&rec.payload_json) {
                    replay_inputs.push_back(WorkflowEvent::ChildFailed {
                        id: data.id,
                        code: data.code,
                        reason: data.reason,
                    });
                }
            }
            "approval_resolved" => {
                if let Ok(data) = serde_json::from_str::<ApprovalResolvedData>(&rec.payload_json) {
                    replay_inputs.push_back(WorkflowEvent::ApprovalResolved {
                        approved: data.approved,
                        reason: data.reason,
                    });
                }
            }
            "workflow_completed" => {
                if let Ok(output) = serde_json::from_str::<W::Output>(&rec.payload_json) {
                    terminal_outcome = Some(RunnerOutcome::Completed(output));
                }
            }
            "workflow_failed" => {
                if let Ok(data) = serde_json::from_str::<WorkflowFailedData>(&rec.payload_json) {
                    terminal_outcome = Some(RunnerOutcome::Failed {
                        code: data.code,
                        reason: data.reason,
                    });
                }
            }
            _ => {}
        }
    }

    // If workflow was already finished in recorded history, return terminal outcome directly
    if let Some(outcome) = terminal_outcome {
        return Ok(outcome);
    }

    // 4. Replay historical events through step() to advance state deterministically
    let mut last_decision: Option<StepDecision<W::Output>> = None;
    let mut last_scheduled_actions: Vec<ScheduledAction> = Vec::new();

    if snapshot_seq == 0 && !history.is_empty() {
        let mut ctx = WorkflowContext::new(clock.now_ms());
        let dec = workflow.step(&mut state, &mut ctx, None);
        let actions = ctx.drain_scheduled();
        if replay_inputs.is_empty() {
            last_decision = Some(dec);
            last_scheduled_actions = actions;
        }
    }

    while let Some(ev) = replay_inputs.pop_front() {
        let mut ctx = WorkflowContext::new(clock.now_ms());
        let dec = workflow.step(&mut state, &mut ctx, Some(ev));
        let actions = ctx.drain_scheduled();
        if replay_inputs.is_empty() {
            last_decision = Some(dec);
            last_scheduled_actions = actions;
        }
    }

    // 5. Restore pending timers into clock seam
    for (id, dur) in &scheduled_timers {
        if !fired_timers.contains(id) {
            clock.schedule_timer(id, *dur);
        }
    }

    // 6. Transition to Forward Execution
    let mut event_queue: VecDeque<WorkflowEvent> = VecDeque::new();
    let mut events_since_snapshot = 0u64;
    let mut continue_immediate = false;

    // Dispatch unscheduled actions from the last step of replay
    for action in last_scheduled_actions {
        match action {
            ScheduledAction::Activity { id, activity } => {
                if !scheduled_activities.contains_key(&id) {
                    current_seq += 1;
                    journal.append_event(
                        workflow_id,
                        &EventRecord::new(
                            current_seq,
                            clock.now_ms(),
                            "activity_scheduled",
                            serde_json::to_string(&ActivityScheduledData {
                                id: id.clone(),
                                activity: activity.clone(),
                            })
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                        ),
                    )?;
                    events_since_snapshot += 1;

                    let result = activities.execute(&activity)?;
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
                    events_since_snapshot += 1;
                    event_queue.push_back(WorkflowEvent::ActivityCompleted { id, result });
                }
            }
            ScheduledAction::Timer { id, duration } => {
                if !scheduled_timers.contains_key(&id) {
                    current_seq += 1;
                    journal.append_event(
                        workflow_id,
                        &EventRecord::new(
                            current_seq,
                            clock.now_ms(),
                            "timer_scheduled",
                            serde_json::to_string(&TimerScheduledData {
                                id: id.clone(),
                                duration_ms: duration.as_millis() as u64,
                            })
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                        ),
                    )?;
                    events_since_snapshot += 1;
                    clock.schedule_timer(&id, duration);
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
    let had_replay_decision = last_decision.is_some();
    if let Some(decision) = last_decision {
        match decision {
            StepDecision::Continue => {
                continue_immediate = true;
            }
            StepDecision::ScheduleActivity { id, activity } => {
                if !scheduled_activities.contains_key(&id) {
                    current_seq += 1;
                    journal.append_event(
                        workflow_id,
                        &EventRecord::new(
                            current_seq,
                            clock.now_ms(),
                            "activity_scheduled",
                            serde_json::to_string(&ActivityScheduledData {
                                id: id.clone(),
                                activity: activity.clone(),
                            })
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                        ),
                    )?;
                    events_since_snapshot += 1;

                    let result = activities.execute(&activity)?;
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
                    events_since_snapshot += 1;
                    event_queue.push_back(WorkflowEvent::ActivityCompleted { id, result });
                }
            }
            StepDecision::ScheduleTimer { id, duration } => {
                if !scheduled_timers.contains_key(&id) {
                    current_seq += 1;
                    journal.append_event(
                        workflow_id,
                        &EventRecord::new(
                            current_seq,
                            clock.now_ms(),
                            "timer_scheduled",
                            serde_json::to_string(&TimerScheduledData {
                                id: id.clone(),
                                duration_ms: duration.as_millis() as u64,
                            })
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                        ),
                    )?;
                    events_since_snapshot += 1;
                    clock.schedule_timer(&id, duration);
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
                if !history.iter().any(|e| e.kind == "pause_for_approval") {
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

    let mut first_step_needed = snapshot_seq == 0 && history.is_empty() && !had_replay_decision;
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
            // No pending timers and no events in queue: workflow is Blocked
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
                    current_seq += 1;
                    journal.append_event(
                        workflow_id,
                        &EventRecord::new(
                            current_seq,
                            clock.now_ms(),
                            "activity_scheduled",
                            serde_json::to_string(&ActivityScheduledData {
                                id: id.clone(),
                                activity: activity.clone(),
                            })
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                        ),
                    )?;
                    events_since_snapshot += 1;

                    let result = activities.execute(&activity)?;
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
                    events_since_snapshot += 1;
                    event_queue.push_back(WorkflowEvent::ActivityCompleted { id, result });
                }
                ScheduledAction::Timer { id, duration } => {
                    current_seq += 1;
                    journal.append_event(
                        workflow_id,
                        &EventRecord::new(
                            current_seq,
                            clock.now_ms(),
                            "timer_scheduled",
                            serde_json::to_string(&TimerScheduledData {
                                id: id.clone(),
                                duration_ms: duration.as_millis() as u64,
                            })
                            .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                        ),
                    )?;
                    events_since_snapshot += 1;
                    clock.schedule_timer(&id, duration);
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
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "activity_scheduled",
                        serde_json::to_string(&ActivityScheduledData {
                            id: id.clone(),
                            activity: activity.clone(),
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                events_since_snapshot += 1;

                let result = activities.execute(&activity)?;
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
                events_since_snapshot += 1;
                event_queue.push_back(WorkflowEvent::ActivityCompleted { id, result });
            }
            StepDecision::ScheduleTimer { id, duration } => {
                current_seq += 1;
                journal.append_event(
                    workflow_id,
                    &EventRecord::new(
                        current_seq,
                        clock.now_ms(),
                        "timer_scheduled",
                        serde_json::to_string(&TimerScheduledData {
                            id: id.clone(),
                            duration_ms: duration.as_millis() as u64,
                        })
                        .map_err(|e| RunnerError::Serialization(e.to_string()))?,
                    ),
                )?;
                events_since_snapshot += 1;
                clock.schedule_timer(&id, duration);
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

        // Periodic snapshotting if configured
        if options.snapshot_interval > 0 && events_since_snapshot >= options.snapshot_interval {
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

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
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
    // Test Workflow 1: Long 7200s Timer Workflow
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

        // Verify recorded events in journal
        let events = journal
            .load_events("wf_timer_7200", 1)
            .expect("events load");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind, "timer_scheduled");
        assert_eq!(events[1].kind, "timer_fired");
        assert_eq!(events[2].kind, "workflow_completed");
    }

    // ------------------------------------------------------------------------
    // Test Workflow 2: Approval Workflow with Activity (Kill & Resume)
    // ------------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct ApprovalWorkflowState {
        activity_result: Option<f64>,
        approved: bool,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct ApprovalWorkflow;

    impl Workflow for ApprovalWorkflow {
        type Input = ();
        type State = ApprovalWorkflowState;
        type Output = String;

        fn step(
            &self,
            state: &mut Self::State,
            _ctx: &mut WorkflowContext,
            event: Option<WorkflowEvent>,
        ) -> StepDecision<Self::Output> {
            match event {
                None => StepDecision::ScheduleActivity {
                    id: "query_spend".to_string(),
                    activity: ActivityRequest::Cost(CostRequest::QuerySpend {
                        session_pattern: "worker_a".to_string(),
                    }),
                },
                Some(WorkflowEvent::ActivityCompleted { id, result }) => {
                    if id == "query_spend" {
                        if let ActivityResult::Cost(CostResult::Spend(v)) = result {
                            state.activity_result = Some(v);
                            StepDecision::PauseForApproval {
                                reason: format!("spend was {v}, need signoff"),
                            }
                        } else {
                            StepDecision::Fail {
                                code: 1,
                                reason: "bad activity result".to_string(),
                            }
                        }
                    } else {
                        StepDecision::Fail {
                            code: 2,
                            reason: "unknown activity".to_string(),
                        }
                    }
                }
                Some(WorkflowEvent::ApprovalResolved { approved, .. }) => {
                    state.approved = approved;
                    if approved {
                        StepDecision::Complete(format!(
                            "approved with spend {}",
                            state.activity_result.unwrap_or(0.0)
                        ))
                    } else {
                        StepDecision::Fail {
                            code: 3,
                            reason: "rejected".to_string(),
                        }
                    }
                }
                _ => StepDecision::Fail {
                    code: 4,
                    reason: "unexpected event".to_string(),
                },
            }
        }
    }

    #[test]
    fn test_kill_and_resume_preserves_state_and_event_sequence() {
        let workflow = ApprovalWorkflow;
        let initial_state = ApprovalWorkflowState {
            activity_result: None,
            approved: false,
        };

        let spend_req = ActivityRequest::Cost(CostRequest::QuerySpend {
            session_pattern: "worker_a".to_string(),
        });
        let spend_res = ActivityResult::Cost(CostResult::Spend(42.50));

        // --- Run 1: Uninterrupted Reference Run ---
        let mut journal_ref = InMemoryJournal::new();
        let mut activities_ref = MockActivityRegistry::new();
        activities_ref.register(spend_req.clone(), spend_res.clone());
        let mut clock_ref = VirtualClock::new(5000);

        // Run until pause for approval (Blocked)
        let outcome_ref1 = run(
            &workflow,
            "ref_wf",
            initial_state.clone(),
            &mut journal_ref,
            &mut activities_ref,
            &mut clock_ref,
        )
        .expect("ref run 1 should succeed");

        assert_eq!(
            outcome_ref1,
            RunnerOutcome::Blocked("pause_for_approval: spend was 42.5, need signoff".to_string())
        );

        // Provide approval to complete uninterrupted run
        let outcome_ref2 = resume(
            &workflow,
            "ref_wf",
            &mut journal_ref,
            &mut activities_ref,
            &mut clock_ref,
            vec![WorkflowEvent::ApprovalResolved {
                approved: true,
                reason: Some("looks good".to_string()),
            }],
        )
        .expect("ref resume should succeed");

        assert_eq!(
            outcome_ref2,
            RunnerOutcome::Completed("approved with spend 42.5".to_string())
        );

        let ref_events = journal_ref
            .load_events("ref_wf", 1)
            .expect("load ref events");
        let ref_snapshot = journal_ref
            .load_latest_snapshot("ref_wf")
            .expect("load ref snapshot")
            .expect("snapshot exists");

        // --- Run 2: Interrupted Run (Kill mid-run -> Resume fresh runner) ---
        let mut journal_interrupted = InMemoryJournal::new();
        let mut activities_interrupted = MockActivityRegistry::new();
        activities_interrupted.register(spend_req.clone(), spend_res);
        let mut clock_interrupted = VirtualClock::new(5000);

        // Run to Blocked
        let outcome_int1 = run(
            &workflow,
            "int_wf",
            initial_state,
            &mut journal_interrupted,
            &mut activities_interrupted,
            &mut clock_interrupted,
        )
        .expect("interrupted run 1 should succeed");

        assert_eq!(
            outcome_int1,
            RunnerOutcome::Blocked("pause_for_approval: spend was 42.5, need signoff".to_string())
        );
        assert_eq!(activities_interrupted.count_for(&spend_req), 1);
        assert_eq!(activities_interrupted.call_count(), 1);
        // *** KILL / CRASH SIMULATION ***
        // Discard old runner context; create a fresh runner over the existing journal!
        let mut fresh_clock = VirtualClock::new(5000);

        // Resume fresh runner with approval event
        let outcome_resumed = resume(
            &workflow,
            "int_wf",
            &mut journal_interrupted,
            &mut activities_interrupted,
            &mut fresh_clock,
            vec![WorkflowEvent::ApprovalResolved {
                approved: true,
                reason: Some("looks good".to_string()),
            }],
        )
        .expect("resumed run should succeed");

        assert_eq!(
            outcome_resumed,
            RunnerOutcome::Completed("approved with spend 42.5".to_string())
        );

        // Crucial acceptance check: Activity MUST NOT execute a second time during replay / resume!
        assert_eq!(
            activities_interrupted.count_for(&spend_req),
            1,
            "activity was re-executed on resume!"
        );
        assert_eq!(activities_interrupted.call_count(), 1);

        // Verify state snapshot equivalence
        let int_snapshot = journal_interrupted
            .load_latest_snapshot("int_wf")
            .expect("load int snapshot")
            .expect("snapshot exists");
        assert_eq!(int_snapshot.state_json, ref_snapshot.state_json);

        // Verify complete journal event sequence equivalence
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
    // Test Workflow 3: Sequential Activities Replay Test
    // ------------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct MultiActivityState {
        stage: usize,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct MultiActivityWorkflow;

    impl Workflow for MultiActivityWorkflow {
        type Input = ();
        type State = MultiActivityState;
        type Output = usize;

        fn step(
            &self,
            state: &mut Self::State,
            _ctx: &mut WorkflowContext,
            event: Option<WorkflowEvent>,
        ) -> StepDecision<Self::Output> {
            match event {
                None => StepDecision::ScheduleActivity {
                    id: "act_1".to_string(),
                    activity: ActivityRequest::Herdr(HerdrRequest::Stop {
                        name: "worker1".to_string(),
                    }),
                },
                Some(WorkflowEvent::ActivityCompleted { id, .. }) => {
                    if id == "act_1" {
                        state.stage = 1;
                        StepDecision::ScheduleActivity {
                            id: "act_2".to_string(),
                            activity: ActivityRequest::Herdr(HerdrRequest::Stop {
                                name: "worker2".to_string(),
                            }),
                        }
                    } else if id == "act_2" {
                        state.stage = 2;
                        StepDecision::ScheduleActivity {
                            id: "act_3".to_string(),
                            activity: ActivityRequest::Herdr(HerdrRequest::Stop {
                                name: "worker3".to_string(),
                            }),
                        }
                    } else if id == "act_3" {
                        state.stage = 3;
                        StepDecision::Complete(state.stage)
                    } else {
                        StepDecision::Fail {
                            code: 9,
                            reason: "unknown act".to_string(),
                        }
                    }
                }
                _ => StepDecision::Fail {
                    code: 10,
                    reason: "bad event".to_string(),
                },
            }
        }
    }

    #[test]
    fn test_replay_never_reexecutes_completed_activities() {
        let workflow = MultiActivityWorkflow;
        let mut journal = InMemoryJournal::new();
        let mut activities = MockActivityRegistry::new();

        let req1 = ActivityRequest::Herdr(HerdrRequest::Stop {
            name: "worker1".to_string(),
        });
        let res1 = ActivityResult::Herdr(HerdrResult::Stopped {
            name: "worker1".to_string(),
        });
        let req2 = ActivityRequest::Herdr(HerdrRequest::Stop {
            name: "worker2".to_string(),
        });
        let res2 = ActivityResult::Herdr(HerdrResult::Stopped {
            name: "worker2".to_string(),
        });
        let req3 = ActivityRequest::Herdr(HerdrRequest::Stop {
            name: "worker3".to_string(),
        });
        let res3 = ActivityResult::Herdr(HerdrResult::Stopped {
            name: "worker3".to_string(),
        });

        activities.register(req1.clone(), res1.clone());
        activities.register(req2.clone(), res2.clone());
        activities.register(req3.clone(), res3);
        let mut clock = VirtualClock::new(100);

        // Pre-populate journal with act_1 and act_2 completed (simulate crash before act_3)
        journal
            .append_event(
                "multi_wf",
                &EventRecord::new(
                    1,
                    100,
                    "activity_scheduled",
                    serde_json::to_string(&ActivityScheduledData {
                        id: "act_1".to_string(),
                        activity: req1.clone(),
                    })
                    .unwrap(),
                ),
            )
            .unwrap();
        journal
            .append_event(
                "multi_wf",
                &EventRecord::new(
                    2,
                    100,
                    "activity_completed",
                    serde_json::to_string(&ActivityCompletedData {
                        id: "act_1".to_string(),
                        result: res1,
                    })
                    .unwrap(),
                ),
            )
            .unwrap();

        journal
            .append_event(
                "multi_wf",
                &EventRecord::new(
                    3,
                    100,
                    "activity_scheduled",
                    serde_json::to_string(&ActivityScheduledData {
                        id: "act_2".to_string(),
                        activity: req2.clone(),
                    })
                    .unwrap(),
                ),
            )
            .unwrap();
        journal
            .append_event(
                "multi_wf",
                &EventRecord::new(
                    4,
                    100,
                    "activity_completed",
                    serde_json::to_string(&ActivityCompletedData {
                        id: "act_2".to_string(),
                        result: res2,
                    })
                    .unwrap(),
                ),
            )
            .unwrap();

        // Run workflow starting from initial state with pre-populated journal
        let outcome = run(
            &workflow,
            "multi_wf",
            MultiActivityState { stage: 0 },
            &mut journal,
            &mut activities,
            &mut clock,
        )
        .expect("should run to completion");

        assert_eq!(outcome, RunnerOutcome::Completed(3));

        // act_1 and act_2 MUST NOT have been executed via MockActivityRegistry during replay!
        assert_eq!(activities.count_for(&req1), 0);
        assert_eq!(activities.count_for(&req2), 0);

        // Only act_3 was executed in forward execution!
        assert_eq!(activities.count_for(&req3), 1);
    }

    #[test]
    fn test_crash_before_activity_completion_executes_once_on_resume() {
        let workflow = MultiActivityWorkflow;
        let mut journal = InMemoryJournal::new();
        let mut activities = MockActivityRegistry::new();

        let req1 = ActivityRequest::Herdr(HerdrRequest::Stop {
            name: "worker1".to_string(),
        });
        let res1 = ActivityResult::Herdr(HerdrResult::Stopped {
            name: "worker1".to_string(),
        });
        let req2 = ActivityRequest::Herdr(HerdrRequest::Stop {
            name: "worker2".to_string(),
        });
        let res2 = ActivityResult::Herdr(HerdrResult::Stopped {
            name: "worker2".to_string(),
        });
        let req3 = ActivityRequest::Herdr(HerdrRequest::Stop {
            name: "worker3".to_string(),
        });
        let res3 = ActivityResult::Herdr(HerdrResult::Stopped {
            name: "worker3".to_string(),
        });

        activities.register(req1.clone(), res1);
        activities.register(req2.clone(), res2);
        activities.register(req3.clone(), res3);
        let mut clock = VirtualClock::new(100);

        // Pre-populate journal with act_1 SCHEDULED but NOT COMPLETED (killed during act_1)
        journal
            .append_event(
                "crash_wf",
                &EventRecord::new(
                    1,
                    100,
                    "activity_scheduled",
                    serde_json::to_string(&ActivityScheduledData {
                        id: "act_1".to_string(),
                        activity: req1.clone(),
                    })
                    .unwrap(),
                ),
            )
            .unwrap();

        // Runner starts and recovers
        let outcome = run(
            &workflow,
            "crash_wf",
            MultiActivityState { stage: 0 },
            &mut journal,
            &mut activities,
            &mut clock,
        )
        .expect("should recover and complete");

        assert_eq!(outcome, RunnerOutcome::Completed(3));
        // act_1 was executed once on recovery
        assert_eq!(activities.count_for(&req1), 1);
        assert_eq!(activities.count_for(&req2), 1);
        assert_eq!(activities.count_for(&req3), 1);
    }

    #[test]
    fn test_snapshot_at_blocked_enables_instant_resume() {
        let workflow = ApprovalWorkflow;
        let mut journal = InMemoryJournal::new();
        let mut activities = MockActivityRegistry::new();

        let req = ActivityRequest::Cost(CostRequest::QuerySpend {
            session_pattern: "worker_a".to_string(),
        });
        let res = ActivityResult::Cost(CostResult::Spend(10.0));
        activities.register(req, res);
        let mut clock = VirtualClock::new(200);

        // Run until Blocked
        let outcome = run(
            &workflow,
            "snap_wf",
            ApprovalWorkflowState {
                activity_result: None,
                approved: false,
            },
            &mut journal,
            &mut activities,
            &mut clock,
        )
        .expect("run to blocked");

        assert!(matches!(outcome, RunnerOutcome::Blocked(_)));

        // Verify snapshot was written at Blocked
        let snap = journal
            .load_latest_snapshot("snap_wf")
            .expect("load snapshot")
            .expect("snapshot must exist at Blocked");
        assert!(snap.seq >= 3);
        let parsed: ApprovalWorkflowState = serde_json::from_str(&snap.state_json).unwrap();
        assert_eq!(parsed.activity_result, Some(10.0));
        assert!(!parsed.approved);

        // When resuming with a fresh runner without passing initial_state, it resumes from snapshot
        let mut clock2 = VirtualClock::new(200);
        let outcome2 = resume(
            &workflow,
            "snap_wf",
            &mut journal,
            &mut activities,
            &mut clock2,
            vec![WorkflowEvent::ApprovalResolved {
                approved: true,
                reason: None,
            }],
        )
        .expect("resume");

        assert_eq!(
            outcome2,
            RunnerOutcome::Completed("approved with spend 10".to_string())
        );
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
