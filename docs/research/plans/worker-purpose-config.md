---
status: active
lane: normal
---
# Worker purpose config — kinds and models from team.toml only

## Goal
- outcome: a workspace picks the worker for a job by purpose (`build`, `review`, `research`, `design`, …) and can add, rename or retune a kind or an opencode model by editing `team.toml` alone, with no engine rebuild.
- success_signal: a fixture `team.toml` with a new kind name (not one of the six built-ins) and a new opencode model spawns, is counted by the cap, cleaned up and costed correctly; `quaqua worker spawn --purpose review …` resolves to the kind the table names.
- actors: CoS and Leads spawning workers; anh iu editing `team.toml`.
- authority: anh iu, 2026-10-22 (question tool: "Purpose → kind", "move the model allowlist into team.toml"); `[kinds.*]` already owns harness/model/effort (`config.rs:468`, `templates/workspace/team.toml` header).
- requirements:
  - R1: no engine code outside tests names a specific kind; every kind list, prefix match and default comes from `team.toml` (`cap.rs:10`, `worker/cleanup.rs:183`, `worker/prompt/cost.rs:155,161`, `lead/spawn.rs:46,51`, `worker/prompt/mod.rs:592`, `config.rs:586`, the spawn usage text `worker/spawn.rs:35`) | acceptance: `grep -nE '"(tieude|detu|sude|ni|suhuynh|truonglao)"' crates/quaqua/src` outside `#[cfg(test)]` returns nothing, and a test with a renamed kind passes cap, cleanup and cost | source: anh iu 2026-10-22
  - R2: `team.toml` gains an optional `[purposes]` table (purpose → kind); `quaqua worker spawn --purpose <p>` resolves the kind from it, an unknown purpose or a purpose naming an unknown kind fails config validation with the offending key | acceptance: config tests for valid, unknown-purpose and dangling-kind cases; spawn test resolving `--purpose` | source: anh iu 2026-10-22
  - R3: the opencode model → allowed variants list moves from `config.rs:210` (`opencode_model_variants`) into `team.toml`; validation reads it, and a model missing from it fails with a fix hint naming `team.toml` | acceptance: config test adding a new model via fixture passes; removing it fails with the hint | source: anh iu 2026-10-22
  - R4: an existing `team.toml` without `[purposes]` or the model table keeps working unchanged (deepseek-v4.1-flash keeps `low/high/max` as the built-in default) | acceptance: `cargo test --locked -p quaqua config` with the current template unchanged passes; `quaqua init` on a fresh workspace emits the new tables | source: R4 compat, AGENTS.md managed templates
- non-goals:
  - NG1: per-spawn `--harness/--model/--effort` overrides (not chosen).
  - NG2: changing harness effort vocabularies (`harness/mod.rs:39`) or launch adapters.
  - NG3: model routing by cost or automatic purpose detection from the brief.

## Phases and Verification
- approach: data in `team.toml`, engine reads it (purpose table + model table + kind names from config); rejected: spawn-time field overrides (wider surface, not chosen).
- phase_slug: `main`
  status: planned
  - goal: R1, R2, R3, R4 | depends_on: none
  - surfaces: `crates/quaqua/src/{config.rs,cap.rs,worker/spawn.rs,worker/cleanup.rs,worker/prompt/cost.rs,worker/prompt/mod.rs,lead/spawn.rs}`, `templates/workspace/team.toml`, `templates/workspace/models.md` | avoided: `harness/`, launch adapters, journal schema
  - phase check: `for f in config cap spawn cleanup cost; do cargo test --locked -p quaqua "$f"; done` plus the R1 grep

## Log

## Validation

## Current State and Next Action
- active_phase: none
- lifecycle_status: planned
- blockers: none
- open_items: none
- exact_next_action: work full phase main
