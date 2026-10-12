# herdr-grid plugin + instruction-layer minimisation

Status: detailed plan, awaiting anh iu approval. Reviews: `raw/2026-10-12-quaqua-bacdau-gridplan.md`, `raw/2026-10-12-quaqua-truonglao-gridplan.md`.
Owner on approval: `quaqua-lead` (SLP). Builders sude/tieude, reviewer quansu (different harness), per `models.md`.
Test entry: `bash tests/run.sh` (all suites). Every phase ends green on it.

## Goal
Mechanical rules move from prose to code: a generic herdr layout reconciler plugin in `plugins/herdr-grid/`, plus script enforcement. `rules/_all` goes from ~7.5 KB to ≤1.5 KB, and the agent no longer arranges panes.

## Decisions (anh iu, 2026-10-12)
- Event hooks + snapshot reconcile; no daemon.
- Pane-token contract; plugin never reads quaqua files.
- Working panes: place new panes, defer moves of working ones.
- Plugin lives in quaqua; quaqua goes public after a full-history audit.
- All review findings accepted.

## Contract (generic, documented in `plugins/herdr-grid/README.md`)
| token | set by | value |
|---|---|---|
| `grid_role` | spawn scripts | `worker` \| `orchestrator` |
| `grid_group` | spawn-worker | project slug (`<repo>`) |
| `grid_owner` | spawn-worker / spawn-lead | owner **agent name** (never a pane id: ids change on move) |
| `grid_ready` | spawn-worker, **last step** | `1` |

- Source always `--source herdr-grid`; no TTL.
- Plugin only touches panes with `grid_ready=1`. Untagged/foreign panes are never moved. After a herdr restart tokens are gone → panes are left alone (no re-tag).
- Layout per owner (in the owner's workspace): 1 worker in total → split beside owner 2/5 | 3/5. Otherwise one tab `<group>-workers` per group: 2 = left|right, 3 = A|B + C, 4 = 2x2. >4 → reject (log + `--check` fails).

---

## Phase 1 — Enforce (independent; ~1 builder + 1 reviewer)
Steps:
1. **Worker push block.** `scripts/worker/spawn-worker.sh`: after worktree add, `git -C "$wt" config extensions.worktreeConfig true` + `git -C "$wt" config --worktree remote.origin.pushurl no-push://blocked` (each remote). → verify: `tests/test_spawn_worker.sh` new case: `git -C "$wt" push` fails, `git -C ~/quaqua push --dry-run` still resolves origin.
2. **Machine-path pre-commit.** New `scripts/hooks/pre-commit` (bash 3.2): staged added lines matching `(^|[[:space:]"'=:(])/(Users|home)/[a-z]`, allowlist `rules/_all/portable-paths.md`, `tests/**`. Install via `scripts/init-workspace.sh` (`git config core.hooksPath scripts/hooks`). → verify: new `tests/test_pre_commit.sh` (hit blocked, fixture allowed, `$HOME` ok).
3. **Run-report grammar.** `scripts/worker/prompt-worker.sh`: before printing the path, require last-section `## Lessons` (builders) or a `Verdict:` line (names `quansu-*` / pipeline reviewers); malformed → exit 6 + `WATCHDOG malformed-report`. Keep size/mtime stability check. → verify: `tests/test_prompt_worker.sh` cases: ok builder, ok reviewer, missing Lessons, truncated.
4. **Gate `--help`.** `scripts/hooks/herdr-gate.sh`: allow any blocked verb with `--help|-h`. → verify: `tests/test_herdr_gate.sh` case.
5. Docs: `AGENTS.md:52` exit-code list +6; `rules/_all/brief.md` drop the atomic-publish prose duplicated by the check (keep 1 line).
Exit: `bash tests/run.sh` green; quansu review clean.

## Phase 2 — Probe (scratch workspace only; detu/sude, ~30 min pilot)
Each item = command + observed output in `runs/<name>.md`:
1. `herdr plugin link plugins/herdr-grid-probe` (logging-only hooks for the 4 events) → `herdr plugin list --json` warnings empty.
2. Script split in scratch workspace → `herdr plugin log list --plugin <id>` shows `pane.created` with `HERDR_PLUGIN_EVENT_JSON`; record PATH, `bash --version`.
3. `report-metadata --source herdr-grid --token grid_ready=1` → token in `herdr api snapshot`; `herdr pane move` → token survives, new id = `previous_pane_id` mapping.
4. Agent integration metadata report on the same pane does not clear `herdr-grid` tokens.
5. `layout.apply` via socket on a 2-pane tab → processes alive, pane ids unchanged or remapped, no scratch tab needed.
6. `agent_status_changed` fires on working→idle.
Exit: all 6 answered. Any "no" → plan amendment before Phase 3 (fallbacks: scratch-tab moves for 5; re-tag on move for 3).

## Phase 3 — Plugin in shadow mode (sude build, quansu review)
Files:
- `plugins/herdr-grid/herdr-plugin.toml`: `[[events]]` for `pane.created`, `pane.closed`, `pane.agent_detected`, `pane.agent_status_changed` → `["bash","hook.sh"]`; `[[actions]] reconcile`, `check`.
- `hook.sh`: increments `pending` in `$HERDR_PLUGIN_STATE_DIR/<session>/`; tries `mkdir lock` (pid file, reclaim if pid dead); holder loops: snapshot → plan → (shadow: log plan) until `pending` unchanged since read; then rmdir. Status-changed hook exits early unless `deferred` file non-empty.
- `plan.jq`: snapshot → ops list (`split|move|apply|defer|reject`). Pure, bash-3.2 free.
- `apply.sh`: Phase 4 only; `layout.apply` or move-with-remap per Phase 2; verify each op from returned ids.
- herdr via `$HERDR_BIN_PATH`.
Script changes: `spawn-worker.sh` tags role/group/owner after split and `grid_ready=1` after its trust/send-keys handling (`:139–153`); `spawn-lead.sh` tags Lead `grid_role=orchestrator`; CoS tagged by `init-workspace.sh`. `grid-workers.sh` unchanged (still the arranger).
Tests (`tests/test_herdr_grid_plan.sh`, fixture snapshots): 0–5 workers; two owners same group; two sessions; untagged pane in tab; working deferral; second run → zero ops. Executor tests with stub herdr: pane closed mid-apply, nth move fails, id remap, zoomed tab, event during unlock, lock holder killed.
Exit: tests green; plugin warnings empty; shadow log plan == `grid-workers.sh` result over ≥5 real spawns.

## Phase 4 — Cutover
1. `hook.sh` applies ops (shadow flag off).
2. `grid-workers.sh` → `--check` (fail only on foreign/untagged-in-group or >4; deferred ops printed, pass) + `--repair` (old mover, kept one cycle).
3. `spawn-worker.sh:159–177`, `cleanup-worker.sh:16–30` call `--check`; `prompt-worker.sh` runs `--check` before `agent prompt` (fail-closed).
4. `pipeline.sh:292–333` grid pause → `--check`; `spawn-lead.sh:132` brief text updated.
Exit: tests green; 1 real fanout (3 workers) + 1 cleanup arranged by the plugin alone.

## Phase 5 — Cut prose (≥1 week after Phase 4 with no `--repair` use)
- Delete `rules/_all/fanout-agent-grid.md`, `.claude/rules/herdr-fanout-grid.md`; `AGENTS.md:51,56` drop grid steps; fix pointers in `herdr-gate.sh:30–31` and pipeline blocker text.
- `rules/_all/worker-timebox.md` → 3 lines (80% nudge → wrap up + `## Blockers`; pilot 30 min). Exit codes/`--resume` → skill `herdr-orchestration`.
- `rules/_all/portable-paths.md` → deleted (hook enforces); `handoff.md` keep minus push line (git config enforces).
Exit: `wc -c rules/_all/*.md` total ≤1500; `tests/test_docs.sh` green.

## Phase 6 — Publish (anh iu approves each step)
1. Audit: `git log --all -p` + refs/tags for `memory/`, `raw/`, `runs/`, `/Users/`, emails, tokens (gitleaks if installed, else grep). Report only.
2. Decision on findings (history rewrite = separate approval).
3. Repo → public; add topic `herdr-plugin`; confirm `herdr plugin install therealtinhtute/quaqua/plugins/herdr-grid` works on a clean config.

## Risks
- herdr has no veto: plugin is post-hoc; `--check` is the pre-prompt barrier.
- pi/codex ignore Claude hooks → push block lives in git config, not the gate.
- Public history exposure; rewrite needs approval.
- Phase 2 "no" answers can reshape Phase 3–4.
