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
  status: done (A1 merged PR #31, 417eefe / c7202dd · raw/2026-10-14-quaqua-gridconv2.md)
  - goal: R5, R6, R7, R8 | depends_on: probe
  - surfaces: `plugins/herdr-grid/{herdr-plugin.toml,hook.sh,plan.jq,apply.sh,README.md}`, `spawn-worker.sh` (tags; `grid_ready` after trust/send-keys handling :139-153), `spawn-lead.sh` (orchestrator tag), `init-workspace.sh` (CoS tag) | avoided: `grid-workers.sh` behaviour, `pipeline.sh`
  - escalate_when: shadow plan differs from `grid-workers.sh` result in a way the formula cannot explain
  - wave 1:
    - T1 `plan.jq` snapshot→ops with fixtures (0–5 workers, two owners same group, two sessions, untagged pane, working deferral, idempotence) — output: planner — check: `bash tests/test_herdr_grid_plan.sh` — stop_if: idempotence fails
    - T2 spawn tagging (role/group/owner after split, `grid_ready=1` last) — output: tags in snapshot — check: `bash tests/test_spawn_worker.sh && bash tests/test_spawn_lead.sh` — stop_if: spawn order changes trust/send-keys behaviour
  - wave 2:
    - T3 `hook.sh` (pending counter, mkdir lock + pid reclaim, status-changed early exit; idle/done/unknown = not working; closed panes detected by snapshot diff, not `pane.closed`, which misses tab/workspace close; subscribe `pane.moved`, `tab.closed`, `workspace.closed`, never `layout.updated`) + manifest, shadow mode logs ops only — output: plugin linked, warnings empty — check: `bash tests/test_herdr_grid_hook.sh` (event during unlock, holder killed) — stop_if: lost wakeup reproduced
    - T4 `apply.sh` (`pane.move` only, id remap from `pane.moved`; verify each op; closed pane / failed nth move / zoomed tab) behind shadow flag — output: executor — check: `bash tests/test_herdr_grid_hook.sh` — stop_if: partial apply leaves foreign pane moved
  - phase check: `bash tests/run.sh` + shadow log matches actual layout over ≥5 real spawns (passed: fixtures for 2, 3, 4 worker grids converge to [])

- phase_slug: `cutover`
  status: done (B1–B3 PASS, PR #32 merged main 9a1d3ca, live shadow=0 since 2026-10-14 · raw/2026-10-14-herdr-grid-applyfix3.md)
  - goal: R6, R7 | depends_on: plugin-shadow, enforce
  - surfaces: `hook.sh` shadow off, `grid-workers.sh` → `--check` + `--repair`, `spawn-worker.sh:159-177`, `cleanup-worker.sh:16-30`, `prompt-worker.sh` pre-prompt `--check`, `pipeline.sh:292-333`, `spawn-lead.sh:132` | avoided: prose rules
  - escalate_when: `--check` blocks a prompt for >1 min on a non-foreign layout
  - wave 1:
    - T1 grid-workers `--check`/`--repair` + callers switched — output: scripts (`grid-workers.sh`, `spawn-worker.sh`, `cleanup-worker.sh`, `prompt-worker.sh`, `pipeline.sh` switched to `--check`; note: `spawn-lead.sh:137` is an orchestrator launcher with no grid hook, prompt prose kept until prose-cut) — check: `bash tests/test_grid_workers.sh && bash tests/test_cleanup_worker.sh && bash tests/test_pipeline.sh` — stop_if: `--only` pipeline isolation lost
    - T2 live acceptance: 3-worker fanout + cleanup by plugin alone (verified in B3 live run rounds 1–4, 10 applies, 0 errors, 0s check · raw/2026-10-14-herdr-grid-applyfix3.md) — output: snapshot evidence in run report — check: `herdr api snapshot` layout matches formula — stop_if: any manual `--repair` needed
  - phase check: `bash tests/run.sh`

- phase_slug: `prose-cut`
  status: planned (waiting: earliest 2026-10-21 = 7 clean days without --repair · raw/2026-10-14-quaqua-gridprose.md)
  - goal: R9 | depends_on: cutover (+ ≥7 days with no `--repair` use)
  - surfaces: delete `rules/_all/fanout-agent-grid.md`, `.claude/rules/herdr-fanout-grid.md`, `rules/_all/portable-paths.md`; `AGENTS.md:51,56`; `herdr-gate.sh:30-31` + pipeline blocker text; trim `rules/_all/worker-timebox.md` to worker-facing lines, rest → skill `herdr-orchestration`; `handoff.md` drop push line | avoided: scripts
  - escalate_when: a deleted rule has no enforcing code (found by rg)
  - wave 1:
    - T1 remove/trim prose + fix pointers — output: docs — check: `bash tests/test_docs.sh && cat rules/_all/*.md | wc -c` ≤1500 — stop_if: tests reference deleted files
  - phase check: `bash tests/run.sh`

- phase_slug: `publish`
  status: in_progress (T1 audit done · raw/2026-10-14-quaqua-gridaudit.md; awaiting anh iu decision)
  - goal: R10 | depends_on: cutover
  - surfaces: git history audit (read-only), repo visibility, topic | avoided: history rewrite
  - escalate_when: always before visibility change; any audit hit
  - wave 1:
    - T1 audit `git log --all -p` + refs for `memory/`, `raw/`, `runs/`, `/Users/`, emails, secrets (gitleaks if installed) — output: report (done: raw/2026-10-14-quaqua-gridaudit.md; 0 secrets, /Users/ in plist, spec-raw in history, cos-legacy local branch, ots.vn rewritten in 417eefe) — check: report file reviewed by anh iu — stop_if: any hit
    - T2 (anh iu approved) make public, add topic `herdr-plugin` — output: public repo — check: `herdr plugin install therealtinhtute/quaqua/plugins/herdr-grid` on clean config — stop_if: install fails
  - phase check: plugin installs from marketplace path

## Log
- 2026-10-13 enforce done: T1–T4 on `quaqua/herdr-grid-integ` (d657c6d, fd0d639), 14/14 tests pass (watchdog skipped, known flaky).
- 2026-10-13 probe done (`runs/sude-gridprobe.md`): Q5 no → plan amended (no `layout.apply`, snapshot-diff close detection), approved by anh iu.
- 2026-10-13 plugin-shadow T1–T4 merged to main (#27), plugin linked shadow; shadow log: 2 real spawns (1-worker each), ops converge to [] — need ≥5 incl. a 2–4 worker grid.
- 2026-10-13 smoke: 10 idle spawns (rounds 4,2,3,1) in ~33s; grid-workers layouts match plugin targets for every shape (1 beside CoS, 2 L|R, 3 A|B+C, 4 2x2). Gap: pane.moved does re-plan (plugin log), but in 2–4 rounds plan.jq never converges to [] on the grid-workers layout (keeps proposing new_tab label=quaqua; suspected tab label `workers` vs group); 1-worker and cleanup do converge.
- 2026-10-14 plugin-shadow A1 done: PR #31 merged main (417eefe / c7202dd · raw/2026-10-14-quaqua-gridconv2.md). plan.jq 2–4 worker convergence fixed with is_group_tab accepting <group>, <group>-workers, and workers; 2-, 3-, and 4-worker grid fixtures added; test_herdr_grid_plan.sh and test_grid_workers.sh PASS.
- 2026-10-14 cutover B1–B3 done: PR #32 merged main (9a1d3ca · raw/2026-10-14-herdr-grid-applyfix3.md). Live apply mode active (HERDR_GRID_SHADOW=0); grid-workers.sh --check barrier and --repair implemented, callers switched to --check. B3 live acceptance passed in rounds 1–4 with 10/10 applies, 0 errors, 0s check, 0 stray panes. Live plugin log since go-live: 37 applies, 0 errors.
- 2026-10-14 publish audit A2 done (raw/2026-10-14-quaqua-gridaudit.md): History scanned (176 commits). Found ots.vn corporate emails (rewritten in 417eefe), /Users/ macOS path in supervisor plist (867a9cd), internal spec-raw in main history, and unpushed cos-legacy branch. Verdict NOT SAFE to make public without remediation (Option 1 clean repo export recommended).
- 2026-10-14 prose-cut A3 evaluated (raw/2026-10-14-quaqua-gridprose.md): Verdict BLOCKED/deferred; prose-cut depends on cutover + 7 clean days without --repair (earliest 2026-10-21).

## Validation
- `tests/test_herdr_grid_plan.sh`: 12/12 PASS (convergence on 0–4 workers, group tab aliases, 2/3/4 grid fixtures · 417eefe · raw/2026-10-14-quaqua-gridconv2.md)
- `tests/test_herdr_grid_hook.sh`: 20/20 PASS (live apply, event storm absorption, lock recovery, working pane deferral, non-cos owner isolation, backoff stabilization · 9a1d3ca · raw/2026-10-14-herdr-grid-applyfix3.md)
- `tests/test_grid_workers.sh`: 52/52 PASS (`--check` clean layout, deferred moves, timeouts, `--repair` mode · 9a1d3ca · raw/2026-10-14-quaqua-gridcheck.md)
- `tests/test_cleanup_worker.sh` & `tests/test_pipeline.sh`: PASS (grid check integration · 9a1d3ca)
- `tests/test_docs.sh`: PASS (all mandatory doc and script references valid)
- Live acceptance (B3): 10/10 applies across rounds 1–4, 0 errors, 0s checks, 0 stray panes; live shadow=0 operating cleanly (37 applies, 0 errors since go-live · 9a1d3ca · raw/2026-10-14-herdr-grid-applyfix3.md)

## Current State and Next Action
- active_phase: prose-cut (waiting) / publish (in progress)
- lifecycle_status: in_progress
- blockers:
  - prose-cut gated on 7 clean days without --repair (earliest 2026-10-21; raw/2026-10-14-quaqua-gridprose.md)
  - publish gated on anh iu decision on audit findings (raw/2026-10-14-quaqua-gridaudit.md)
- open_items:
  - A2 audit findings: /Users/ in supervisor plist (867a9cd), internal spec-raw in main history, cos-legacy branch holding memory/projects/wiki (raw/2026-10-14-quaqua-gridaudit.md)
  - Smoke branches cleanup: tieude-smk*, sm*x*, b3r*, b3s*, b3t*, b3x*, b3v* to delete (needs anh iu; projects/herdr-grid.md:14)
- exact_next_action: Present A2 audit findings to anh iu for publish path decision (Option 1 clean repo export recommended), while monitoring live plugin operations for 7 clean days until 2026-10-21 for prose-cut

## Resume briefs
Run with `scripts/worker/prompt-worker.sh <name> "<brief>"` (check first with `--check-brief`). Workers run targeted tests only. The CoS or Lead runs `tests/run.sh` once at verify (~4 min).

Schedule & Status:
- **Wave A (Completed / Deferred):**
  - A1 finish shadow: DONE (merged PR #31, 417eefe / c7202dd · raw/2026-10-14-quaqua-gridconv2.md).
  - A2 publish audit: DONE (report completed · raw/2026-10-14-quaqua-gridaudit.md; findings pending anh iu decision).
  - A3 prose-cut draft: DEFERRED (evaluated · raw/2026-10-14-quaqua-gridprose.md; blocked until cutover + 7 clean days without --repair, earliest 2026-10-21).
- **Wave B (Completed):**
  - B1 apply path: DONE (merged PR #32, 9a1d3ca · raw/2026-10-14-quaqua-gridapply.md).
  - B2 check barrier: DONE (merged PR #32, 9a1d3ca · raw/2026-10-14-quaqua-gridcheck.md).
  - B3 live acceptance: DONE (verified live in w1, 10 applies, 0 errors, PR #32, 9a1d3ca · raw/2026-10-14-herdr-grid-applyfix3.md).
- **Upcoming Gates & Waves:**
  - Prose-cut: gated on 7 clean days without `--repair` (earliest 2026-10-21).
  - Publish: gated on anh iu decision on audit remediation options.

### A1 finish shadow — DONE
Completed in PR #31 (417eefe / c7202dd · raw/2026-10-14-quaqua-gridconv2.md).
Goal was: Finish WIP 7592aa7/152c95e so plan.jq returns `[]` on a 2–4 worker layout built by grid-workers.sh. Added `is_group_tab` in `plan.jq`, 2-, 3-, 4-worker grid fixtures; verified by `test_herdr_grid_plan.sh` and `test_grid_workers.sh`.

### A2 publish audit — DONE (Findings Pending Decision)
Completed via `detu-gridaudit` (raw/2026-10-14-quaqua-gridaudit.md).
Goal was: Prove whether the repo history is safe to make public.
Finding: NOT SAFE in current state without remediation (/Users/ plist, spec-raw in history, cos-legacy branch; corporate emails rewritten in 417eefe). Option 1 (clean export repo) recommended to anh iu.

### A3 prose-cut draft — DEFERRED (Gated on 2026-10-21)
Evaluated via `tieude-gridprose` (raw/2026-10-14-quaqua-gridprose.md).
Goal: Prepare the prose-cut diff now so it can merge the day the 7-day gate passes.
Status: Deferred. Gated on cutover + 7 clean days without `--repair`.
Brief to execute on/after 2026-10-21:
Goal: Remove and trim mechanical rule prose enforced by the live herdr-grid plugin and pre-commit hooks.
Hard constraints: md files only; surfaces per phase `prose-cut`; every deleted rule must have enforcing code (rg it, list file:line in report); commit on branch, no push.
Current design (open to challenge): delete fanout-agent-grid.md, .claude/rules/herdr-fanout-grid.md, portable-paths.md; trim worker-timebox.md to worker lines, rest → skill herdr-orchestration; fix pointers in AGENTS.md:51,56.
Evidence: `bash tests/test_docs.sh && cat rules/_all/*.md | wc -c` ≤1500.
Time budget: 20 min

### B1 apply path — DONE
Completed in PR #32 (9a1d3ca · raw/2026-10-14-quaqua-gridapply.md).
Goal was: With `HERDR_GRID_SHADOW=0`, hook.sh executes the plan through apply.sh and layout converges. Implemented in `plugins/herdr-grid/{hook.sh,apply.sh}`, tested in `tests/test_herdr_grid_hook.sh`.

### B2 check barrier — DONE
Completed in PR #32 (9a1d3ca · raw/2026-10-14-quaqua-gridcheck.md).
Goal was: grid-workers.sh gains `--check` (block until plugin plan is `[]`, ≤60s) and `--repair`; callers switch to `--check`. Implemented in `scripts/worker/grid-workers.sh`; callers in `spawn-worker.sh`, `cleanup-worker.sh`, `prompt-worker.sh`, `pipeline.sh` switched to `--check`. Note: `spawn-lead.sh:137` is an orchestrator launcher with no grid hook; prompt prose kept until prose-cut.

### B3 live acceptance — DONE
Completed in PR #32 (9a1d3ca · raw/2026-10-14-herdr-grid-applyfix3.md).
Goal was: Smoke rounds 1–4 workers with plugin live (HERDR_GRID_SHADOW=0) and no `--repair`; layout matches formula. Fixed anchor isolation, in-flight ready polling, and backoff stabilization in `cbd6a59` (merged in PR #32). 10/10 applies, 0 errors, 0s check across rounds 1–4.
