---
status: active
lane: high-risk
---
# herdr-grid plugin + instruction-layer minimisation

## Goal
- outcome: Mechanical rules are enforced by code instead of prose: a generic herdr layout reconciler plugin in `plugins/herdr-grid/` arranges worker panes, and scripts enforce push block, portable paths and run-report shape; `rules/_all` shrinks to ≤1.5 KB.
- success_signal: `bash tests/run.sh` green; a real 3-worker fanout + cleanup is arranged by the plugin with no agent grid step; `wc -c rules/_all/*.md` total ≤1500; `herdr plugin install therealtinhtute/quaqua/plugins/herdr-grid` works on a clean config.
- actors: anh iu (Principal), CoS, Leads, task workers (pi/codex/claude), herdr users installing the plugin.
- authority: anh iu decisions 2026-10-12 (this session); reviews `raw/2026-10-12-quaqua-bacdau-gridplan.md`, `raw/2026-10-12-quaqua-truonglao-gridplan.md`; `AGENTS.md`; `scripts/worker/*.sh`.
- requirements:
  - R1: A worker cannot `git push` from its worktree, regardless of harness; the CoS still can. | acceptance: spawn test — `git -C $wt push` fails, main checkout push URL unchanged | source: AGENTS.md:59, review truonglao #9
  - R2: Commits adding machine-specific absolute paths are rejected, with policy/fixture allowlist. | acceptance: `tests/test_pre_commit.sh` | source: rules/_all/portable-paths.md
  - R3: `prompt-worker.sh` rejects a run report missing `## Lessons` (builders) or `Verdict:` (reviewers). | acceptance: `tests/test_prompt_worker.sh` new cases | source: rules/_all/brief.md:21-25
  - R4: herdr-gate allows `--help`/`-h` on blocked verbs. | acceptance: `tests/test_herdr_gate.sh` case | source: review truonglao #13
  - R5: Plugin contract is pane tokens (`grid_role`, `grid_group`, `grid_owner`=agent name, `grid_ready`=1 set last); plugin moves only `grid_ready=1` panes and never reads quaqua files. | acceptance: planner fixture tests incl. untagged pane untouched | source: anh iu 2026-10-12
  - R6: Layout formula per owner: 1 worker total → split beside owner 2/5|3/5; else one `<group>-workers` tab per group, 2=left|right, 3=A|B+C, 4=2x2; >4 rejected. Second run produces zero ops. | acceptance: `tests/test_herdr_grid_plan.sh` | source: rules/_all/fanout-agent-grid.md:6
  - R7: Moves of `working` panes are deferred and complete on `pane.agent_status_changed`; `--check` passes with only deferred ops. | acceptance: planner + executor tests | source: anh iu 2026-10-12, review blocker 2
  - R8: Reconciler is event-hook driven (created/closed/agent_detected/agent_status_changed), serialized by a mkdir lock with pending counter, bash-3.2 safe, no flock. | acceptance: executor tests (event during unlock, holder killed) | source: anh iu 2026-10-12, reviews
  - R9: After cutover no agent-facing instruction tells the agent to run the grid; prose rules removed/trimmed. | acceptance: `rg grid-workers AGENTS.md .claude/rules rules/_all` → only `--repair` mention; size check | source: anh iu 2026-10-12
  - R10: quaqua is public with topic `herdr-plugin` only after a full-history privacy audit approved by anh iu. | acceptance: audit report + anh iu approval logged | source: anh iu 2026-10-12
- non-goals:
  - NG1: No daemon / `events.subscribe` subscriber.
  - NG2: No re-tagging after herdr restart (tokens are not persisted; untagged panes are left alone).
  - NG3: No veto of herdr commands (herdr has none); Claude gate stays a guardrail only.
  - NG4: No history rewrite without separate approval.

## Phases and Verification
- approach: Enforcement first (cheap, independent), then a live probe of herdr plugin facts, then the plugin in shadow mode beside the existing synchronous `grid-workers.sh`, then cutover with `--check`/`--repair`, then prose cut after a week of clean use, then publish. Rejected: direct replacement of `grid-workers.sh` by the plugin (reviews: async hooks are no pre-prompt barrier; pipeline/`--only` callers would race).
- risks:
  - Hook race with spawn (`pane.created` before tagging; pane ids change on move) → `grid_ready=1` set last, owner by agent name; stop if probe shows tokens lost on move → re-tag on move fallback.
  - Event storms / lost wakeups → pending counter re-checked before unlock; executor tests; recovery: disable plugin (`herdr plugin disable`), `grid-workers.sh --repair`.
  - `layout.apply` kills processes and remaps ids (probe Q5 = no) → never use it; arrange panes with `pane.move` only, re-keying by `pane.moved.previous_pane_id`.
  - Public history leak → audit before publish; stop on any secret/private hit and ask anh iu.
- seams:
  - existing: `tests/test_spawn_worker.sh`, `tests/test_prompt_worker.sh`, `tests/test_herdr_gate.sh`, `tests/test_grid_workers.sh`, `tests/test_pipeline.sh`, `tests/test_docs.sh` via `bash tests/run.sh`
  - new (pending anh iu confirmation): `scripts/hooks/pre-commit` (`tests/test_pre_commit.sh`); `plugins/herdr-grid/plan.jq` snapshot→ops (`tests/test_herdr_grid_plan.sh`); `plugins/herdr-grid/hook.sh` with stub herdr (`tests/test_herdr_grid_hook.sh`)
- constraints: bash 3.2 + `/usr/bin/jq` only in the plugin; herdr via `$HERDR_BIN_PATH`; no push/merge/publish without anh iu; builder ≠ reviewer harness (sude builds, quansu reviews); probe only in a scratch workspace.
- recovery: every phase lands on `quaqua/herdr-grid-integ` and is revertable per merge commit; plugin can be unlinked at any time with `grid-workers.sh` still functional until phase `prose-cut`.

- phase_slug: `enforce`
  status: done
  - goal: R1, R2, R3, R4 | depends_on: none
  - surfaces: `scripts/worker/spawn-worker.sh`, `scripts/hooks/pre-commit` (new), `scripts/workspace/init-workspace.sh`, `scripts/worker/prompt-worker.sh`, `scripts/hooks/herdr-gate.sh`, `AGENTS.md:52`, `rules/_all/brief.md`, tests | avoided: grid scripts, plugin
  - escalate_when: push block also breaks the CoS checkout, or report check rejects an existing pipeline reviewer format
  - wave 1:
    - T1 worktree push block: `extensions.worktreeConfig` + `--worktree remote.<r>.pushurl no-push://blocked` per remote after worktree add — output: worker push fails with clear error — check: `bash tests/test_spawn_worker.sh` — stop_if: main checkout `git remote get-url --push origin` changes
    - T2 pre-commit machine-path check (anchored regex, allowlist `rules/_all/portable-paths.md`, `tests/**`), installed via `core.hooksPath` in init-workspace — output: hook + test — check: `bash tests/test_pre_commit.sh` — stop_if: existing tracked files fail the hook outside the allowlist
    - T3 run-report grammar in prompt-worker (builder `## Lessons`, reviewer `Verdict:`, malformed → exit 6 `WATCHDOG malformed-report`) + AGENTS.md:52 exit list — output: validator + docs — check: `bash tests/test_prompt_worker.sh && bash tests/test_pipeline.sh` — stop_if: pipeline reviewer reports fail validation
    - T4 gate allows `--help`/`-h` — output: gate change — check: `bash tests/test_herdr_gate.sh` — stop_if: none
  - phase check: `bash tests/run.sh`

- phase_slug: `probe`
  status: done
  - goal: R5, R7, R8 (facts only) | depends_on: none
  - surfaces: scratch herdr workspace, `plugins/herdr-grid-probe/` (logging-only, deleted after) | avoided: live CoS/Lead panes
  - escalate_when: any probe answer is "no" (requires plan amendment before `plugin-shadow`)
  - wave 1:
    - T1 link logging plugin for the 4 events; script-created split shows `pane.created` in plugin log; record PATH + bash version — output: log excerpt — check: `herdr plugin list --json | jq '.result.plugins[]|select(.plugin_id|test("grid-probe"))|.warnings'` → `[]` — stop_if: hooks do not fire for script-created panes
    - T2 token survives `pane move`; agent integration metadata does not clear `herdr-grid` source tokens — output: snapshot before/after — check: `herdr api snapshot | jq '..|.tokens? // empty'` — stop_if: tokens dropped
    - T3 `layout.apply` on a 2-pane tab keeps processes; `agent_status_changed` fires on working→idle — output: pids + ids before/after, log excerpt — check: recorded in `runs/<name>.md` — stop_if: processes restarted
  - phase check: `runs/<probe worker>.md` answers all 6 questions with command output

- phase_slug: `plugin-shadow`
  status: in_progress (code merged #27; phase check pending)
  - goal: R5, R6, R7, R8 | depends_on: probe
  - surfaces: `plugins/herdr-grid/{herdr-plugin.toml,hook.sh,plan.jq,apply.sh,README.md}`, `spawn-worker.sh` (tags; `grid_ready` after trust/send-keys handling :139-153), `spawn-lead.sh` (orchestrator tag), `init-workspace.sh` (CoS tag) | avoided: `grid-workers.sh` behaviour, `pipeline.sh`
  - escalate_when: shadow plan differs from `grid-workers.sh` result in a way the formula cannot explain
  - wave 1:
    - T1 `plan.jq` snapshot→ops with fixtures (0–5 workers, two owners same group, two sessions, untagged pane, working deferral, idempotence) — output: planner — check: `bash tests/test_herdr_grid_plan.sh` — stop_if: idempotence fails
    - T2 spawn tagging (role/group/owner after split, `grid_ready=1` last) — output: tags in snapshot — check: `bash tests/test_spawn_worker.sh && bash tests/test_spawn_lead.sh` — stop_if: spawn order changes trust/send-keys behaviour
  - wave 2:
    - T3 `hook.sh` (pending counter, mkdir lock + pid reclaim, status-changed early exit; idle/done/unknown = not working; closed panes detected by snapshot diff, not `pane.closed`, which misses tab/workspace close; subscribe `pane.moved`, `tab.closed`, `workspace.closed`, never `layout.updated`) + manifest, shadow mode logs ops only — output: plugin linked, warnings empty — check: `bash tests/test_herdr_grid_hook.sh` (event during unlock, holder killed) — stop_if: lost wakeup reproduced
    - T4 `apply.sh` (`pane.move` only, id remap from `pane.moved`; verify each op; closed pane / failed nth move / zoomed tab) behind shadow flag — output: executor — check: `bash tests/test_herdr_grid_hook.sh` — stop_if: partial apply leaves foreign pane moved
  - phase check: `bash tests/run.sh` + shadow log matches actual layout over ≥5 real spawns

- phase_slug: `cutover`
  status: planned
  - goal: R6, R7 | depends_on: plugin-shadow, enforce
  - surfaces: `hook.sh` shadow off, `grid-workers.sh` → `--check` + `--repair`, `spawn-worker.sh:159-177`, `cleanup-worker.sh:16-30`, `prompt-worker.sh` pre-prompt `--check`, `pipeline.sh:292-333`, `spawn-lead.sh:132` | avoided: prose rules
  - escalate_when: `--check` blocks a prompt for >1 min on a non-foreign layout
  - wave 1:
    - T1 grid-workers `--check`/`--repair` + callers switched — output: scripts — check: `bash tests/test_grid_workers.sh && bash tests/test_cleanup_worker.sh && bash tests/test_pipeline.sh` — stop_if: `--only` pipeline isolation lost
    - T2 live acceptance: 3-worker fanout + cleanup by plugin alone — output: snapshot evidence in run report — check: `herdr api snapshot` layout matches formula — stop_if: any manual `--repair` needed
  - phase check: `bash tests/run.sh`

- phase_slug: `prose-cut`
  status: planned
  - goal: R9 | depends_on: cutover (+ ≥7 days with no `--repair` use)
  - surfaces: delete `rules/_all/fanout-agent-grid.md`, `.claude/rules/herdr-fanout-grid.md`, `rules/_all/portable-paths.md`; `AGENTS.md:51,56`; `herdr-gate.sh:30-31` + pipeline blocker text; trim `rules/_all/worker-timebox.md` to worker-facing lines, rest → skill `herdr-orchestration`; `handoff.md` drop push line | avoided: scripts
  - escalate_when: a deleted rule has no enforcing code (found by rg)
  - wave 1:
    - T1 remove/trim prose + fix pointers — output: docs — check: `bash tests/test_docs.sh && cat rules/_all/*.md | wc -c` ≤1500 — stop_if: tests reference deleted files
  - phase check: `bash tests/run.sh`

- phase_slug: `publish`
  status: planned
  - goal: R10 | depends_on: cutover
  - surfaces: git history audit (read-only), repo visibility, topic | avoided: history rewrite
  - escalate_when: always before visibility change; any audit hit
  - wave 1:
    - T1 audit `git log --all -p` + refs for `memory/`, `raw/`, `runs/`, `/Users/`, emails, secrets (gitleaks if installed) — output: report — check: report file reviewed by anh iu — stop_if: any hit
    - T2 (anh iu approved) make public, add topic `herdr-plugin` — output: public repo — check: `herdr plugin install therealtinhtute/quaqua/plugins/herdr-grid` on clean config — stop_if: install fails
  - phase check: plugin installs from marketplace path

## Log
- 2026-10-13 enforce done: T1–T4 on `quaqua/herdr-grid-integ` (d657c6d, fd0d639), 14/14 tests pass (watchdog skipped, known flaky).
- 2026-10-13 probe done (`runs/sude-gridprobe.md`): Q5 no → plan amended (no `layout.apply`, snapshot-diff close detection), approved by anh iu.
- 2026-10-13 plugin-shadow T1–T4 merged to main (#27), plugin linked shadow; shadow log: 2 real spawns (1-worker each), ops converge to [] — need ≥5 incl. a 2–4 worker grid.
- 2026-10-13 smoke: 10 idle spawns (rounds 4,2,3,1) in ~33s; grid-workers layouts match plugin targets for every shape (1 beside CoS, 2 L|R, 3 A|B+C, 4 2x2). Gap: pane.moved does re-plan (plugin log), but in 2–4 rounds plan.jq never converges to [] on the grid-workers layout (keeps proposing new_tab label=quaqua; suspected tab label `workers` vs group); 1-worker and cleanup do converge.

## Validation

## Current State and Next Action
- active_phase: plugin-shadow
- lifecycle_status: in_progress
- blockers: none
- open_items: none (integ merged via #25/#27; QUAQUA_OWNER test fixed in b54968f)
- exact_next_action: resume WIP branch quaqua/tieude-gridconv (plan.jq 2–4 convergence, unverified), run targeted tests, then close plugin-shadow

## Resume briefs
Run with `scripts/worker/prompt-worker.sh <name> "<brief>"` (check first with `--check-brief`). Workers run targeted tests only. The CoS or Lead runs `tests/run.sh` once at verify (~4 min).

Schedule (fastest path, SLP):
- **Wave A, now, 3 CoS workers in parallel:** A1 finish shadow, A2 publish audit, A3 prose-cut draft. A2 and A3 do not depend on A1.
- **Wave B, after A1 is verified:** the CoS spawns `herdr-grid-lead` (`scripts/lead/spawn-lead.sh herdr-grid`). The Lead runs B1 and B2 in parallel on `quaqua/herdr-grid-integ`, gets quansu to review the merged diff, and the CoS then runs B3, the live acceptance check.
- **Gates that parallelism cannot skip:**
  - The A3 draft merges only after cutover plus 7 clean days, unless anh iu waives the wait.
  - Making the repo public is anh iu's call.

### A1 finish shadow — `tieude-gridconv2` (`--base quaqua/tieude-gridconv`)
Goal: Finish WIP 7592aa7/152c95e so plan.jq returns `[]` on a 2–4 worker layout built by grid-workers.sh.
Hard constraints: plan.jq, tests/test_herdr_grid_plan.sh, tests/fixtures/herdr-grid/ only; bash 3.2 + /usr/bin/jq; 1-worker plan unchanged; commit on branch, no push; runs/<name>.md with ## Lessons + Verdict:.
Current design (open to challenge): WIP accepts the grid-workers tab as the group tab; fixture 3-workers-grid.json (A|B + C) must plan `[]`; add 2- and 4-worker fixtures.
Unknowns: whether split order/anchor also differs, not only the tab label.
Evidence: plugin log 2026-10-13 smoke: rounds 2/3/4 never converged, round 1 did. Check: `bash tests/test_herdr_grid_plan.sh && bash tests/test_grid_workers.sh`.
Time budget: 15 min
CoS after: smoke rounds 1–4 idle workers (~30s), last plugin plan `[]` per round → mark plugin-shadow done → Wave B.

### A2 publish audit — `detu-gridaudit` (read-only)
Goal: Prove whether the repo history is safe to make public.
Hard constraints: read-only; no history rewrite; report hits as file:commit with values redacted; runs/<name>.md with ## Lessons + Verdict:.
Current design (open to challenge): scan `git log --all -p` + all refs for memory/, raw/, runs/, reports/, /Users/, emails, tokens (gitleaks if installed); list which tracked paths are private-by-nature.
Unknowns: whether private memory/raw files were ever committed (if yes: rewrite vs fresh public repo, anh iu decides).
Evidence: the report itself, every hit with a reproducible command.
Time budget: 15 min

### A3 prose-cut draft — `tieude-gridprose` (branch only, merge gated)
Goal: Prepare the prose-cut diff now so it can merge the day the 7-day gate passes.
Hard constraints: md files only; surfaces per phase `prose-cut`; every deleted rule must have enforcing code (rg it, list file:line in report); commit on branch, no push.
Current design (open to challenge): delete fanout-agent-grid.md, .claude/rules/herdr-fanout-grid.md, portable-paths.md; trim worker-timebox.md to worker lines, rest → skill herdr-orchestration; fix pointers in AGENTS.md:51,56.
Unknowns: rules whose enforcement only lands in cutover (grid prose) — mark them "pending cutover" instead of deleting.
Evidence: `bash tests/test_docs.sh && cat rules/_all/*.md | wc -c` ≤1500.
Time budget: 20 min

### B1 apply path — `sude-gridapply` (Lead spawns, `--base quaqua/herdr-grid-integ`)
Goal: With `HERDR_GRID_SHADOW=0`, hook.sh executes the plan through apply.sh and the layout converges.
Hard constraints: plugins/herdr-grid/{hook.sh,apply.sh} + tests/test_herdr_grid_hook.sh only; `pane.move` only, never `layout.apply`; working panes deferred; rollback = `HERDR_GRID_SHADOW=1`; no push.
Current design (open to challenge): apply ops in order, remap ids from `pane.moved.previous_pane_id`, verify each op, stop on a foreign pane or a failed move.
Unknowns: event storm from its own moves (lock + pending counter should absorb it).
Evidence: `bash tests/test_herdr_grid_hook.sh` incl. a partial-apply case.
Time budget: 30 min

### B2 check barrier — `tieude-gridcheck` (Lead spawns, `--base quaqua/herdr-grid-integ`)
Goal: grid-workers.sh gains `--check` (block until the plugin plan is `[]`, ≤60s) and `--repair` (today's arranging); callers switch to `--check`.
Hard constraints: scripts/worker/{grid-workers,spawn-worker,cleanup-worker,prompt-worker}.sh, scripts/lead/{pipeline,spawn-lead}.sh + their tests; `--only` pipeline isolation kept; no plugin files; no push.
Current design (open to challenge): `--check` polls `herdr api snapshot` through plan.jq; prompt-worker.sh runs `--check` before `herdr agent prompt`.
Unknowns: whether a working pane's deferred move should block `--check` (proposal: no, deferred ops are ignored).
Evidence: `bash tests/test_grid_workers.sh && bash tests/test_cleanup_worker.sh && bash tests/test_pipeline.sh`.
Time budget: 30 min

### B3 live acceptance — the CoS (no worker)
After the Lead merges B1+B2 into integ (quansu review clean): smoke rounds 1–4 workers with the plugin live and no `--repair`, then `herdr api snapshot` matches the formula. The PR from integ to main needs anh iu.
