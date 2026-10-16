#!/usr/bin/env bash
# PreToolUse gate (rules/_all/fanout-agent-grid.md). This hook MUST block with
# exit 2 whenever it cannot run the gate: Claude Code treats every exit code
# other than 2 as a non-blocking error, so a missing binary would silently
# remove the fence. Binary resolution: $QUAQUA_BIN, else `quaqua` on PATH.
bin=${QUAQUA_BIN:-quaqua}
if ! command -v "$bin" >/dev/null 2>&1; then
  echo "quaqua gate: '$bin' not found — install quaqua or set QUAQUA_BIN; blocking pane/workspace spawn+move (fail-closed)" >&2
  exit 2
fi
exec "$bin" gate "$@"
