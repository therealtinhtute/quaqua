# herdr-grid plugin + instruction-layer minimisation

Status: revised after review (raw/2026-10-12-quaqua-bacdau-gridplan.md, raw/2026-10-12-quaqua-truonglao-gridplan.md). Awaiting anh iu approval.

## Goal
Replace LLM-followed prose with code: a generic herdr layout reconciler plugin (published from a quaqua subdir), plus script enforcement for mechanical rules. `rules/_all` shrinks from ~7.5 KB to ~1.5 KB.

## Decisions (anh iu, 2026-10-12)
- Event hooks + snapshot reconcile, no daemon. Pane-token contract. Defer moves of working panes. Plugin in `plugins/herdr-grid/`; quaqua itself goes public (full history audit first).
- Review findings accepted in full.

## Contract
- Tokens (fixed `--source herdr-grid`): `grid_role=worker|orchestrator`, `grid_group=<project>`, `grid_owner=<owner agent name>` (never a pane id — ids change on move), `grid_ready=1` set **last** by spawn (after trust/prompt handling).
- Plugin touches only panes with `grid_ready=1`; untagged/foreign panes are never moved. Tokens are not persisted across herdr restart → after restart panes are untagged and left alone (no startup re-tag).
- Formula per owner: 1 worker in total → split beside owner (2/5 | 3/5); else one tab per group in the owner's workspace: 2 = left|right, 3 = A|B + C, 4 = 2x2; >4 rejected.

## Reconciler
- Hooks: `pane.created`, `pane.closed`, `pane.agent_detected`, `pane.agent_status_changed` (fast exit unless deferred work is pending). Never `pane.moved`/`pane.updated`/`layout.updated`.
- Lock: `mkdir` lock in `$HERDR_PLUGIN_STATE_DIR/<session>/` with pid + stale reclaim; pending counter (not boolean) re-checked before unlock. No `flock`; bash-3.2 safe; planner in jq; herdr via `$HERDR_BIN_PATH`.
- Apply: prefer `layout.apply` per tab (probe first); fallback = existing scratch-tab moves with id remap and post-move verification.
- Working panes: deferred; `--check` passes with only deferred ops (prints them), fails only on foreign/untagged-in-group panes or >4.

## Phases (each stands alone)
1. **Enforce** (independent, cheap)
   - Worker push block: spawn sets worktree-local `remote.*.pushurl` to an invalid URL (harness-agnostic); CoS unaffected.
   - Pre-commit: anchored machine-path check `(^|[[:space:]"'=:(])/(Users|home)/[a-z]` with fixture/policy allowlist; document hook install.
   - Run reports: role-aware grammar check (`## Lessons` for builders, `Verdict:` for reviewers) in `prompt-worker.sh`; atomic `.tmp→mv` stays a producer contract.
   - Gate: allow `--help`/`-h`.
2. **Probe** (scratch workspace): hooks fire for script-created panes (`herdr plugin log list`); tokens survive `pane move`; source scoping vs agent integration reports; `layout.apply` keeps processes/ids; hook PATH/bash; link warnings empty.
3. **Plugin, shadow mode** — plugin computes + logs plan only; `grid-workers.sh` stays the synchronous arranger. Spawn/cleanup/`spawn-lead.sh`/`pipeline.sh` set tokens. Tests: planner (0–5 workers, two owners same group, mixed sessions, deferral, idempotence), executor (closed pane mid-apply, failed nth move, id remap, zoomed tab, event during unlock, lock-holder death). Exit: plugin warnings empty; shadow plan == actual layout over real spawns.
4. **Cutover** — plugin applies; `grid-workers.sh` → `--check` + `--repair` (old mover kept one cycle); `pipeline.sh` grid pause uses `--check`.
5. **Cut prose** (after ≥1 week clean) — delete `rules/_all/fanout-agent-grid.md`, `.claude/rules/herdr-fanout-grid.md`, grid steps in `AGENTS.md:51,56`, update `herdr-gate.sh` / pipeline text pointing at them; trim `worker-timebox.md` to worker-facing lines (80% nudge, `## Blockers`, pilot 30 min), rest → skill `herdr-orchestration`.
6. **Publish** (anh iu approves) — audit full git history + refs for `memory/`, `raw/`, `runs/`, personal paths/secrets; make quaqua public; add topic `herdr-plugin`.

## Risks
- No veto in herdr: plugin is post-hoc; pi/codex bypass Claude hooks (push block moved to git config).
- Making quaqua public exposes its history; a history rewrite would itself need approval.
