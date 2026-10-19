use std::path::{Path, PathBuf};
use std::time::Duration;
use serde::{Deserialize, Serialize};

use crate::engine::activity::{
    ActivityRequest, ActivityResult, GitRequest, GitResult,
};
use crate::engine::child::{SingleWorkerInput, SingleWorkerOutput};
use crate::engine::guard::{
    scan_directory_for_determinism_violations, scan_source_for_determinism_violations,
};
use crate::engine::journal::{EventRecord, Journal, SnapshotRecord};
use crate::engine::telemetry::{
    GenAiSpan, GEN_AI_AGENT_NAME, GEN_AI_AGENT_ROLE, GEN_AI_OPERATION_NAME,
    GEN_AI_REQUEST_MODEL, GEN_AI_SYSTEM, GEN_AI_USAGE_INPUT_TOKENS, GEN_AI_USAGE_OUTPUT_TOKENS,
};
use crate::engine::testkit::{
    run_to_completion, InMemoryJournal, MockActivities, TestkitError, VirtualClock,
};
use crate::engine::workflow::{StepDecision, Workflow, WorkflowContext, WorkflowEvent};

// ----------------------------------------------------------------------------
// Toy Workflow for Deterministic Testkit Verification
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ToyState {
    step_count: u32,
    activity_done: bool,
    timer_done: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ToyWorkflow;

impl Workflow for ToyWorkflow {
    type Input = String;
    type State = ToyState;
    type Output = String;

    fn step(
        &self,
        state: &mut Self::State,
        _ctx: &mut WorkflowContext,
        event: Option<WorkflowEvent>,
    ) -> StepDecision<Self::Output> {
        state.step_count += 1;

        match event {
            None => {
                // First step: schedule an activity
                StepDecision::ScheduleActivity {
                    id: "get_git_sha".to_string(),
                    activity: ActivityRequest::Git(GitRequest::CurrentSha),
                }
            }
            Some(WorkflowEvent::ActivityCompleted { id, result }) => {
                if id == "get_git_sha" {
                    if let ActivityResult::Git(GitResult::Sha(sha)) = result {
                        state.activity_done = true;
                        // Second step: schedule a timer after activity completes
                        return StepDecision::ScheduleTimer {
                            id: format!("timer_after_{sha}"),
                            duration: Duration::from_millis(500),
                        };
                    }
                }
                StepDecision::Fail {
                    code: 1,
                    reason: "unexpected activity result".to_string(),
                }
            }
            Some(WorkflowEvent::TimerFired { id }) => {
                if id.starts_with("timer_after_") {
                    state.timer_done = true;
                    // Third step: complete workflow with output
                    return StepDecision::Complete("TOY_SUCCESS".to_string());
                }
                StepDecision::Fail {
                    code: 2,
                    reason: "unexpected timer id".to_string(),
                }
            }
            _ => StepDecision::Fail {
                code: 3,
                reason: "unexpected workflow event".to_string(),
            },
        }
    }
}

#[test]
fn test_toy_workflow_runs_deterministically_twice() {
    let run_once = || {
        let workflow = ToyWorkflow;
        let mut clock = VirtualClock::new(1_000_000);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::new();

        activities.register(
            "get_git_sha",
            ActivityResult::Git(GitResult::Sha("6fc7b8a".to_string())),
        );

        let initial_state = ToyState {
            step_count: 0,
            activity_done: false,
            timer_done: false,
        };

        let result = run_to_completion(
            &workflow,
            "test-toy-run",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        );

        (result, journal.all_events("test-toy-run").to_vec())
    };

    let (res1, events1) = run_once();
    let (res2, events2) = run_once();

    assert_eq!(res1, Ok("TOY_SUCCESS".to_string()));
    assert_eq!(res2, Ok("TOY_SUCCESS".to_string()));

    // Verify exact event ordering and content matching
    assert_eq!(events1.len(), events2.len(), "event sequence length mismatch");
    assert!(!events1.is_empty(), "expected recorded journal events");

    for (i, (e1, e2)) in events1.iter().zip(events2.iter()).enumerate() {
        assert_eq!(e1.seq, e2.seq, "seq mismatch at event index {i}");
        assert_eq!(e1.ts_ms, e2.ts_ms, "timestamp mismatch at event index {i}");
        assert_eq!(e1.kind, e2.kind, "kind mismatch at event index {i}");
        assert_eq!(e1.payload_json, e2.payload_json, "payload mismatch at event index {i}");
    }

    // Verify the expected sequence of kinds
    let kinds: Vec<&str> = events1.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "activity_scheduled",
            "activity_completed",
            "timer_scheduled",
            "timer_fired",
            "workflow_completed"
        ]
    );
}

// ----------------------------------------------------------------------------
// Journal Object Safety Verification
// ----------------------------------------------------------------------------

#[test]
fn test_journal_trait_is_object_safe() {
    let mut in_mem = InMemoryJournal::new();
    let journal: &mut dyn Journal = &mut in_mem;

    let event = EventRecord::new(1, 100, "test_event", r#"{"hello":"world"}"#);
    let seq = journal.append_event("wf-1", &event).expect("append_event failed");
    assert_eq!(seq, 1);

    let snapshot = SnapshotRecord::new(1, r#"{"state":"init"}"#);
    journal.append_snapshot("wf-1", &snapshot).expect("append_snapshot failed");

    let loaded_events = journal.load_events("wf-1", 1).expect("load_events failed");
    assert_eq!(loaded_events.len(), 1);
    assert_eq!(loaded_events[0].kind, "test_event");

    let loaded_snap = journal.load_latest_snapshot("wf-1").expect("load_snapshot failed");
    assert!(loaded_snap.is_some());
    assert_eq!(loaded_snap.unwrap().state_json, r#"{"state":"init"}"#);
}

// ----------------------------------------------------------------------------
// Child Contract Verification
// ----------------------------------------------------------------------------

#[test]
fn test_child_worker_output_validation() {
    // Valid exit codes: 0 and 2..=6
    for valid_exit in [0, 2, 3, 4, 5, 6] {
        let out = SingleWorkerOutput::new(valid_exit, PathBuf::from("runs/test.md"));
        assert!(out.is_ok(), "expected exit code {valid_exit} to be valid");
        let unwrapped = out.unwrap();
        assert_eq!(unwrapped.exit, valid_exit);
        assert_eq!(unwrapped.report_path, PathBuf::from("runs/test.md"));
    }

    // Invalid exit codes
    for invalid_exit in [-1, 1, 7, 10, 127] {
        let out = SingleWorkerOutput::new(invalid_exit, PathBuf::from("runs/test.md"));
        assert!(out.is_err(), "expected exit code {invalid_exit} to be rejected");
    }

    let input = SingleWorkerInput::new(
        "worker-1",
        "sude",
        "quaqua",
        "build task",
        1800,
        Some(3.5),
    );
    assert_eq!(input.name, "worker-1");
    assert_eq!(input.budget_sec, 1800);
    assert_eq!(input.cost_budget, Some(3.5));
}

// ----------------------------------------------------------------------------
// Telemetry GenAI Semantic Conventions
// ----------------------------------------------------------------------------

#[test]
fn test_telemetry_genai_attributes() {
    let span = GenAiSpan::new("trace-1", "span-1", "pipeline_plan", "anthropic", 1000, 2500)
        .with_model("claude-3-5-sonnet")
        .with_usage(500, 120)
        .with_agent("sude-arch-w0", "builder");

    assert_eq!(span.attributes.get(GEN_AI_SYSTEM), Some(&"anthropic".to_string()));
    assert_eq!(span.attributes.get(GEN_AI_OPERATION_NAME), Some(&"pipeline_plan".to_string()));
    assert_eq!(span.attributes.get(GEN_AI_REQUEST_MODEL), Some(&"claude-3-5-sonnet".to_string()));
    assert_eq!(span.attributes.get(GEN_AI_USAGE_INPUT_TOKENS), Some(&"500".to_string()));
    assert_eq!(span.attributes.get(GEN_AI_USAGE_OUTPUT_TOKENS), Some(&"120".to_string()));
    assert_eq!(span.attributes.get(GEN_AI_AGENT_NAME), Some(&"sude-arch-w0".to_string()));
    assert_eq!(span.attributes.get(GEN_AI_AGENT_ROLE), Some(&"builder".to_string()));
}

// ----------------------------------------------------------------------------
// Determinism Guard Positive and Negative Tests
// ----------------------------------------------------------------------------

#[test]
fn test_determinism_detector_fires_on_positive_fixture() {
    let bad_code = include_str!("fixtures/bad_workflow.txt");
    let violations = scan_source_for_determinism_violations(bad_code, "fixtures/bad_workflow.txt");

    assert_eq!(violations.len(), 3, "expected exactly 3 violations, got {violations:?}");

    let patterns: Vec<&str> = violations.iter().map(|v| v.forbidden_pattern.as_str()).collect();
    assert!(patterns.contains(&"std::fs"), "missing std::fs detection");
    assert!(patterns.contains(&"std::process"), "missing std::process detection");
    assert!(patterns.contains(&"std::net"), "missing std::net detection");

    // Line numbers in fixtures/bad_workflow.txt:
    // line 8: std::fs
    // line 19: std::process
    // line 30: std::net
    let fs_violation = violations.iter().find(|v| v.forbidden_pattern == "std::fs").unwrap();
    assert_eq!(fs_violation.line, 8);

    let proc_violation = violations.iter().find(|v| v.forbidden_pattern == "std::process").unwrap();
    assert_eq!(proc_violation.line, 19);

    let net_violation = violations.iter().find(|v| v.forbidden_pattern == "std::net").unwrap();
    assert_eq!(net_violation.line, 30);
}

#[test]
fn test_all_crate_workflows_pass_determinism_guard() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");

    let violations = scan_directory_for_determinism_violations(&src_dir)
        .expect("failed to scan crate source directory");

    if !violations.is_empty() {
        let mut err = String::from("Determinism violations found in crate Workflow implementations:\n");
        for v in &violations {
            err.push_str(&format!("  {v}\n"));
        }
        panic!("{err}");
    }
}

// ----------------------------------------------------------------------------
// OWNERS.md Parsing and W2 Existence Tests
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct OwnerEntry {
    target: String,
    file_path: String,
    owner_lead: String,
    worker: String,
    description: String,
}

fn parse_owners_table(content: &str) -> Vec<OwnerEntry> {
    let mut entries = Vec::new();
    let mut in_table = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("| Target |") || trimmed.starts_with("|Target|") {
            in_table = true;
            continue;
        }
        if in_table && trimmed.starts_with("|---") {
            continue;
        }
        if in_table && trimmed.starts_with('|') {
            let cols: Vec<&str> = trimmed.split('|').map(|c| c.trim()).collect();
            // cols has leading and trailing empty elements from outer pipes: ["", Target, File Path, Owner, Worker, Description, ""]
            if cols.len() >= 6 {
                let target = cols[1].to_string();
                let file_path = cols[2].to_string();
                let owner_lead = cols[3].to_string();
                let worker = cols[4].to_string();
                let description = cols[5].to_string();

                if !target.is_empty() && !file_path.is_empty() {
                    entries.push(OwnerEntry {
                        target,
                        file_path,
                        owner_lead,
                        worker,
                        description,
                    });
                }
            }
        }
    }
    entries
}

fn find_owners_file() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("src/engine/OWNERS.md")
}

#[test]
fn test_w1_owners_table_parses() {
    let owners_path = find_owners_file();
    assert!(owners_path.is_file(), "OWNERS.md must exist at {}", owners_path.display());

    let content = std::fs::read_to_string(&owners_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", owners_path.display()));

    let entries = parse_owners_table(&content);
    assert!(!entries.is_empty(), "OWNERS.md table must contain entries");

    // Must cover all 12 W1 targets: E1..E4, K1..K4, P1..P4
    let expected_targets = [
        "E1", "E2", "E3", "E4",
        "K1", "K2", "K3", "K4",
        "P1", "P2", "P3", "P4",
    ];

    for expected in expected_targets {
        let found = entries.iter().any(|e| e.target == expected);
        assert!(found, "Target {expected} missing in OWNERS.md");
    }

    // Verify all file paths start with crates/quaqua/
    for entry in &entries {
        assert!(
            entry.file_path.starts_with("crates/quaqua/"),
            "entry {} path {} does not start with crates/quaqua/",
            entry.target,
            entry.file_path
        );
    }
}

/// Enforced in W2 after all 12 pipeline targets land their files.
/// Run post-W2 with:
/// `cargo test --locked -p quaqua -- --ignored test_w1_owners_files_exist`
#[test]
#[ignore = "enforced in W2 after all files are created: cargo test --locked -p quaqua -- --ignored test_w1_owners_files_exist"]
fn test_w1_owners_files_exist() {
    let owners_path = find_owners_file();
    let content = std::fs::read_to_string(&owners_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", owners_path.display()));

    let entries = parse_owners_table(&content);
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("could not find workspace root from CARGO_MANIFEST_DIR");

    let mut missing = Vec::new();
    for entry in entries {
        let path = workspace_root.join(&entry.file_path);
        if !path.exists() {
            missing.push(format!("Target {} missing file: {}", entry.target, entry.file_path));
        }
    }

    if !missing.is_empty() {
        panic!("Missing W1 target files:\n{}", missing.join("\n"));
    }
}

// ----------------------------------------------------------------------------
// Overlapping Timers and Deadlock Tests
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct OverlappingTimersState {
    fired_timers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct OverlappingTimersWorkflow;

impl Workflow for OverlappingTimersWorkflow {
    type Input = ();
    type State = OverlappingTimersState;
    type Output = Vec<String>;

    fn step(
        &self,
        state: &mut Self::State,
        ctx: &mut WorkflowContext,
        event: Option<WorkflowEvent>,
    ) -> StepDecision<Self::Output> {
        match event {
            None => {
                // Schedule overlapping timers via context scheduling surface
                ctx.schedule_timer("timer_fast", Duration::from_millis(100));
                ctx.schedule_timer("timer_coincident", Duration::from_millis(100));
                ctx.schedule_timer("timer_slow", Duration::from_millis(200));
                StepDecision::Continue
            }
            Some(WorkflowEvent::TimerFired { id }) => {
                state.fired_timers.push(id);
                if state.fired_timers.len() == 3 {
                    StepDecision::Complete(state.fired_timers.clone())
                } else {
                    StepDecision::Continue
                }
            }
            _ => StepDecision::Fail {
                code: 1,
                reason: "unexpected event".to_string(),
            },
        }
    }
}

#[test]
fn test_overlapping_timers_all_delivered() {
    let workflow = OverlappingTimersWorkflow;
    let mut clock = VirtualClock::new(0);
    let mut activities = MockActivities::new();
    let mut journal = InMemoryJournal::new();

    let initial_state = OverlappingTimersState {
        fired_timers: Vec::new(),
    };

    let result = run_to_completion(
        &workflow,
        "test-overlapping-timers",
        initial_state,
        &mut clock,
        &mut activities,
        &mut journal,
    );

    assert!(result.is_ok(), "workflow should complete: {result:?}");
    let fired = result.unwrap();
    assert_eq!(fired.len(), 3, "expected all 3 timers to be delivered");
    assert!(fired.contains(&"timer_fast".to_string()));
    assert!(fired.contains(&"timer_coincident".to_string()));
    assert!(fired.contains(&"timer_slow".to_string()));
    assert_eq!(fired[2], "timer_slow");
    assert_eq!(clock.now_ms(), 200);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DeadlockWorkflow;

impl Workflow for DeadlockWorkflow {
    type Input = ();
    type State = ();
    type Output = ();

    fn step(
        &self,
        _state: &mut Self::State,
        _ctx: &mut WorkflowContext,
        _event: Option<WorkflowEvent>,
    ) -> StepDecision<Self::Output> {
        // Continues without scheduling any timer or activity
        StepDecision::Continue
    }
}

#[test]
fn test_deadlock_detection_when_no_pending_events() {
    let workflow = DeadlockWorkflow;
    let mut clock = VirtualClock::new(0);
    let mut activities = MockActivities::new();
    let mut journal = InMemoryJournal::new();

    let result = run_to_completion(
        &workflow,
        "test-deadlock",
        (),
        &mut clock,
        &mut activities,
        &mut journal,
    );

    assert!(
        matches!(result, Err(TestkitError::Deadlock(_))),
        "expected deadlock, got: {result:?}"
    );
}
