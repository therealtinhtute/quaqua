#!/usr/bin/env bash
# Usage: cleanup-worker.sh <name> <repo-path>
# Stops the agent pane and removes its worktree. Branch quaqua/<name> is kept.
# Best-effort hook (never blocks cleanup): re-arranges the remaining worker
# panes (grid-workers.sh).
set -euo pipefail

name=$1 repo=$(realpath "$2")
wt="$(dirname "$repo")/.wt/$name"

# Grid hook (rules/_all/fanout-agent-grid.md): re-arrange the remaining workers
# around the caller's pane. Best-effort: a grid failure warns, never blocks the
# cleanup. 2-4 workers stay in tab GRID_TAB (default <repo-basename>-workers).
# GRID_SKIP=1 is honoured only for pipeline.sh (PIPELINE_TASK) or tests
# (QUAQUA_TEST=1); otherwise it is ignored.
grid_hook() {
  if [[ "${GRID_SKIP:-}" == 1 ]]; then
    [[ -n "${PIPELINE_TASK:-}" || "${QUAQUA_TEST:-}" == 1 ]] && return 0
    echo "warning: GRID_SKIP=1 ignored outside pipeline.sh/tests (rules/_all/fanout-agent-grid.md)" >&2
  fi
  local grid
  grid="$(dirname "$0")/grid-workers.sh"
  [[ -f "$grid" ]] || return 0
  bash "$grid" --tab "${GRID_TAB:-"$(basename "$repo")-workers"}" >&2 \
    || echo "warning: grid-workers.sh failed (exit $?); arrange panes manually" >&2
}

pane=$(herdr agent get "$name" | jq -r '.result.agent.pane_id')
herdr pane close "$pane" >/dev/null
grid_hook
git -C "$repo" worktree remove "$wt"
runs_dir="${QUAQUA_RUNS_DIR:-$HOME/quaqua/runs}"
rm -f "$runs_dir/$name.owner" "$runs_dir/$name.grid" "$runs_dir/$name.pipeline" "$runs_dir/$name.srt.json"
echo "cleaned $name (branch quaqua/$name kept)"
