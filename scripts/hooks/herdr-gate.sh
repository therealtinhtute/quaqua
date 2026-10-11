#!/usr/bin/env bash
# PreToolUse(Bash) hook for the CoS and Leads (.claude/settings.json): pane
# placement and agent spawning go through scripts/ only (spawn-worker.sh,
# spawn-lead.sh, grid-workers.sh, cleanup-worker.sh, cleanup-lead.sh), so the
# harness — not the agent — decides tabs, team workspaces, owners and the
# 4-worker cap. A direct `herdr pane split|move`, `herdr tab create`,
# `herdr workspace create|close` or `herdr agent start` is blocked (exit 2;
# stderr goes back to the agent).
set -euo pipefail
cmd=$(jq -r '.tool_input.command // ""')
if grep -Eq '(^|[;&|(`[:space:]])herdr[[:space:]]+(pane[[:space:]]+(split|move)|tab[[:space:]]+create|workspace[[:space:]]+(create|close)|agent[[:space:]]+start)([[:space:]]|$)' <<<"$cmd"; then
  echo "blocked: spawn/move panes and workspaces only via scripts/ (spawn-worker.sh, spawn-lead.sh, grid-workers.sh, cleanup-worker.sh, cleanup-lead.sh) — rules/_all/fanout-agent-grid.md" >&2
  exit 2
fi
exit 0
