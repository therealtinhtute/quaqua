---
status: done
lane: normal
---
# Stale-session guard (issue #72)

## Goal
- outcome: a Claude Code CoS/Lead learns, before it spawns or prompts, that team.toml, AGENTS.md or rules/_all/ changed since its session started, and gets a re-read list after context compaction.
- success_signal: `cargo test --locked` passes with new drift tests; the template settings.json registers the SessionStart hook.
- actors: CoS and Leads on Claude Code; anh iu (edits rules).
- authority: issue #72 (warn + re-read, never block); owner decisions 2026-10-23: hướng A (SessionStart hook + gate), tracked files = team.toml + rules/_all + AGENTS.md.
- requirements:
  - R1: `quaqua gate --session-start` reads the SessionStart hook JSON and writes a per-session baseline (path → md5) of team.toml, AGENTS.md, rules/_all/*.md under `runs/.sessions/<session_id>.json`; `resume` keeps an existing baseline. | acceptance: unit tests: startup writes, resume keeps, unsafe session id rejected | source: #72 open question
  - R2: on `source=compact` it prints a re-read list (AGENTS.md, memory/rules.md, projects/INDEX.md, newest reports/audit-*.md) to stdout, then rebaselines. | acceptance: unit test on the printed list | source: #72 shape
  - R3: `quaqua gate` (PreToolUse) on a `quaqua worker …` / `quaqua lead …` command whose verdict is allow compares the baseline; on drift it emits `additionalContext` + `systemMessage` naming the changed files, exits 0, and rebaselines (warn once per change). No baseline or no drift → no output. Never blocks. | acceptance: binary tests over stdin JSON: drift → JSON with the file name; no drift → empty stdout; exit 0 both | source: #72 decision
  - R4: template `.claude/settings.json` adds the SessionStart hook via `scripts/hooks/herdr-gate.sh --session-start`; the script's no-binary fallback exits 0 for it. | acceptance: grep + `cargo test --locked init` + `bash tests/test_herdr_gate.sh` | source: #72 shape
- non-goals:
  - NG1: blocking on drift.
  - NG2: non-Claude harnesses (omp/codex Leads).
  - NG3: tracking memory/rules.md for drift (re-read list only).

## Phases and Verification
- approach: hook + gate (hướng A); rejected compact-only (misses mid-session drift) and spawn-side check (no session id → shared baseline).
- phase_slug: `main`
  status: in-progress
  - goal: R1–R4 | depends_on: none
  - surfaces: `crates/quaqua/src/gate/{mod,drift}.rs`, `main.rs`, `tests/gate.rs`, `templates/workspace/.claude/settings.json`, `templates/workspace/scripts/hooks/herdr-gate.sh` | avoided: `worker/`, `lead/`, `pipeline/`
  - phase check: `cargo test --locked` + all `tests/test_*.sh`

## Log

- 2026-10-23T15:35:26Z — main — plan — locked by owner ("Lock, tui tự build"); worktree ~/Lab/.wt/quaqua-stale from origin/main
- 2026-10-24T00:00:00Z — main — done — R1–R4 drift.rs + gate warn + --session-start hook | check: `cargo test --locked` 627 passed 0 failed; 17 bash tests PASS
- 2026-10-24T00:00:00Z — main — done — review fixes: atomic counter in drift test fixture, stale-dir cleanup in gate.rs test

## Validation

- 2026-10-24 — phase `main` — verdict: APPROVE_WITH_REQUESTS → requests fixed — mode: full
  - `cargo test --locked` 627 passed, 0 failed; all `tests/test_*.sh` PASS
  - judge: independent (runs/sude-stalerev.md), omp/gemini

## Current State and Next Action
- active_phase: main
- lifecycle_status: done
- blockers: none
- open_items: none
- exact_next_action: owner merges PR
