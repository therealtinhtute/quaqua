# quaqua SLP slim — Plan (2026-10-08)

Lock: `spec-raw/quaqua-slp-slim-lock.md`. Repo: `~/quaqua`, base `master`.
Each phase = one task worker on its own branch (`quaqua/<name>`); merge needs anh iu's approval. Each phase leaves quaqua usable alone.
Gate for every phase: all `scripts/test_*.sh` PASS + the phase's new test.

## P1 — Docs: roles, brief, rules diet (lock §3, §4, §5) · no code · worker `sude`
1. `rules/_all/brief.md` → 5 sections Goal / Constraints / Current design / Unknowns / Evidence + "no pre-solve" + `## Challenge` in runs file → verify: file read back, ≤ 30 lines.
2. `AGENTS.md`: SLP roles table (Principal/Supervisor/Lead/Peer); "Only anh iu opens one" → "CoS spawns a Lead for pipeline-size work"; Lead merges project integration branch; no-SLP rule (<15' single file, or continuous-feedback work); kill criteria (same error ≥3 iterations) → verify: `rg -n "Only anh iu opens"` empty.
3. Rules diet: drop from `memory/rules.md` what scripts enforce (pane layout, herdr move, stale 15-min watchdog); merge duplicates with `rules/_all/` → verify: diff lists each removed line + which script/rule covers it.
4. Rename Supervisor → Auditor in docs mentioning `scripts/supervisor.sh` (script name unchanged) → verify: `rg -n "Supervisor" AGENTS.md` only refers to CoS role.
5. `docs/plans/active/self-learning.md` → mark superseded by telemetry (lock §2).

## P2 — `spawn-worker.sh --review` read-only mode (lock §1 reviewer) · worker `sude`
1. Flag `--review` for codex kinds: `--sandbox read-only`, no `--approve-for-me` write access to repo; still writes `~/quaqua/runs` → verify: extend `scripts/test_spawn_worker.sh` (fake herdr asserts args).
2. Refuse `--review` on non-codex kinds → verify: test exit code.

## P3 — `scripts/pipeline.sh` + telemetry (lock §1, §2) · worker `sude`, review `quansu`
1. Args per lock; refuse same harness for builder/reviewer; counts 2 toward cap.
2. Flow: builder plan → (Lead approves plan: pipeline pauses with exit 5 `plan-ready`, `--resume` continues) → build → `--review auto|always|skip` (auto: diff >150 lines or >1 top-level dir) → fresh reviewer per round, `Verdict:` line required → fix → max 2 rounds; dispute → `disputed`.
3. Budgets build 45' / review 15' / fix 20' / total 2h via `PROMPT_WORKER_BUDGET_SEC`.
4. Write `runs/pipeline-<task>.md` (status, rounds, commit, Challenge/steering log) + append `runs/telemetry.jsonl` line.
5. Verify: `scripts/test_pipeline.sh` with fake herdr: clean, rounds exhausted, disputed, review skipped, watchdog exit, plan-ready pause → telemetry line each case.

## P4 — Lead spawn (lock §5) · worker `sude`
1. `scripts/spawn-lead.sh <slug>`: Claude session `quaqua-<slug>-orchestrator` in `~/quaqua` (no worktree), split beside CoS, appends lead brief; refuses if one exists for slug; updates `projects/INDEX.md` owner → verify: `scripts/test_spawn_lead.sh` fake herdr.
2. Not counted toward the 4-worker cap (Lead is not a worker).

## P5 — Auditor checks (lock §3) · worker `sude`
1. `supervisor.sh` prompt adds: `runs/pipeline-*.md` `disputed|blocked` without a memory handling line; `clean` without commit or reviewer `Verdict`; unresolved `## Challenge` → verify: extend `scripts/test_supervisor.sh` fixture.

## P6 — Rollout (lock Verify)
CoS spawns a coucou-island Lead (P4), Lead runs one small real task through `pipeline.sh`; CoS re-runs tests, reads telemetry. Only then document pipeline usage in `AGENTS.md`.

## Order
P1 ∥ P2 (independent, 2 workers). P3 needs P2. P4 independent of P3 (can run ∥ P3). P5 needs P3 output format. P6 last.

## Risk
- Plan-approval pause adds a round-trip per pipeline → telemetry field `plan_changed: bool`; drop it after ~10 runs if never true (Better-SLP).
- `spawn-lead.sh` is +1 script (entity delta); alternative reusing `spawn-worker.sh` rejected: Lead needs no worktree and isn't capped.
