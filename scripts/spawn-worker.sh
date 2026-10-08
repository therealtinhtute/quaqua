#!/usr/bin/env bash
# Usage: spawn-worker.sh [--base <ref>] [--review] <name> <kind: tieude|tieunhi|sude|ni|suhuynh|truonglao|bacdau|quansu> <repo-path>
#   name = <kind>-<task>, ascii [a-z0-9-], <=20 chars, not ending in -lead
#          (*-lead is the project Lead convention — spawn-lead.sh <slug>-lead)
#   --base <ref>  start branch quaqua/<name> at <ref> instead of <repo> HEAD
#   --review      codex kinds (bacdau|quansu) only: repo read-only, $HOME/quaqua/runs writable
set -euo pipefail

usage() { sed -n '2,5p' "$0" >&2; exit 1; }

pos=()
while (($#)); do
  case $1 in
    --base)   (($# >= 2)) || usage; base=$2; shift 2 ;;
    --base=*) base=${1#--base=}; shift ;;
    --review) review=1; shift ;;
    *)        pos+=("$1"); shift ;;
  esac
done
if [[ ${base+x} && ( -z $base || $base == -* ) ]]; then usage; fi
if ((${#pos[@]} < 3)); then usage; fi
name=${pos[0]} kind=${pos[1]} repo=$(realpath "${pos[2]}")
case $kind in
  tieude|tieunhi|sude|ni|suhuynh|truonglao|bacdau|quansu) ;;
  *) sed -n '2p' "$0" >&2; exit 1 ;;
esac
if [[ ! $name =~ ^$kind-[abcdefghijklmnopqrstuvwxyz0123456789-]+$ ]] || (( ${#name} > 20 )) \
    || [[ $name == *-lead ]]; then
  sed -n '2,3p' "$0" >&2; exit 1
fi
case $kind in bacdau|quansu) ;; *)
  if [[ ${review+x} ]]; then
    echo "error: --review is only supported for codex kinds (bacdau, quansu), not $kind" >&2
    usage
  fi ;;
esac

wt="$(dirname "$repo")/.wt/$name"

# --base must resolve to a commit before anything is created (worktree, rules, pane)
if [[ ${base+x} ]]; then
  git -C "$repo" rev-parse --verify --quiet --end-of-options "${base}^{commit}" >/dev/null \
    || { echo "error: --base '$base' is not a known commit-ish in $repo" >&2; exit 1; }
fi

git -C "$repo" worktree add -b "quaqua/$name" "$wt" ${base:+"$base"} >&2

# worker rules: rules/_all/*.md + rules/<repo-basename>/*.md -> one file, injected as system prompt
rules_dir="$HOME/quaqua/rules" rf="$HOME/quaqua/runs/$name.rules.md"
cat "$rules_dir"/_all/*.md "$rules_dir/$(basename "$repo")"/*.md 2>/dev/null > "$rf" || true
codex_rules="developer_instructions=$(jq -Rs . < "$rf")"

# codex access args. --review keeps the repo read-only and $HOME/quaqua/runs writable
# via a permission profile extending the built-in :read-only one. Plain
# `-s read-only` cannot do this: codex only honours --add-dir under
# workspace-write (a read-only sandbox makes runs/ read-only too), and
# --approve-for-me conflicts with an explicit -s.
if [[ ${review+x} ]]; then
  access=(-a never -c 'default_permissions="quaqua-review"'
          -c 'permissions.quaqua-review.extends=":read-only"'
          -c "permissions.quaqua-review.filesystem={\"$HOME/quaqua/runs\"=\"write\"}")
else
  access=(--approve-for-me --add-dir "$HOME/quaqua/runs")
fi

split=$(herdr pane split --current --direction right --no-focus --cwd "$wt")
pane=$(printf '%s\n' "$split" | jq -r '.result.pane.pane_id')

case $kind in
  tieude|sude|tieunhi)
    case $kind in
      tieude) args=(--model devin/deepseek-v4.1-flash --thinking max) ;;
      sude) args=(--model devin/swe-2 --thinking max) ;;
      tieunhi) args=(--model antigravity/gemini-3.8-flash --thinking high) ;;
    esac
    herdr agent start "$name" --kind pi --pane "$pane" --timeout 60000 -- "${args[@]}" --append-system-prompt "$rf" >/dev/null ;;
  ni|suhuynh|truonglao)
    case $kind in ni) effort=low ;; suhuynh) effort=medium ;; truonglao) effort=high ;; esac
    herdr agent start "$name" --kind claude --pane "$pane" --timeout 60000 -- --model claude-opus-5-5 --effort "$effort" --append-system-prompt-file "$rf" >/dev/null ;;
  bacdau)
    herdr agent start "$name" --kind codex --pane "$pane" --timeout 60000 -- "${access[@]}" -m astra -c model_reasoning_effort=medium -c "$codex_rules" >/dev/null || true ;;
  quansu)
    herdr agent start "$name" --kind codex --pane "$pane" --timeout 60000 -- "${access[@]}" -m gpt-6.1-sol -c model_reasoning_effort=high -c "$codex_rules" >/dev/null || true ;;
esac
case $kind in bacdau|quansu)
  # new worktree -> codex asks "Trust this folder?"; accept default "Trust and continue".
  # start may already have failed with agent_not_ready while the dialog blocked readiness, so confirm idle here
  if herdr pane wait-output "$pane" --match "Trust this folder" --timeout 10000 >/dev/null 2>&1; then
    herdr pane send-keys "$pane" enter >/dev/null
  fi
  herdr agent wait "$name" --until idle --timeout 60000 >/dev/null
esac

# Grid hook (rules/_all/fanout-agent-grid.md): arrange the new worker's pane
# around the caller's pane. Best-effort: a grid failure warns, never fails the
# spawn. GRID_TAB=<label> selects the 2-4 worker tab; GRID_SKIP=1 disables.
grid_hook() {
  [[ "${GRID_SKIP:-}" == 1 ]] && return 0
  local grid args=()
  grid="$(dirname "$0")/grid-workers.sh"
  [[ -f "$grid" ]] || return 0
  [[ -n "${GRID_TAB:-}" ]] && args=(--tab "$GRID_TAB")
  bash "$grid" "${args[@]}" >&2 \
    || echo "warning: grid-workers.sh failed (exit $?); worker $name spawned, arrange panes manually" >&2
}

echo "worker=$name kind=$kind pane=$pane worktree=$wt"
grid_hook
