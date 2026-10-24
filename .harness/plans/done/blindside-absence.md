---
status: done
lane: normal
---
# Blindside escalation + absence ≠ broken (issue #69)

## Goal
- outcome: herdr verbs never treat one failed lookup as "agent is down"; a worker spending ≥2× its budget raises an escalation line the CoS relays to anh iu; AGENTS.md rule 3 carries the judgment-only blindside test.
- success_signal: `cargo test --locked` passes with new tests for the liveness helper and the 2× cost mark; `grep -n blindside templates/workspace/AGENTS.md` returns the rule-3 line.
- actors: CoS, Leads, workers (callers of `quaqua worker/lead` verbs); anh iu (receives escalations).
- authority: issue #69; owner decision 2026-10-23 (hybrid "hướng C": tooling where a machine signal exists, one text line for judgment); existing pattern `crates/quaqua/src/lead/cleanup.rs:15-70`.
- requirements:
  - R1: one shared liveness helper returns `Alive | Down | Unknown`; `Down` only when `agent_not_found` is confirmed by a second independent check (`herdr agent list` + pane read); timeout, empty list or other errors → `Unknown`. | acceptance: unit tests over mocked `HerdrOps` cover alive, confirmed-down, not_found-but-listed (→ Alive/Unknown), timeout (→ Unknown), empty list (→ Unknown) | source: #69 "absence ≠ broken"
  - R2: `lead cleanup`, `worker cleanup`, `worker prompt` and `lead prompt` use the helper; on `Unknown` they refuse to act and exit non-zero with "cannot determine state" instead of respawning/closing/reporting down. | acceptance: one test per verb with an `Unknown` mock asserting non-zero exit and no close/spawn call | source: #69; `cleanup.rs:20`
  - R3: the watchdog cost tracker adds a 200% mark: once per worker, records it in worker metadata and prints `WATCHDOG cost 200% | name=… cost=… budget=… | ESCALATE` to stderr; it never kills the worker. | acceptance: `CostTracker`/`CostState` tests: 199% emits no 200% line, 200% emits exactly once across repeated checks and restarts (metadata) | source: #69 "cost >2× budget"; `worker/prompt/cost.rs:906`
  - R4: `templates/workspace/AGENTS.md` rule 3 gains one line: a goal/scope change, red main, or a decision taken on anh iu's behalf → tell anh iu now; anything else → decide and log in today's memory. | acceptance: `grep -n blindside templates/workspace/AGENTS.md` matches; template tests (`cargo test --locked init`) pass | source: #69 "blindside test"
  - R5: the seed rule "Spawn checks — agent_not_found is not failure" in `templates/seed/memory-rules.md` is removed, since R1–R2 enforce it. | acceptance: `grep -c agent_not_found templates/seed/memory-rules.md` = 0 | source: owner preference "tooling over text rules" (#55)
- non-goals:
  - NG1: killing or pausing a worker on cost (warn vs kill stays open in #48).
  - NG2: detecting red main automatically (judgment line only in R4).
  - NG3: a new `rules/_all/` file (would bloat every worker prompt).
  - NG4: farm-layouts mail/events (PR #120); the 200% line moves to `farm_events` when P2 lands.

## Phases and Verification
- approach: hybrid — shared liveness helper + 200% cost mark in code, one rule-3 line for judgment; rejected text-only (owner prefers tooling) and tooling-only (scope/decision changes are not machine-detectable).
- phase_slug: `main`
  status: checked
  - goal: R1, R2, R3, R4, R5 | depends_on: none
  - surfaces: `crates/quaqua/src/herdr.rs` (or new `liveness.rs`), `lead/cleanup.rs`, `lead/prompt.rs`, `worker/cleanup.rs`, `worker/prompt/{cost,workflow}.rs`, `state.rs`, `templates/workspace/AGENTS.md`, `templates/seed/memory-rules.md` | avoided: `gate/`, `grid/`, `pipeline/`, `engine/journal_sqlite.rs`
  - phase check: `cargo test --locked && cargo fmt --check && grep -n blindside templates/workspace/AGENTS.md && test "$(grep -c agent_not_found templates/seed/memory-rules.md)" = 0`

## Log

- 2026-10-23T13:17:21Z — main — start — worktree ~/Lab/.wt/quaqua-blindside, branch quaqua/blindside-absence from origin/main d49e93d
- 2026-10-23T13:42:56Z — main — decision — R1 second check uses `agent list` instead of `pane read`: a not_found agent has no pane id to read
- 2026-10-23T13:42:56Z — main — done — R1 liveness helper | check: `cargo test --lib liveness` 6/6 | source: crates/quaqua/src/liveness.rs
- 2026-10-23T13:42:56Z — main — done — R2 wired into lead/worker cleanup + prompt | check: test_cleanup_lead.sh, test_cleanup_worker.sh, test_prompt_worker.sh PASS; lead_prompt 5/5 | source: lead/cleanup.rs, lead/prompt.rs, worker/cleanup.rs, worker/prompt/mod.rs
- 2026-10-23T13:42:56Z — main — decision — R3 lives in `CostTracker` only (live path, mod.rs:431/:927); `CostState`/`cost_warning` are test-only and left unchanged
- 2026-10-23T13:42:56Z — main — done — R3 200% ESCALATE mark, once, persisted in `.{name}.cost200` | check: red (no `observe`/`has_cost200`) → `cargo test --lib cost` 29/29 | source: worker/prompt/cost.rs, state.rs
- 2026-10-23T13:51:36Z — main — done — R4 blindside line appended to rule 3 | check: `grep -c blindside templates/workspace/AGENTS.md` = 1 | source: templates/workspace/AGENTS.md
- 2026-10-23T13:51:36Z — main — done — R5 seed rule "Spawn checks — agent_not_found" removed | check: `grep -c agent_not_found templates/seed/memory-rules.md` = 0 | source: templates/seed/memory-rules.md
- 2026-10-23T13:51:36Z — main — decision — `cargo fmt --check` is not a usable gate: main is not fmt-clean (no rustfmt.toml, CI has no fmt step); repo-wide churn reverted, only touched files formatted
- 2026-10-23T13:51:36Z — main — check — phase check: `cargo test --locked` 621 passed 0 failed; test_cleanup_lead/worker, test_prompt_worker PASS; next: `check full`
- 2026-10-23T14:20:45Z — main — decision — request 1 accepted as designed: an empty `agent list` stays Unknown (fail closed); the CoS itself runs as a herdr agent (live `herdr agent list` names it), so an empty list means herdr is unreliable, not that the worker is gone. No `--force` added; owner can request one
- 2026-10-23T14:20:45Z — main — done — request 2: 38 fmt-only hunks reverted in touched files | check: `cargo test --locked` 621 passed 0 failed; 3 bash tests PASS

## Validation

- 2026-10-23T13:59:55Z — phase `main` — verdict: APPROVE_WITH_REQUESTS — mode: full
  - `cargo test --locked` — 621 passed, 0 failed
  - `bash tests/test_cleanup_lead.sh && bash tests/test_cleanup_worker.sh && bash tests/test_prompt_worker.sh` — all PASS
  - `test "$(grep -c agent_not_found templates/seed/memory-rules.md)" = 0 && grep -q blindside templates/workspace/AGENTS.md` — pass
  - scope: on target — repo-wide fmt churn reverted; residual fmt-only hunks remain in touched files
  - requirements: R1 met (liveness 6/6) | R2 met (4 verbs + bash/integration tests) | R3 met (cost 29/29, red first) | R4 met (grep) | R5 met (grep = 0); success_signal met
  - rollback_point: d49e93d
  - requests: 1. worker/cleanup.rs liveness: an empty `agent list` (CoS outside herdr, last agent closed) is Unknown, so cleanup of a truly gone worker refuses with no override — decide accept or add `--force`; 2. drop fmt-only hunks in lib.rs, worker/cleanup.rs, worker/prompt/mod.rs, lead/cleanup.rs before PR; 3. open question: herdr notification on 200% ESCALATE
  - not_independently_verified: spec + standards review (author judged own diff)
  - judge: same-session
  - judge_model: claude-opus-5-5
- 2026-10-23T14:29:24Z — phase `main` — verdict: APPROVE_WITH_REQUESTS — mode: full
  - `cargo test --locked` — 621 passed, 0 failed (re-run after review fixes)
  - `bash tests/test_cleanup_lead.sh && bash tests/test_cleanup_worker.sh && bash tests/test_prompt_worker.sh` — all PASS
  - `test "$(grep -c agent_not_found templates/seed/memory-rules.md)" = 0 && grep -q blindside templates/workspace/AGENTS.md` — pass
  - scope: on target — lib.rs reorder reverted to one added line; residual fmt hunks are mixed with semantic edits
  - requirements: R1 met | R2 met | R3 met | R4 met (now case-exact grep) | R5 met; success_signal met
  - rollback_point: d49e93d
  - requests: 1. residual fmt-only wrapping inside mixed hunks (cosmetic, accepted); 2. empty agent list stays Unknown (accepted, see Log decision); 3. open question: herdr notification on 200% ESCALATE (owner)
  - judge: independent (runs/sude-blindrev.md)
  - judge_model: google-antigravity/gemini-3.8-flash (omp)

## Current State and Next Action
- active_phase: main
- lifecycle_status: checked
- blockers: none
- open_items: open_question: should the 200% ESCALATE line also trigger a herdr notification before farm-layouts P6? | owner: anh iu
- exact_next_action: commit, push branch, open PR (owner approved)
