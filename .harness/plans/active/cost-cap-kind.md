---
status: active
lane: normal
---
# Per-kind cost-cap action (issue #48)

## Goal
- outcome: a kind can choose what the watchdog does when a worker reaches 100% of its cost budget: keep warning (default) or interrupt the worker and stop waiting, so the CoS decides with the pane and worktree intact.
- success_signal: `cargo test --locked` passes with new config and watchdog tests; a `kill` kind at 100% gets one `worker stop` interrupt and a distinct watchdog exit, while a `warn` kind behaves exactly as today.
- actors: CoS and Leads (read the exit), workers (interrupted), anh iu (sets `team.toml`).
- authority: issue #48; owner decisions 2026-10-23 (configurable per kind; kill = interrupt + stop waiting; threshold 100%); stacked on PR #121 (`CostTracker::observe`).
- requirements:
  - R1: `[kinds.<k>].on_cost_cap = "warn" | "kill"`, default `"warn"`; any other value fails config load with a clear error. | acceptance: config tests: missing → warn, kill parses, "pause" rejected | source: owner decision
  - R2: on a `kill` kind, the first check at or over 100% sends the harness interrupt from `worker stop` (never ctrl+c) once, prints `WATCHDOG cost 100% | … | STOP`, and ends `worker prompt` with a dedicated exit code; pane, worktree and journal stay intact so `--resume` can continue. | acceptance: watchdog test with mocked spend and herdr: one interrupt, the exit code, no cleanup call; a restart after the stop does not interrupt again | source: owner decision "interrupt + dừng chờ"
  - R3: a `warn` kind is unchanged: 80/100% warn plus the 200% ESCALATE mark from #121, never interrupted. | acceptance: existing cost tests stay green, plus one warn-kind test asserting no interrupt at 250% | source: #48 Q2
  - R4: the exit code and `on_cost_cap` are documented where the CoS reads watchdog exits and in `templates/workspace/team.toml` (or wherever kinds are templated). | acceptance: grep of `on_cost_cap` in the template and in the exit-code docs | source: AGENTS.md "trust runs file"
- non-goals:
  - NG1: unifying `quaqua usage` with `read_worker_spend` (#122).
  - NG2: closing panes or removing worktrees on cost.
  - NG3: kill at 200% or per-kind thresholds.

## Phases and Verification
- approach: extend `Kind` like `spawn` (const list + default + validation); `CostTracker` learns the action and reports a stop request; the watchdog turns it into one interrupt plus a terminal outcome. Rejected: a global flag (owner chose per kind) and full cleanup (loses uncommitted work).
- phase_slug: `main`
  status: checked
  - goal: R1, R2, R3, R4 | depends_on: PR #121
  - surfaces: `crates/quaqua/src/config.rs`, `worker/prompt/{cost,mod,workflow}.rs`, `worker/stop.rs` (reuse only), `state.rs`, templated `team.toml`, exit-code docs | avoided: `gate/`, `grid/`, `pipeline/`, `usage.rs`
  - phase check: `cargo test --locked` plus the `on_cost_cap` greps

## Log

- 2026-10-23T14:49:56Z — main — plan — drafted from owner answers; stacked on quaqua/blindside-absence (b22dafc)
- 2026-10-23T15:04:50Z — main — start — owner locked ("Lock, tui tự build"); worktree ~/Lab/.wt/quaqua-costcap, branch quaqua/cost-cap-kind on quaqua/blindside-absence
- 2026-10-23T15:04:50Z — main — decision — R2 "dedicated exit" is exit 5 with reason "cost cap reached (on_cost_cap=kill)…": a new code would break the engine/child exit contract (0, 2..6, `engine/child.rs`) and the pipeline; the activity rewrites the AgentWait status to `blocked` after one `worker stop` interrupt
- 2026-10-23T15:04:50Z — main — done — R1 `on_cost_cap` warn|kill, default warn, unknown rejected | check: `cargo test --lib on_cost_cap` pass | source: config.rs
- 2026-10-23T15:04:50Z — main — done — R2 kill kind: one interrupt, `| STOP`, exit 5, no cleanup | check: red tracker tests → `cargo test --lib cost` 32/32; `tests/test_prompt_worker_watchdog.sh` sude-costkill PASS | source: worker/prompt/{cost,mod}.rs
- 2026-10-23T15:04:50Z — main — done — R3 warn kind unchanged | check: `test_warn_kind_never_requests_stop`; sude-cost case PASS
- 2026-10-23T15:04:50Z — main — done — R4 docs | check: `grep -n on_cost_cap templates/workspace/team.toml templates/workspace/rules/_all/worker-timebox.md templates/workspace/AGENTS.md` 3 hits; `cargo test --locked init` pass
- 2026-10-23T15:04:50Z — main — check — `cargo test --locked` 624 passed 0 failed; all 17 `tests/test_*.sh` PASS (watchdog suite regression from #121 fixed on that branch, e7051e1)
- 2026-10-23T15:17:15Z — main — done — review request: watchdog stdout kept clean (`stop::interrupt_worker` returns the line, logged on stderr) | check: `cargo test --locked` 624 passed; 17 bash suites PASS; sude-costkill asserts empty stdout

## Validation

- 2026-10-23T15:17:15Z — phase `main` — verdict: APPROVE_WITH_REQUESTS — mode: full
  - `cargo test --locked` — 624 passed, 0 failed; all 17 `tests/test_*.sh` PASS
  - requirements: R1 met | R2 met (exit 5 by logged decision) | R3 met | R4 met; NG1–NG3 untouched
  - requests: 1. `stop_worker` println leaked to watchdog stdout — fixed
  - rollback_point: e7051e1
  - judge: independent (runs/sude-costrev.md)
  - judge_model: google-antigravity/gemini-3.8-flash (omp)

## Current State and Next Action
- active_phase: main
- lifecycle_status: checked
- blockers: none
- open_items: none
- exact_next_action: commit, push, stacked PR on #121
