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
  - `layout.apply` kills or remaps processes → fallback to scratch-tab moves with id remap.
  - Public history leak → audit before publish; stop on any secret/private hit and ask anh iu.
- seams:
  - existing: `tests/test_spawn_worker.sh`, `tests/test_prompt_worker.sh`, `tests/test_herdr_gate.sh`, `tests/test_grid_workers.sh`, `tests/test_pipeline.sh`, `tests/test_docs.sh` via `bash tests/run.sh`
  - new (pending anh iu confirmation): `scripts/hooks/pre-commit` (`tests/test_pre_commit.sh`); `plugins/herdr-grid/plan.jq` snapshot→ops (`tests/test_herdr_grid_plan.sh`); `plugins/herdr-grid/hook.sh` with stub herdr (`tests/test_herdr_grid_hook.sh`)
- constraints: bash 3.2 + `/usr/bin/jq` only in the plugin; herdr via `$HERDR_BIN_PATH`; no push/merge/publish without anh iu; builder ≠ reviewer harness (sude builds, quansu reviews); probe only in a scratch workspace.
- recovery: every phase lands on `quaqua/herdr-grid-integ` and is revertable per merge commit; plugin can be unlinked at any time with `grid-workers.sh` still functional until phase `prose-cut`.

- phase_slug: `enforce`
  status: planned
  - goal: R1, R2, R3, R4 | depends_on: none
  - surfaces: `scripts/worker/spawn-worker.sh`, `scripts/hooks/pre-commit` (new), `scripts/init-workspace.sh`, `scripts/worker/prompt-worker.sh`, `scripts/hooks/herdr-gate.sh`, `AGENTS.md:52`, `rules/_all/brief.md`, tests | avoided: grid scripts, plugin
  - escalate_when: push block also breaks the CoS checkout, or report check rejects an existing pipeline reviewer format
  - wave 1:
    - T1 worktree push block: `extensions.worktreeConfig` + `--worktree remote.<r>.pushurl no-push://blocked` per remote after worktree add — output: worker push fails with clear error — check: `bash tests/test_spawn_worker.sh` — stop_if: main checkout `git remote get-url --push origin` changes
    - T2 pre-commit machine-path check (anchored regex, allowlist `rules/_all/portable-paths.md`, `tests/**`), installed via `core.hooksPath` in init-workspace — output: hook + test — check: `bash tests/test_pre_commit.sh` — stop_if: existing tracked files fail the hook outside the allowlist
    - T3 run-report grammar in prompt-worker (builder `## Lessons`, reviewer `Verdict:`, malformed → exit 6 `WATCHDOG malformed-report`) + AGENTS.md:52 exit list — output: validator + docs — check: `bash tests/test_prompt_worker.sh && bash tests/test_pipeline.sh` — stop_if: pipeline reviewer reports fail validation
    - T4 gate allows `--help`/`-h` — output: gate change — check: `bash tests/test_herdr_gate.sh` — stop_if: none
  - phase check: `bash tests/run.sh`

- phase_slug: `probe`
  status: planned
  - goal: R5, R7, R8 (facts only) | depends_on: none
  - surfaces: scratch herdr workspace, `plugins/herdr-grid-probe/` (logging-only, deleted after) | avoided: live CoS/Lead panes
  - escalate_when: any probe answer is "no" (requires plan amendment before `plugin-shadow`)
  - wave 1:
    - T1 link logging plugin for the 4 events; script-created split shows `pane.created` in plugin log; record PATH + bash version — output: log excerpt — check: `herdr plugin list --json | jq '.result.plugins[]|select(.plugin_id|test("grid-probe"))|.warnings'` → `[]` — stop_if: hooks do not fire for script-created panes
    - T2 token survives `pane move`; agent integration metadata does not clear `herdr-grid` source tokens — output: snapshot before/after — check: `herdr api snapshot | jq '..|.tokens? // empty'` — stop_if: tokens dropped
    - T3 `layout.apply` on a 2-pane tab keeps processes; `agent_status_changed` fires on working→idle — output: pids + ids before/after, log excerpt — check: recorded in `runs/<name>.md` — stop_if: processes restarted
  - phase check: `runs/<probe worker>.md` answers all 6 questions with command output

- phase_slug: `plugin-shadow`
  status: planned
  - goal: R5, R6, R7, R8 | depends_on: probe
  - surfaces: `plugins/herdr-grid/{herdr-plugin.toml,hook.sh,plan.jq,apply.sh,README.md}`, `spawn-worker.sh` (tags; `grid_ready` after trust/send-keys handling :139-153), `spawn-lead.sh` (orchestrator tag), `init-workspace.sh` (CoS tag) | avoided: `grid-workers.sh` behaviour, `pipeline.sh`
  - escalate_when: shadow plan differs from `grid-workers.sh` result in a way the formula cannot explain
  - wave 1:
    - T1 `plan.jq` snapshot→ops with fixtures (0–5 workers, two owners same group, two sessions, untagged pane, working deferral, idempotence) — output: planner — check: `bash tests/test_herdr_grid_plan.sh` — stop_if: idempotence fails
    - T2 spawn tagging (role/group/owner after split, `grid_ready=1` last) — output: tags in snapshot — check: `bash tests/test_spawn_worker.sh && bash tests/test_spawn_lead.sh` — stop_if: spawn order changes trust/send-keys behaviour
  - wave 2:
    - T3 `hook.sh` (pending counter, mkdir lock + pid reclaim, status-changed early exit) + manifest, shadow mode logs ops only — output: plugin linked, warnings empty — check: `bash tests/test_herdr_grid_hook.sh` (event during unlock, holder killed) — stop_if: lost wakeup reproduced
    - T4 `apply.sh` (layout.apply or move+remap per probe; verify each op; closed pane / failed nth move / zoomed tab) behind shadow flag — output: executor — check: `bash tests/test_herdr_grid_hook.sh` — stop_if: partial apply leaves foreign pane moved
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

## Validation

## Current State and Next Action
- active_phase: none
- lifecycle_status: planned
- blockers: new seams (pre-commit, plan.jq, hook.sh tests) await anh iu confirmation
- open_items: plan not yet approved; spawn `quaqua-lead` (slug `herdr-grid`) on approval
- exact_next_action: work full phase enforce
