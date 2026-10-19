# quaqua-rs split — LOCK (2026-10-16)

Status: locked, awaiting approval → to-plan

## Goal
The Rust engine lives in its own repo `~/Lab/quaqua-rs`. `~/quaqua` holds only the workspace and uses the installed `quaqua` binary. The workspace is the md files, memory, runs, rules, team.toml and the hooks.

## Decisions (anh iu, 2026-10-16)
- Repo: `~/Lab/quaqua-rs`
- History: keep it with `git subtree split` (blame and PR #42 stay traceable)
- Grid: port `grid-workers.sh` to Rust before the split
- Order: lock the plan and run to-plan for both phases first, then build

## Phase 1: decouple in place (no behaviour change, ships alone)
- Move every hardcoded workspace path into one `workspace_root()`. The default stays `~/quaqua`, and one override (env or team.toml) can change it:
  - `crates/quaqua/src/worker/spawn.rs:257` (rules)
  - `crates/quaqua/src/worker/cleanup.rs:12` (runs)
  - `crates/quaqua/src/config.rs:262` and `crates/quaqua/src/main.rs:183` (team.toml)
- Port `grid-workers.sh` to `quaqua grid`. The binary then stops searching for the script (`cleanup.rs:37-80`), and pipeline/spawn/cleanup call the Rust grid.
- Done when:
  - `rg 'join\("quaqua/' crates/` finds only the default in `workspace_root()`
  - no Rust code shells out to `grid-workers.sh`
  - cargo tests pass, and the grid tests and the targeted bash suites pass

## Phase 2: split the repo
- Move `crates/`, `Cargo.*` and the engine tests to `~/Lab/quaqua-rs` with `git subtree split`, keeping history.
- Install from the new repo with `cargo install --path ~/Lab/quaqua-rs/crates/quaqua`.
- In `~/quaqua`:
  - `.claude/settings.json` calls `quaqua gate` directly
  - `scripts/*` become thin shims or are removed
  - `crates/` is removed in a reviewed commit
- Split tests by owner: engine tests go with the code, workspace and docs tests stay in `~/quaqua`.
- Register `quaqua-rs` in `projects/INDEX.md` with path `~/Lab/quaqua-rs`.
- Done when a fresh `cargo install` from `~/Lab/quaqua-rs` plus `~/quaqua` with no `crates/` passes:
  - a worker spawn → prompt → cleanup smoke run
  - the gate hook

## Steps (to-plan 2026-10-16)
Driver: the Lead `quaqua-rs-lead` (`quaqua lead spawn quaqua-rs`) runs the pipeline in Peer worktrees from `quaqua/quaqua-rs-integ`. The CoS verifies, and anh iu approves every push and every merge to main.

### P1: decouple in place
1. **workspace_root**
   - Builder: a Peer (tieude).
   - Change: add one resolver. It reads `QUAQUA_HOME` first, then `$HOME/quaqua`.
   - Route 4 call sites through it: `spawn.rs:257`, `cleanup.rs:12`, `config.rs:262`, `main.rs:183`.
   - Verify:
     - `rg -n 'join\("quaqua/' crates/quaqua/src` finds only the resolver
     - a new unit test covers the env override and the default
     - `cargo test --locked` passes
2. **grid → Rust**
   - Builder: a Peer (tieude).
   - Change: give `quaqua grid reconcile` the full behaviour of `grid-workers.sh`, including `--only`, `--tab` and `--dry-run`. Then point every caller of `find_grid_script`/`run_grid_hook` at it: spawn, cleanup and pipeline (`pipeline/mod.rs:667-1230`).
   - Verify:
     - `rg -n 'grid-workers.sh' crates/quaqua/src` finds nothing
     - the 25 grid fixtures pass the differential test
     - `bash tests/test_pipeline_grid.sh` and `tests/test_spawn_worker.sh` pass
     - one live spawn→cleanup run on herdr arranges the panes correctly
3. **shim**
   - Change: `scripts/worker/grid-workers.sh` becomes a thin `exec quaqua grid reconcile "$@"`.
   - Verify: `bash tests/run.sh --slow` gives 25 pass / 0 fail (3 skip allowed).
4. **gate P1**
   - Review: suhuynh reviews the integ branch.
   - Then: PR integ→main, anh iu approves the merge, reinstall the binary.
   - P1 is done here. The system works even if P2 never lands.

### P2: split the repo
5. **subtree**
   - Run `git subtree split --prefix=crates -b quaqua-rs-export` on main.
   - Create `~/Lab/quaqua-rs`: root `Cargo.toml` (workspace), `crates/quaqua` from the export, `.github/workflows/ci.yml`, the engine shell tests.
   - Verify:
     - `git log --follow` on one file shows history from before PR #42
     - `cargo test --locked` passes in the new repo
6. **remote**
   - Create a GitHub repo for quaqua-rs and push. This step is outward-facing: ask anh iu for the name, private/public, and approval to push.
7. **consume**
   - `cargo install --path ~/Lab/quaqua-rs/crates/quaqua --locked`.
   - In `~/quaqua`: `.claude/settings.json` calls `quaqua gate` directly.
   - `scripts/*` stay as shims onto the binary.
   - Verify: the gate hook still blocks a direct `herdr pane split` and allows `quaqua worker spawn`.
8. **cut**
   - Remove `crates/`, `Cargo.*`, and the engine tests moved in step 5 from `~/quaqua`. Use a reviewed PR, delete with `git rm`, and give anh iu the list first.
   - Verify:
     - `~/quaqua` contains no `.rs` files
     - `bash tests/run.sh` (the workspace suites) is green
     - a spawn → prompt → cleanup smoke run of a small worker succeeds
9. **register**
   - `projects/INDEX.md`: point quaqua-rs at path `~/Lab/quaqua-rs`.
   - Update `AGENTS.md`, `rules/_all/fanout-agent-grid.md` and `rules/_all/hooks.md`: say the binary comes from `~/Lab/quaqua-rs`, and drop the `scripts/*.sh` names where nothing uses them any more.
   - Rollback: `cargo install` from a `~/quaqua` commit taken before step 8.

## Fragile assumption
Every workspace path the binary uses goes through `workspace_root()`. If one is missed, the binary silently reads or writes `~/quaqua`. The Phase 1 rg check is the guard.

## Out of scope
- New features
- Publishing to crates.io
- Changing CI beyond what `tieude-ci` adds

## Engine core decisions (anh iu, 2026-10-19 · raw/2026-10-19-quaqua-archresearch.md)
- Core: embedded event-sourced workflow engine (6 layers: runtime · journal · activities · patterns · CLI/OTel · testkit).
- Journal: SQLite (`rusqlite` bundled, WAL) replaces `runs/.{name}.*` dotfiles.
- Process: ephemeral CLI, no daemon.
- Patterns: Rust `Workflow` trait impls; declarative TOML deferred.
- Migration: strangler fig in `crates/quaqua` — taskforce → worker/prompt → pipeline, tests green each commit; before the repo split.

## Steps: engine core (to-plan 2026-10-19, 3 Leads × 8)
Drivers: `engine-lead`, `worker-lead`, `pipe-lead` — 4 pipelines each (sude build + suhuynh review, one round) = 24 task workers. Peers branch from `quaqua/arch-integ`. One owner per **file** (listed below); nobody else edits it. `engine-lead` owns `arch-integ` and runs W2. CoS re-runs every Verify; anh iu approves the W0 contract, every push and merge to main.
Critical path: W0 (~1h) → anh iu approves → W1 (12 pipelines, longest ~2h) → W2 (~2h). Target: same day.

### W0: freeze the contract (`engine-lead`, one pipeline, blocks W1)
- `crates/quaqua/src/engine/` API only (types, traits, docs, `todo!()` bodies; no callers):
  - `workflow.rs`: `Workflow` trait `step(&self, state, ctx, event) -> StepDecision` (Continue · ScheduleActivity · ScheduleTimer · StartChild · PauseForApproval · Complete · Fail); `WorkflowContext` = scheduling + `now()` only.
  - `activity.rs`: `Activity` trait + typed request/result enums for herdr, herdr events, git, grid, report-file, cost.
  - `journal.rs`: `Journal` trait; SQLite schema `events(workflow_id, seq, ts_ms, kind, payload_json)` + `snapshots(workflow_id, seq, state_json)`; DB `<workspace_root>/runs/quaqua.db`.
  - `telemetry.rs`: `SpanSink` trait, OTel GenAI field list.
  - `testkit.rs` (implemented, small): `VirtualClock`, `MockActivities`, in-memory `Journal`, `run_to_completion`.
  - Child contract: `SingleWorkerInput {name, kind, repo, brief, budget_sec, cost_budget}` → `SingleWorkerOutput {exit: 0|2..6, report_path}`.
  - File-ownership table for W1 checked into `engine/OWNERS.md`; a test fails if a W1 file listed there is missing after W2.
- Verify: `cargo test --locked` green; toy workflow runs on the testkit; determinism-guard test fails on `std::(fs|process|net)` in any `*Workflow` impl. **Gate: anh iu reviews `engine/` API before W1.**

### W1: 12 pipelines in parallel
Peers verify only their own module: `cargo test --locked --lib <module>::` (testkit, no herdr, no bash suites, no full `cargo test`). Integration and live checks belong to the test team in W2.
**engine-lead** (`src/engine/`)
- E1 `journal_sqlite.rs`: `rusqlite` (`bundled`, pinned `=`), WAL, `busy_timeout=5000`. Verify: 2 concurrent writers, crash between append and snapshot recovers.
- E2 `runner.rs`: load snapshot → replay → step → run activity → append → exit when blocked. Verify: kill mid-run → resume = same state; 7200 s timer <50 ms on `VirtualClock`.
- E3 `activities.rs` + `clock.rs`: real impls over `herdr.rs`/`git.rs`. Verify: unit tests with a `HERDR_BIN_PATH` stub.
- E4 `telemetry_otel.rs`: spans to `runs/traces.jsonl`. Verify: golden-file test of one span.

**worker-lead** (`src/worker/prompt/`, `src/herdr_events.rs`, `src/taskforce/`)
- K1 `worker/prompt/workflow.rs`: `SingleWorkerWorkflow` timers (budget, grace, autoext, stall 600 s) → exit codes 2–6. Verify: one testkit test per exit code.
- K2 `herdr_events.rs`: `events.subscribe` activity (`pane.agent_status_changed`, `pane.exited`). Verify: stub socket test.
- K3 `worker/prompt/cost.rs` + report activity: report mtime + cost80/100 as activity results; drop ANSI scraping in `observer.rs`. Verify: testkit tests for cost80/100 + report-ready.
- K4 `taskforce/mod.rs`: `TaskforceWorkflow`. Verify: quorum + one-fail testkit tests.

**pipe-lead** (`src/pipeline/`, `src/grid/`, `src/status.rs`, `tests/worker_unit.rs`)
- P1 `pipeline/mod.rs` + `pipeline/state.rs`: `PipelineWorkflow` (builder child → plan gate → reviewer child → fix), children mocked per the W0 contract. Verify: plan gate, review fail → fix, resume at phase.
- P2 `grid/` activity: grid repair driven by events, no sync call in phases/spawn. Verify: unit tests on layout decisions.
- P3 `status.rs` (#47 P2): `quaqua status` reads `runs/quaqua.db`; WAITER-LOST flag. Verify: fixture DB tests.
- P4 `tests/worker_unit.rs`: replace the 6 ad-hoc `HerdrOps` mocks with `MockActivities`. Verify: compiles; test team runs it in W2.

Contract changes during W1: only via `engine-lead`, landed on `arch-integ` first; every team rebases.

### W2: integrate (`engine-lead`) + one test team
- Merge the 12 branches into `arch-integ` in batches; wire K/P workflows to E1–E3; delete the `.{name}.*` dotfile code in `state.rs`.
- Test team `tester` (1 worker, tieude, own worktree on `arch-integ`) runs the full suites once per merge batch and files failures back to the owning Lead: `cargo test --locked`, `test_taskforce_cap.sh`, `test_prompt_worker*.sh`, `test_pipeline*.sh`, `--test pipeline_prompt`, `--test worker_unit`, live herdr probe.
- Done when: `bash tests/run.sh --slow` all PASS; `rg -n '\.budget|\.grace|\.autoext' crates/quaqua/src` empty; live spawn → prompt → cleanup, live pipeline, kill → `--resume`.

### Risks
- Provider concurrency (12 omp + 12 opencode at once) is unverified; throttling stalls workers → CoS checks the first 10 min and drops to 2 Leads if more than 3 stall.
- W2 conflicts if W0 under-specifies; mitigated by the contract tests and file ownership.

### Then
Repo split (Phase 2 above) after W2, so `quaqua-rs` starts on the new core.
