//! The Quaqua embedded workflow engine core (W0 contract).
//!
//! Exposes event-sourced workflow interfaces, activity execution contracts,
//! object-safe journaling, GenAI telemetry, child worker definitions, and
//! deterministic in-memory testkit.

pub mod workflow;
pub mod activities;
pub mod activity;
pub mod journal;
pub mod journal_sqlite;
pub mod telemetry;
pub mod telemetry_otel;
pub mod child;
pub mod clock;
pub mod testkit;
pub mod guard;

#[cfg(test)]
mod tests;

pub use workflow::{Workflow, WorkflowContext, WorkflowEvent, StepDecision, ScheduledAction};
pub use activity::{Activity, ActivityRequest, ActivityResult, ActivityError};
pub use journal::{Journal, EventRecord, SnapshotRecord, JournalError};
pub use telemetry::{
    SpanSink, GenAiSpan, TelemetryError,
    GEN_AI_SYSTEM, GEN_AI_REQUEST_MODEL, GEN_AI_USAGE_INPUT_TOKENS,
    GEN_AI_USAGE_OUTPUT_TOKENS, GEN_AI_OPERATION_NAME, GEN_AI_AGENT_NAME, GEN_AI_AGENT_ROLE,
};
pub use child::{SingleWorkerInput, SingleWorkerOutput};
pub use testkit::{
    VirtualClock, MockActivities, InMemoryJournal, run_to_completion, TestkitError,
};
pub use guard::{
    DeterminismViolation, scan_source_for_determinism_violations,
    scan_directory_for_determinism_violations,
};
