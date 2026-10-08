---
name: herdr-orchestration
description: Herdr command recipes and worker spawn, prompt, monitoring, recovery, and cleanup. Use before every delegation or herdr pane/agent/tab command; never native subagents.
---

# Herdr orchestration

Authority: `AGENTS.md`, `models.md`, and `rules/_all/`; references explain mechanics, not new roles.
Precondition: `test "${HERDR_ENV:-}" = 1`, else tell anh iu and stop. Never run bare `herdr` (it attaches the TUI). `herdr --help` / `herdr <group>` are the syntax authority.

## Flow
1. Pick kind from `models.md`. Max 4 task workers per orchestrator; a pipeline counts as 2. Project Leads do not count toward that cap. Query `herdr agent list`, never infer live names from memory.
2. `scripts/spawn-worker.sh [--base <ref>] <name> <kind> <repo>` → prints pane + worktree. **MANDATORY: then `scripts/grid-workers.sh` before any prompt** (`rules/_all/fanout-agent-grid.md`). It injects `rules/_all/*.md` + `rules/<repo-basename>/*.md` as the worker's system prompt (`runs/<name>.rules.md`); don't repeat those rules in the brief.
3. Use the five-part brief from `rules/_all/brief.md`; set `PROMPT_WORKER_BUDGET_SEC` to its time budget. `scripts/prompt-worker.sh <name> "<brief>"` submits with `--wait`, then watches for an atomically published `runs/<name>.md`. A pipeline-size builder plans before code; its project Lead approves via `pipeline.sh --resume` after the plan-ready pause.
4. Read the report and re-run its test yourself before reporting completion. Update the project card's `## Now` and today's `memory/YYYY-MM-DD.md` per `AGENTS.md`.
5. Cleanup immediately after the verified outcome, no extra approval: `scripts/cleanup-worker.sh <name> <repo>`, then `scripts/grid-workers.sh`. Branch kept. Only the project Lead accepts into its integration branch; main merge, push, and branch deletion need the Principal's approval.

Worker names: `<kind>-<task>`, ASCII `[a-z0-9-]`, at most 20 characters, never ending `-lead` (`spawn-worker.sh`). A failed startup may reuse its registered worktree; a foreign directory is refused. Lead names: `<slug>-lead`, Herdr's 32-character limit (`spawn-lead.sh`).

## References
- Read `references/operations.md` before manual pane/tab commands, layout recovery, plugin operations, or launcher/environment advice.
- Read `references/recovery.md` on watchdog exits 2–5, blocked/timeout/stalled/not-ready/unknown state, before any input.
- Read `references/judgment.md` for neutral assignments, ownership, risk-driven extra review, and acceptance evidence. Independent bounded-task handbacks retain the contract in `PEER.md`.
