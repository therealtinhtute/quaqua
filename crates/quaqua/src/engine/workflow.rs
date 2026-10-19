//! Workflow abstractions and execution contracts for the event-sourced engine.
//!
//! Workflows are deterministic state machines executed step-by-step.
//! Inside [`Workflow::step`], workflows must not perform side-effects or direct I/O
//! (`std::fs`, `std::process`, `std::net`). All interaction with external systems
//! occurs through scheduled activities or timers.

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use crate::engine::activity::{ActivityRequest, ActivityResult};

/// Pure workflow context passed to [`Workflow::step`].
///
/// Exposes scheduling time and metadata only. Intentionally provides no handles
/// to filesystem, processes, or network resources.
#[derive(Debug, Clone)]
pub struct WorkflowContext {
    now_ms: u64,
}

impl WorkflowContext {
    /// Creates a new workflow context with the specified virtual timestamp (milliseconds since Unix epoch).
    pub fn new(now_ms: u64) -> Self {
        Self { now_ms }
    }

    /// Virtual current time in milliseconds since Unix epoch.
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// Virtual current time as [`SystemTime`].
    pub fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.now_ms)
    }

    /// Updates virtual current time in milliseconds.
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }
}

/// Incoming event delivered to a workflow step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkflowEvent {
    /// An activity previously scheduled has completed.
    ActivityCompleted {
        id: String,
        result: ActivityResult,
    },
    /// A timer previously scheduled has fired.
    TimerFired {
        id: String,
    },
    /// A child workflow has completed successfully.
    ChildCompleted {
        id: String,
        output: serde_json::Value,
    },
    /// A child workflow has failed.
    ChildFailed {
        id: String,
        code: i32,
        reason: String,
    },
    /// An approval pause was resolved.
    ApprovalResolved {
        approved: bool,
        reason: Option<String>,
    },
}

/// Outgoing decision returned by [`Workflow::step`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StepDecision<Output> {
    /// Transition state and continue immediately without external events.
    Continue,
    /// Schedule an asynchronous activity.
    ScheduleActivity {
        id: String,
        activity: ActivityRequest,
    },
    /// Schedule a timer for future execution.
    ScheduleTimer {
        id: String,
        duration: Duration,
    },
    /// Start a child workflow.
    StartChild {
        id: String,
        workflow_type: String,
        input: serde_json::Value,
    },
    /// Pause workflow execution waiting for operator/user approval.
    PauseForApproval {
        reason: String,
    },
    /// Complete the workflow with final output.
    Complete(Output),
    /// Terminate the workflow with failure.
    Fail {
        code: i32,
        reason: String,
    },
}

/// Deterministic, event-driven workflow definition.
pub trait Workflow: Send + Sync + 'static {
    /// Initial input data for the workflow.
    type Input: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;
    /// Workflow internal state, serializable for checkpointing/snapshots.
    type State: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;
    /// Terminal workflow output.
    type Output: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;

    /// Evaluates one deterministic step of the workflow.
    ///
    /// The workflow updates its `state` based on the incoming `event` and returns
    /// a [`StepDecision`].
    fn step(
        &self,
        state: &mut Self::State,
        ctx: &mut WorkflowContext,
        event: Option<WorkflowEvent>,
    ) -> StepDecision<Self::Output>;
}
