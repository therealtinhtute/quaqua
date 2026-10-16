# quaqua-rs

repo: ~/quaqua
test: `cargo test` + only the `tests/test_<x>.sh` a phase touches; full `bash tests/run.sh` at handback (S1 step 7) and cutover

## Vision
One `quaqua` Rust binary (crate `crates/quaqua/`) replaces the bash bodies under `scripts/`; `team.toml` is the single config source; piggery ideas (merge queue, taskforce, policy rules) land in it. Done = cutover gate passes and anh iu merges the cutover PR. Plan: `docs/research/herdr-grid-rust-migration-plan.md` (§ Build phases, § Cutover gate, § Execution).

## Now
- [x] S0.4: `quaqua-rs-lead` spawned, card read (2026-10-15)
- [x] S1 step 1 R0: accepted, integ `ec2cf6e` (2026-10-15) — paused for CoS verify
- [ ] Handoff pushed: `wip/2026-10-15-handoff` (main@86f0032), `quaqua/quaqua-rs-integ`, `quaqua/tieude-r0`. origin/main diverged, 4 files conflict (2026-10-15)
- [ ] S1 step 2 R1: `team.toml` + config validate/get (incl. default_reviewer, can_review) — waits for CoS go

## Rules
- Integ branch `quaqua/quaqua-rs-integ`; Peers `--base` it; `main` untouched until R6.
- `scripts/`, `rules/`, AGENTS.md, memory are CoS-only: R0–R5 add the crate, tests and fixtures only; R6 is the CoS + one worker.
- Builder `sude` (R2–R4) or `tieude` (R0, R1, R5); reviewer `suhuynh` (quansu unavailable, anh iu 2026-10-15).
- Drift guard: any `scripts/` change on `main` → ported into its owning phase the same week; logged below.

## Decisions
- 2026-10-15 One big-bang binary, CLI + hook fence, cutover = tests + 1 real SLP run (anh iu).
- 2026-10-15 S1 step 1 runs targeted checks; step 7 runs the full `tests/run.sh` (anh iu).

## Drift log
- 2026-10-15 `db6dee0` sude → omp (`spawn-worker.sh` sude branch, `pipeline.sh` `harness_of`) → port in R3/R4.
- 2026-10-15 claude `--review` (tieude-revclaude, pending merge) → port in R3; R4 `pipeline` default reviewer from `strategy.default_reviewer`.
- 2026-10-15 bash `pipeline.sh` grids builder + reviewer into the Lead tab despite `GRID_TAB` (seen r0 twice) → R4 must not port it (test in plan); bash fix deferred by anh iu.
- 2026-10-15 root cause: `grid-workers.sh --only` ignores `--tab`, plans a move to tab `quaqua` (grid_group) that never converges → pipeline blocks. R0 resumed with grid off (anh iu, one-off). R2 `grid` + R4 test must cover `--only` + custom tab.
- 2026-10-15 origin/main `fc369b7` (#36) `init-workspace.sh` orchestrator tag on the real CoS pane → port in R5 `init`.
- 2026-10-15 origin/main `b9bf382` (#38) `tests/run.sh` skips the 2 slow suites → reconcile with R0's `cargo test` hook in `tests/run.sh`.
