#!/usr/bin/env bash
# Test for hooks/herdr-gate.sh: direct herdr spawn/move calls are blocked,
# scripts/ and read-only herdr calls pass.
# Usage: tests/test_herdr_gate.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
fail() { echo "FAIL: $*" >&2; exit 1; }
gate() {  # gate <command>; prints the hook's exit code
  local rc=0
  jq -nc --arg c "$1" '{tool_name:"Bash",tool_input:{command:$c}}' \
    | bash "$src/hooks/herdr-gate.sh" 2>/dev/null || rc=$?
  echo "$rc"
}

for c in 'herdr pane split --current --direction right' \
         'herdr pane move w1:p5 --new-tab --label x' \
         'herdr tab create --label x --no-focus' \
         'herdr agent start sude-x --kind pi --pane w1:p5' \
         'herdr workspace create --label baro --no-focus' \
         'herdr workspace close w7' \
         'cd ~/quaqua && herdr pane split --current' \
         'X=$(herdr tab create --label y)'; do
  [[ $(gate "$c") == 2 ]] || fail "not blocked: $c"
done
echo "ok: direct herdr split/move/tab+workspace create/workspace close/agent start -> blocked"

# equivalent spellings of the same calls must not slip past the gate:
# quoted/escaped executable, path-qualified executable, bash -c payload,
# eval, prefix wrappers, command substitutions, keywords
for c in '"herdr" pane split --current' \
         "'herdr' pane move w1:p5 --new-tab" \
         'h\e\rdr tab create --label x' \
         '/usr/local/bin/herdr workspace close w7' \
         './herdr tab create --label x' \
         'bash -c '\''herdr agent start sude-x --kind pi'\''' \
         'sh -c "herdr workspace create --label b"' \
         'bash -c '\''cd /tmp && herdr pane split'\''' \
         'eval '\''herdr pane split --current'\''' \
         'sudo herdr pane split --current' \
         'sudo -u root herdr workspace close w7' \
         'env FOO=1 herdr workspace close w7' \
         'FOO=bar herdr tab create' \
         'command herdr agent start sude-x' \
         'xargs herdr pane split' \
         'time herdr pane move w1:p5 --new-tab' \
         'if herdr workspace close w7; then echo x; fi' \
         'echo `herdr tab create`' \
         'y="$(herdr agent start z)"' \
         'herdr >&2 pane split'; do
  [[ $(gate "$c") == 2 ]] || fail "not blocked: $c"
done
echo "ok: quoted/pathed/-c/eval/wrapper/substitution spellings -> blocked"

for c in 'scripts/worker/spawn-worker.sh sude-x sude ~/repo' \
         'scripts/worker/grid-workers.sh --tab x-workers --dry-run' \
         'herdr agent list' 'herdr agent read sude-x --lines 80' \
         'herdr pane send-keys w1:p5 enter' 'herdr tab list' \
         'herdr workspace list' 'herdr workspace get w7' 'scripts/lead/cleanup-lead.sh baro' \
         'grep -n "herdr pane split" scripts/worker/spawn-worker.sh' \
         'bash scripts/worker/spawn-worker.sh sude-x sude ~/repo' \
         'bash -c '\''echo no herdr here'\''' \
         'echo $(herdr agent list)'; do
  [[ $(gate "$c") == 0 ]] || fail "wrongly blocked: $c"
done
echo "ok: scripts/ and read-only herdr calls -> allowed"

# herdr spelled inside argument text, assignments, heredocs and redirect
# targets is not an executable call — the gate must stay quiet
for c in 'printf '\''%s\n'\'' '\''example: herdr pane split --current'\''' \
         'printf "%s\n" "example: herdr pane split --current"' \
         'echo "see herdr tab create in docs"' \
         'note='\''herdr agent start'\''' \
         'cat <<< "herdr pane split"' \
         'var=$(printf "herdr agent start")' \
         'herdr agent prompt sude-x "please run herdr pane split"' \
         'echo hi >&2 && herdr agent list'; do
  [[ $(gate "$c") == 0 ]] || fail "wrongly blocked: $c"
done
[[ $(gate 'cat <<EOF
use herdr pane split to arrange
EOF') == 0 ]] || fail "wrongly blocked: heredoc body"
[[ $(gate 'cat <<-EOF
	herdr agent start sude-x
	EOF') == 0 ]] || fail "wrongly blocked: <<- heredoc body"
[[ $(gate 'cat <<EOF; herdr pane split
body
EOF') == 2 ]] || fail "not blocked: real command beside a heredoc"
echo "ok: argument strings, assignments, heredocs, redirect targets -> allowed"

echo "PASS"
