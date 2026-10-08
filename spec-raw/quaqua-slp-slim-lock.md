# Lock: SLP slim — pipeline + telemetry + rules diet (2026-10-08)

Source: https://vhlam.com/article/agent-orchestration-multi-agent-slp — roles = authority, not persona; right to challenge ≠ duty; drop mechanisms that don't change outcomes (Better-SLP).

## Decisions (anh iu)

### 1. `scripts/pipeline.sh` — build → review → fix
- `pipeline.sh <task> <repo> --brief <file> [--builder sude] [--reviewer quansu] [--review auto|always|skip] [--rounds 2]`
- Callers: project Leads (`<slug>-lead`), own project only — CoS never leads pipeline-size work (no self-review, see §5). Counts as 2 workers toward the cap of 4.
- Roles selectable, default sude/quansu; same harness for builder and reviewer → script refuses.
- Reviewer: codex `--sandbox read-only` (new review mode in `spawn-worker.sh`); a fresh reviewer each round; `runs/<reviewer>.md` ends with `Verdict: clean | N findings`.
- `--review auto` (default): review only when the diff crosses a size threshold or touches >1 module; else skip.
- Between rounds: no CoS gate; findings go straight to builder (same pane/worktree). Max 2 review rounds.
- Builder disputes a finding (with evidence) → stop as `disputed`, Lead decides; escalate to CoS only if goal/cost changes.
- Cleanup: each reviewer right after its runs file is read; builder at pipeline end after CoS test re-run passes.
- Budgets: build 45', review 15', fix 20', whole pipeline 2h; over → stop + `## Blockers`.
- Output: `runs/pipeline-<task>.md`; CoS reports 1 line: `clean|disputed|blocked`, rounds, commit, test.

### 2. Telemetry (replaces paused self-learning plan)
- Each pipeline appends one line to `runs/telemetry.jsonl`: `{ts, task, repo, builder, reviewer, review: run|skip, rounds, review_changed_outcome, status, minutes}`.
- After ~10 pipelines CoS reviews it: a reviewer that rarely changes outcomes → default off. Removing a mechanism is a valid outcome.

### 3. Rules diet + naming
- Remove from `memory/rules.md` rules already enforced by scripts (pane layout, herdr move mechanics, stale 15-min watchdog note); merge duplicates with `rules/_all/`.
- Rename the role of `scripts/supervisor.sh` to **Auditor** in docs (CoS holds the Supervisor role; anh iu is Principal, outside SLP). Auditor also flags: `runs/pipeline-*.md` `disputed|blocked` with no memory handling line; `clean` pipelines with no matching commit or reviewer `Verdict`.

### 4. Brief + small-task shortcut
- `rules/_all/brief.md` sections: `Goal` / `Constraints` (hard, from anh iu) / `Current design` (may be challenged) / `Unknowns` (+ how to verify) / `Evidence` (what to return). No pre-solving: brief never fixes the cause or the solution.
- No SLP (one worker, no Lead/pipeline): task <15 min single file, or work needing continuous anh iu feedback (UI/UX, game feel).

### 5. SLP roles (added 2026-10-08, sources: vhlam, Addy Osmani Code Agent Orchestra, Anthropic/Cognition)
| Role | Who | Owns | Authority |
|---|---|---|---|
| Principal | anh iu | goal, priorities, trade-offs | approve push / merge main; may steer any agent, steering recorded in Lead state |
| Supervisor | CoS Quả Quả | intake, sizing, brief, cross-project view (cap, CPU/benchmark contention), digest | spawn Lead; verify Lead result (re-run test); escalate goal/cost changes to anh iu |
| Lead | `<slug>-lead` (spawned by CoS via `scripts/spawn-lead.sh <slug>`) | project state, ownership, dependencies, evidence review | run `pipeline.sh`, spawn ≤4 Peers, approve Peer plans, **merge into project integration branch** (acceptance) |
| Peer | task workers (sude build, quansu review, …) | one task in own worktree | commit on own branch; may challenge premises with evidence |

- CoS never acts as Lead on pipeline-size work (no self-review).
- Challenge channel: Peer writes `## Challenge` (premise + evidence) in `runs/<name>.md`; Lead must resolve it before acceptance. Right to challenge, not duty.
- Plan approval: pipeline-size tasks → builder writes a short plan first; Lead approves before code.
- Kill criteria: same error ≥3 iterations → kill, spawn a fresh worker (alongside the watchdog).
- Steering from anh iu or CoS directly to a Peer must be recorded in Lead state (`runs/pipeline-<task>.md`).

## Amendments (2026-10-08, post-review)
- Callers corrected to project Leads only (was "CoS + project orchestrators") — AGENTS.md forbids CoS leading pipeline-size work.
- Lead agent name is `<slug>-lead` (spawn-lead.sh; herdr caps names at 32 chars), not the earlier `quaqua-<slug>-orchestrator` label; repo base is `main`, not `master`.
- Report tails: a task worker's runs file ends `## Lessons` (rules/_all/brief.md); a pipeline reviewer's runs file ends `Verdict:` — two different contracts, both intentional.
- `plan_changed` telemetry field is dropped — telemetry carries only the §2 fields.
- `--rounds` is configurable; "2" everywhere else means the default.

## Out of scope
Ownership lock registry (worktree/branch = owner); peer-to-peer messaging; new roles.

## Verify
- Fake-herdr test for pipeline: `clean`, rounds exhausted, `disputed`, review skipped, watchdog exit; telemetry line written in each case.
- All existing `scripts/test_*.sh` PASS.
- Rollout: one small real coucou-island task through the pipeline before documenting it in `AGENTS.md`.
