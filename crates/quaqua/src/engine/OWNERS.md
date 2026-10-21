# W1 File Ownership Table

This table defines the strict file-level ownership for the 12 parallel W1 pipelines.
Every file has exactly one owning lead and pipeline.
Enforced in W2 with: `cargo test --locked -p quaqua -- --ignored test_w1_owners_files_exist`.

| Target | File Path | Owner (Lead) | Pipeline / Worker | Scope / Description |
|---|---|---|---|---|
| E1 | crates/quaqua/src/engine/journal_sqlite.rs | engine-lead | sude-arch-e1 | SQLite journal (rusqlite bundled, WAL mode, busy_timeout=5000) |
| E2 | crates/quaqua/src/engine/runner.rs | engine-lead | sude-arch-e2 | Workflow runner: load snapshot, replay, step, activity execution, append |
| E3 | crates/quaqua/src/engine/activities.rs | engine-lead | sude-arch-e3 | Real activity implementations over herdr.rs, git.rs, and fs |
| E3 | crates/quaqua/src/engine/clock.rs | engine-lead | sude-arch-e3 | System real-time clock implementation for production runtime |
| E4 | crates/quaqua/src/engine/telemetry_otel.rs | engine-lead | sude-arch-e4 | OpenTelemetry GenAI spans recorder writing to runs/traces.jsonl |
| K1 | crates/quaqua/src/worker/prompt/workflow.rs | worker-lead | sude-arch-k1 | SingleWorkerWorkflow state machine with budget, grace, autoext, stall timers |
| K1 | crates/quaqua/src/worker/prompt/mod.rs | worker-lead | sude-arch-k1 | Prompt worker entry point and workflow runner integration |
| K1 | crates/quaqua/src/worker/prompt/watchdog.rs | worker-lead | sude-arch-k1 | Watchdog timer state machine adaptation |
| K2 | crates/quaqua/src/herdr.rs | worker-lead | sude-arch-k2 | Blocking herdr agent wait + synchronous prompt land check |
| K3 | crates/quaqua/src/worker/prompt/cost.rs | worker-lead | sude-arch-k3 | Cost tracking activity (cost80/cost100 and report mtime checks) |
| K3 | crates/quaqua/src/worker/prompt/observer.rs | worker-lead | sude-arch-k3 | Glyph-free report readiness and Lead/pipeline liveness |
| K4 | crates/quaqua/src/taskforce/mod.rs | worker-lead | sude-arch-k4 | TaskforceWorkflow implementation (quorum, dual-lens, advisor) |
| P1 | crates/quaqua/src/pipeline/mod.rs | pipe-lead | sude-arch-p1 | PipelineWorkflow: plan gate, build child, review child, fix cycle |
| P1 | crates/quaqua/src/pipeline/state.rs | pipe-lead | sude-arch-p1 | Pipeline state transitions and durable event recovery |
| P1 | crates/quaqua/src/pipeline/phases/mod.rs | pipe-lead | sude-arch-p1 | Phase execution abstractions and common helpers |
| P1 | crates/quaqua/src/pipeline/phases/build.rs | pipe-lead | sude-arch-p1 | Build phase child execution and outcome inspection |
| P1 | crates/quaqua/src/pipeline/phases/fix.rs | pipe-lead | sude-arch-p1 | Fix phase child execution and retry handling |
| P1 | crates/quaqua/src/pipeline/phases/plan.rs | pipe-lead | sude-arch-p1 | Plan phase child execution and plan gate |
| P1 | crates/quaqua/src/pipeline/phases/review.rs | pipe-lead | sude-arch-p1 | Review phase child execution and verdict evaluation |
| P2 | crates/quaqua/src/pipeline/phases/spawn.rs | pipe-lead | sude-arch-p2 | Spawn phase: removal of sync grid call in favour of event repair |
| P2 | crates/quaqua/src/grid/mod.rs | pipe-lead | sude-arch-p2 | Grid activity entry point and layout repair driven by events |
| P2 | crates/quaqua/src/grid/apply.rs | pipe-lead | sude-arch-p2 | Grid pane split and movement actions |
| P2 | crates/quaqua/src/grid/hook.rs | pipe-lead | sude-arch-p2 | Grid hook invocation helpers |
| P2 | crates/quaqua/src/grid/plan.rs | pipe-lead | sude-arch-p2 | Grid placement calculation and layout planning |
| P2 | crates/quaqua/src/grid/reconcile.rs | pipe-lead | sude-arch-p2 | Grid reconciliation algorithm |
| P2 | crates/quaqua/src/grid/types.rs | pipe-lead | sude-arch-p2 | Grid layout data types and structures |
| P3 | crates/quaqua/src/status.rs | pipe-lead | sude-arch-p3 | quaqua status query reading runs/quaqua.db (issue #47) |
| P4 | crates/quaqua/tests/worker_unit.rs | pipe-lead | sude-arch-p4 | Migration of HerdrOps mocks to MockActivities |
| H2 | crates/quaqua/src/harness/mod.rs | worker-lead | cos-h2 | Harness registry: names, effort sets, stop keys, fallback idle (harness-hygiene.md H2) |
