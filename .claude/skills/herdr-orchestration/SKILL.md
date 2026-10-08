---
name: herdr-orchestration
description: Spawn, prompt, monitor, and clean up worker agents through herdr. Use for every delegation — never native subagents.
---

# herdr orchestration

Precondition: `test "${HERDR_ENV:-}" = 1`, else tell anh iu and stop.

## Flow
1. Pick kind from `models.md`. Max 3 live workers (`herdr agent list`).
2. `scripts/spawn-worker.sh <name> <kind> <repo>` → prints pane + worktree. **MANDATORY: then `scripts/grid-workers.sh` before any prompt** (rules/_all/fanout-agent-grid.md). It injects `rules/_all/*.md` + `rules/<repo-basename>/*.md` as the worker's system prompt (copy at `runs/<name>.rules.md`); don't repeat those rules in the prompt.
3. Prompt with objective + constraints + evidence required:
   `herdr agent prompt <name> "<task>. When done, write summary (changes, test cmd+result, risks) to ~/quaqua/runs/<name>.md; reply only the path." --wait --timeout 1800000`
4. Read `runs/<name>.md`; re-run its test yourself. Then update memory per AGENTS.md (rewrite card `## Now`, one line in today's `memory/YYYY-MM-DD.md`).
5. Cleanup right after the run is verified (report read + test re-run passes), no approval needed: `scripts/cleanup-worker.sh <name> <repo>`, then `scripts/grid-workers.sh`. Branch kept; merge/push/branch delete still need approval.

Name: `[a-z][a-z0-9_-]{0,31}`, unique, and branch `quaqua/<name>` must not exist yet.

## When something goes wrong
Wait returned `blocked`, `timeout`, `agent_prompt_stalled`, `agent_not_ready`, or state `unknown` → read `references/recovery.md` before any input.
