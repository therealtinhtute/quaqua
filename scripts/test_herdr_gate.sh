#!/usr/bin/env bash
# Test for hooks/herdr-gate.sh: direct herdr spawn/move calls are blocked,
# scripts/ and read-only herdr calls pass.
# Usage: scripts/test_herdr_gate.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
fail() { echo "FAIL: $*" >&2; exit 1; }
gate() {  # gate <command>; prints the hook's exit code
  local rc=0
  jq -nc --arg c "$1" '{tool_name:"Bash",tool_input:{command:$c}}' \
    | bash "$here/hooks/herdr-gate.sh" 2>/dev/null || rc=$?
  echo "$rc"
}

for c in 'herdr pane split --current --direction right' \
         'herdr pane move w1:p5 --new-tab --label x' \
         'herdr tab create --label x --no-focus' \
         'herdr agent start sude-x --kind pi --pane w1:p5' \
         'cd ~/quaqua && herdr pane split --current' \
         'X=$(herdr tab create --label y)'; do
  [[ $(gate "$c") == 2 ]] || fail "not blocked: $c"
done
echo "ok: direct herdr split/move/tab create/agent start -> blocked"

for c in 'scripts/spawn-worker.sh sude-x sude ~/repo' \
         'scripts/grid-workers.sh --tab x-workers --dry-run' \
         'herdr agent list' 'herdr agent read sude-x --lines 80' \
         'herdr pane send-keys w1:p5 enter' 'herdr tab list' \
         'grep -n "herdr pane split" scripts/spawn-worker.sh'; do
  [[ $(gate "$c") == 0 ]] || fail "wrongly blocked: $c"
done
echo "ok: scripts/ and read-only herdr calls -> allowed"

echo "PASS"
