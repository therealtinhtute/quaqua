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

## Steps: engine core (to-plan 2026-10-19, parallel)
Driver: one Lead `arch-lead` (8 task workers = 4 pipelines, the cap). Peers branch from `quaqua/arch-integ`; each team = sude build + suhuynh review, one review round. CoS re-runs every Verify; anh iu approves the W0 contract before W1 opens, and every push/merge main.
Waves: W0 (alone) → W1 (4 teams in parallel, one owner per directory) → W2 (integrate). Each wave ends with tests green on `quaqua/arch-integ`.

### W0: freeze the contract (blocks W1, ~1h)
- `crates/quaqua/src/engine/` API only — types, traits, docs, `todo!()` bodies; no callers:
  - `Workflow` trait: `step(&self, state, ctx, event) -> StepDecision` (Continue · ScheduleActivity · ScheduleTimer · StartChild · PauseForApproval · Complete · Fail). `WorkflowContext` exposes only scheduling + `now()` — no fs/net/process.
  - `Activity` trait + typed requests/results for herdr, git, grid, report-file.
  - `Journal` trait; SQLite schema `events(workflow_id, seq, ts_ms, kind, payload_json)` + `snapshots(workflow_id, seq, state_json)`, DB `<workspace_root>/runs/quaqua.db`.
  - Testkit API: `VirtualClock`, `MockActivities`, in-memory `Journal`, `run_to_completion(workflow, input)`. The in-memory pieces are implemented here (small) so W1 teams test without team A.
  - Child-workflow contract: `SingleWorkerInput {name, kind, repo, brief, budget_sec, cost_budget}` → `SingleWorkerOutput {exit: 0|2..6, report_path}`; D depends on this, not on C's code.
- Verify: `cargo build --locked` + `cargo test --locked` green; a toy workflow runs to completion on the in-memory testkit; determinism-guard test fails on `std::(fs|process|net)` inside any `*Workflow` impl. Anh iu reviews the trait file.

### W1: four teams in parallel (2.5–4h wall)
- **A — engine impl** (`src/engine/` impls): `rusqlite` (`bundled`, pinned `=`), WAL + `busy_timeout=5000`, ephemeral runner (load snapshot → replay → step → run activity → append → exit when blocked), real clock.
  Verify: `cargo test --locked engine::` — kill mid-run → resume gives the same state; 2 concurrent writers don't corrupt; 7200 s timer under `VirtualClock` <50 ms.
- **B — taskforce** (`taskforce/`): `TaskforceWorkflow` replaces the 274-LoC loop; cap/quorum state into workflow state.
  Verify: testkit tests for quorum + one worker failing; `bash tests/test_taskforce_cap.sh` PASS.
- **C — worker prompt/watchdog** (`worker/prompt/`): `SingleWorkerWorkflow` with durable timers (budget, grace, autoext, stall 600 s) + herdr `events.subscribe` (`pane.agent_status_changed`, `pane.exited`) + report mtime as activities. Drop ANSI scraping in `observer.rs` and the `.{name}.*` dotfiles. Exit codes 2–6 and `--resume/--extend` stay byte-identical.
  Verify: testkit test per exit code 2–6 under virtual time; `bash tests/test_prompt_worker.sh` + `tests/test_prompt_worker_watchdog.sh` PASS.
- **D — pipeline** (`pipeline/`): `PipelineWorkflow` = builder child → `PauseForApproval` plan gate → reviewer child → fix round, children mocked per the W0 contract; replaces the `pipeline/state.rs` key=value parser; grid repair as an activity; OTel GenAI-shaped spans to `runs/traces.jsonl`.
  Verify: testkit tests for plan gate, review fail → fix, resume at the same phase; `cargo test --locked --test pipeline_prompt` PASS.
- A contract change during W1 goes through the Lead, lands on `arch-integ` first, and every team rebases — no team edits `src/engine/` API except A via the Lead.

### W2: integrate + cleanup (1.5–2h)
- Merge A–D on `arch-integ`; wire B/C/D to A's SQLite journal + real activities.
- Replace the 6 ad-hoc `HerdrOps` mocks in `tests/worker_unit.rs` with `MockActivities`; remove dead dotfile code in `state.rs`.
- `quaqua status` (issue #47 P2) reads `runs/quaqua.db`; WAITER-LOST = a workflow blocked with no live waiter.
- Verify: `bash tests/run.sh --slow` all PASS; `rg -n '\.budget|\.grace|\.autoext' crates/quaqua/src` empty; one live spawn → prompt → cleanup and one live pipeline on herdr, plus kill → `--resume`.

### Then
The repo split (Phase 2 above) runs after W2, so `quaqua-rs` starts on the new core.
