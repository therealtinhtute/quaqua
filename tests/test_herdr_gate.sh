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
# eval, prefix wrappers, command substitutions, keywords. The last four are
# finding 1 (runs/suhuynh-r5r1.md): an empty word from an unquoted command
# substitution vanishes under bash's TAB/IFS re-split, so the herdr call after
# it is still the executable and must block.
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
         'herdr >&2 pane split' \
         '$(true) herdr pane split' \
         '`:` herdr tab create --label x' \
         'sudo $(true) herdr workspace close w7' \
         'X=1 $(true) herdr agent start x'; do
  [[ $(gate "$c") == 2 ]] || fail "not blocked: $c"
done
echo "ok: quoted/pathed/-c/eval/wrapper/substitution spellings -> blocked"

for c in 'quaqua worker spawn sude-x sude ~/repo' \
         'quaqua grid reconcile --tab x-workers --dry-run' \
         'herdr agent list' 'herdr agent read sude-x --lines 80' \
         'herdr pane send-keys w1:p5 enter' 'herdr tab list' \
         'herdr workspace list' 'herdr workspace get w7' 'quaqua lead cleanup baro' \
         'grep -n "herdr pane split" crates/quaqua/src/lib.rs' \
         'quaqua worker spawn sude-x sude ~/repo' \
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

# herdr honors --help/-h only as the first word after `herdr` (top-level
# help) or as the first word after the guarded verb pair (that verb's help);
# it prints usage and exits without running the verb. Verified against
# herdr 0.9.3: `herdr <verb> --help` and `herdr <verb> -h` are byte-identical
# rc=0 usage text on all six guarded verbs, and none of those help texts
# documents -h as anything but help.
for c in 'herdr pane split --help' 'herdr pane split -h' \
         'herdr pane move --help' 'herdr pane move -h' \
         'herdr tab create --help' 'herdr tab create -h' \
         'herdr workspace create --help' 'herdr workspace create -h' \
         'herdr workspace close --help' 'herdr workspace close -h' \
         'herdr agent start --help' 'herdr agent start -h' \
         'herdr --help' 'herdr -h' \
         'herdr --help pane split' 'herdr -h tab create' \
         'herdr --session quaqua pane split --help' \
         'herdr --session quaqua workspace close -h' \
         'herdr pane split --help --current' \
         'bash -c '\''herdr agent start --help'\''' \
         'sudo herdr tab create -h' \
         'X=$(herdr pane move -h)'; do
  [[ $(gate "$c") == 0 ]] || fail "wrongly blocked: $c"
done
echo "ok: --help/-h at a help position -> allowed"

# a help-shaped word anywhere else is not help: herdr reads it as an option
# value (`herdr tab list --workspace --help` looks up a workspace literally
# named "--help") or rejects it as an unknown option while the real command
# still runs — so the gate must keep blocking, and near-miss words stay 2.
for c in 'herdr pane split --pane --help' \
         'herdr pane split --current --help' \
         'herdr pane split nosuchpane -h' \
         'herdr tab create --label --help' \
         'herdr tab create --workspace x -h' \
         'herdr workspace close --group --help' \
         'herdr agent start --kind --help --pane x' \
         'herdr agent start sude-x --kind pi --pane w1:p5 --help' \
         'herdr pane split --hel' \
         'herdr pane split --helpful' \
         'herdr pane split -H' \
         'herdr pane split -help' \
         'herdr pane split -hx' \
         'herdr pane split x -h'; do
  [[ $(gate "$c") == 2 ]] || fail "not blocked: $c"
done
echo "ok: help-shaped words outside a help position -> blocked"

echo "PASS"
