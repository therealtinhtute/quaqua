//! Activity trait and typed request/result contracts.
//!
//! Activities encapsulate all side-effecting operations (herdr execution,
//! git commands, pane grid layouts, report filesystem access, and cost tracking).

use serde::{Deserialize, Serialize};

/// Common error type for activity execution failures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ActivityError {
    /// The activity execution failed with an error message.
    ExecutionFailed(String),
    /// The target resource was not found.
    NotFound(String),
    /// The operation timed out.
    Timeout(String),
    /// The operation was cancelled.
    Cancelled(String),
}

impl std::fmt::Display for ActivityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExecutionFailed(msg) => write!(f, "activity execution failed: {msg}"),
            Self::NotFound(msg) => write!(f, "activity resource not found: {msg}"),
            Self::Timeout(msg) => write!(f, "activity timed out: {msg}"),
            Self::Cancelled(msg) => write!(f, "activity cancelled: {msg}"),
        }
    }
}

impl std::error::Error for ActivityError {}

// ----------------------------------------------------------------------------
// Herdr Control
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HerdrRequest {
    Spawn {
        name: String,
        kind: String,
        repo: String,
    },
    Cleanup {
        name: String,
        repo: String,
    },
    Prompt {
        name: String,
        task: String,
    },
    SendKeys {
        name: String,
        keys: String,
    },
    Stop {
        name: String,
    },
    ReadAgent {
        name: String,
        lines: usize,
    },
    AgentInfo {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HerdrResult {
    Spawned { name: String },
    Cleaned { name: String },
    Prompted { name: String },
    KeysSent { name: String },
    Stopped { name: String },
    AgentOutput { content: String },
    AgentInfo {
        name: String,
        status: String,
        harness: Option<String>,
        hook_active: bool,
    },
}

// ----------------------------------------------------------------------------
// Herdr Events
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HerdrEventRequest {
    Subscribe {
        agent: String,
        event_types: Vec<String>,
    },
    Unsubscribe {
        agent: String,
    },
    PollEvents,
    PollEventsFor {
        agent: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HerdrEventResult {
    Subscribed { agent: String },
    Unsubscribed { agent: String },
    Events(Vec<String>),
    ScopedEvents {
        agent: String,
        events: Vec<String>,
    },
}

// ----------------------------------------------------------------------------
// Git
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GitRequest {
    WorktreeAdd {
        path: String,
        branch: String,
        base: Option<String>,
    },
    WorktreeRemove {
        path: String,
    },
    BranchCreate {
        name: String,
        base: String,
    },
    Commit {
        message: String,
    },
    Merge {
        branch: String,
    },
    CurrentSha,
    DiffHash {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GitResult {
    WorktreeAdded { path: String },
    WorktreeRemoved { path: String },
    BranchCreated { name: String },
    Committed { sha: String },
    Merged { sha: String },
    Sha(String),
    DiffHash {
        hash: String,
        stat: String,
    },
}

// ----------------------------------------------------------------------------
// Grid
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GridRequest {
    Reconcile {
        tab: Option<String>,
        only: Option<String>,
    },
    Plan {
        tab: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GridResult {
    Reconciled { changed: bool },
    PlanJson(String),
}

// ----------------------------------------------------------------------------
// Report File
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReportFileRequest {
    Read { path: String },
    Write { path: String, content: String },
    CheckMtime { path: String },
    Exists { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReportFileResult {
    Content(String),
    Written,
    MtimeMs(u64),
    Exists(bool),
}

// ----------------------------------------------------------------------------
// Cost
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CostRequest {
    QuerySpend { session_pattern: String },
    CheckBudget { spend: f64, budget: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CostResult {
    Spend(f64),
    BudgetExceeded(bool),
}

// ----------------------------------------------------------------------------
// Unified Request / Result Enums
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ActivityRequest {
    Herdr(HerdrRequest),
    HerdrEvents(HerdrEventRequest),
    Git(GitRequest),
    Grid(GridRequest),
    ReportFile(ReportFileRequest),
    Cost(CostRequest),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ActivityResult {
    Herdr(HerdrResult),
    HerdrEvents(HerdrEventResult),
    Git(GitResult),
    Grid(GridResult),
    ReportFile(ReportFileResult),
    Cost(CostResult),
}

/// Object-safe activity execution interface.
pub trait Activity: Send + Sync {
    /// Executes a typed activity request and returns a typed result.
    fn execute(&self, req: &ActivityRequest) -> Result<ActivityResult, ActivityError>;
}
