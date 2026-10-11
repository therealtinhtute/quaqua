#!/usr/bin/env bash
# PreToolUse(Bash) hook for the CoS and Leads (.claude/settings.json): pane
# placement and agent spawning go through scripts/ only (spawn-worker.sh,
# spawn-lead.sh, grid-workers.sh, cleanup-worker.sh), so the harness — not
# the agent — decides tabs, owners and the 4-worker cap. A direct
# `herdr pane split|move`, `herdr tab create` or `herdr agent start` is
# blocked (exit 2; stderr goes back to the agent).
set -euo pipefail
cmd=$(jq -r '.tool_input.command // ""')
if grep -Eq '(^|[;&|(`[:space:]])herdr[[:space:]]+(pane[[:space:]]+(split|move)|tab[[:space:]]+create|agent[[:space:]]+start)([[:space:]]|$)' <<<"$cmd"; then
  echo "blocked: spawn/move panes only via scripts/ (spawn-worker.sh, spawn-lead.sh, grid-workers.sh, cleanup-worker.sh) — rules/_all/fanout-agent-grid.md" >&2
  exit 2
fi
exit 0
