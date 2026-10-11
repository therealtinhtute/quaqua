#!/usr/bin/env bash
# Usage: spawn-worker.sh [--base <ref>] [--review] <name> <kind: tieude|detu|sude|ni|suhuynh|truonglao|bacdau|quansu> <repo-path>
#   name = <kind>-<task>, ascii [a-z0-9-], <=20 chars, not ending in -lead
#          (*-lead is the project Lead convention — spawn-lead.sh <slug>-lead)
#   --base <ref>  start branch quaqua/<name> at <ref> instead of <repo> HEAD
#   --review      codex kinds (bacdau|quansu) only: repo read-only, $HOME/quaqua/runs writable
#   Resume-safe: when $wt is already a registered worktree of <repo> (a
#   previous attempt died after creating it) it is reused and startup
#   continues with a fresh pane/agent. A pane this attempt created is closed
#   if the agent never becomes ready; the worktree+branch stay for resume.
#   A dir at $wt that is not a worktree of <repo> is refused, never reused.
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
  tieude|detu|sude|ni|suhuynh|truonglao|bacdau|quansu) ;;
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

# Worker cap: max 4 task workers per orchestrator; pipelines reserve 2
owner=${QUAQUA_OWNER:-cos}
runs_dir="$HOME/quaqua/runs"
mkdir -p "$runs_dir"

count_owner_slots() {
  local o=${1:-cos} exclude=${2:-}
  local agents live_workers=() o_live=0 res_slots=0

  agents=$(herdr agent list 2>/dev/null || true)
  if jq -e . >/dev/null 2>&1 <<<"$agents"; then
    while IFS= read -r a_name; do
      [[ -n $a_name ]] || continue
      [[ $a_name == "$exclude" ]] && continue
      live_workers+=("$a_name")
    done < <(jq -r '.result.agents[]?
      | select((.agent_status // "") != "dead")
      | select((.name // "") | test("^(tieude|detu|sude|ni|suhuynh|truonglao|bacdau|quansu)-") and (test("-lead$") | not))
      | .name' <<<"$agents" 2>/dev/null || true)
  fi

  local o_workers=()
  for w in ${live_workers[@]+"${live_workers[@]}"}; do
    local w_owner
    w_owner=$(cat "$runs_dir/$w.owner" 2>/dev/null || true)
    w_owner=${w_owner:-cos}
    if [[ "$w_owner" == "$o" ]]; then
      (( o_live++ )) || true
      o_workers+=("$w")
    fi
  done

  for s in "$runs_dir"/pipeline-*.state; do
    [[ -f $s ]] || continue
    grep -q '^finished=0$' "$s" || continue
    local p_lead p_o p_pid p_b p_t p_bname p_alive=0
    p_lead=$(sed -n 's/^lead=\(.*\)/\1/p' "$s" | head -1)
    p_o=${p_lead:-cos}
    [[ "$p_o" == "$o" ]] || continue

    p_pid=$(sed -n 's/^pipeline_pid=\([0-9][0-9]*\).*/\1/p' "$s" | head -1)
    p_b=$(sed -n 's/^builder=\(.*\)/\1/p' "$s" | head -1)
    p_t=$(sed -n 's/^task=\(.*\)/\1/p' "$s" | head -1)
    p_bname="$p_b-$p_t"

    if [[ -n $p_pid ]] && kill -0 "$p_pid" 2>/dev/null; then
      p_alive=1
    else
      for a in ${live_workers[@]+"${live_workers[@]}"}; do
        if [[ "$a" == "$p_bname" ]]; then
          p_alive=1
          break
        fi
      done
    fi

    if (( p_alive )); then
      local p_cur_live=0
      for w in ${o_workers[@]+"${o_workers[@]}"}; do
        local w_pipe
        w_pipe=$(cat "$runs_dir/$w.pipeline" 2>/dev/null || true)
        if [[ "$w_pipe" == "$p_t" || "$w" == "$p_bname" || "$w" == *-pr[0-9]* || "$w" == *-${p_t}r[0-9]* ]]; then
          (( p_cur_live++ )) || true
        fi
      done
      if (( p_cur_live < 2 )); then
        (( res_slots += (2 - p_cur_live) )) || true
      fi
    fi
  done

  echo $(( o_live + res_slots ))
}

if [[ -z "${PIPELINE_TASK:-}" ]]; then
  slots_used=$(count_owner_slots "$owner" "$name")
  if (( slots_used >= 4 )); then
    echo "error: worker cap exceeded ($slots_used live worker slots for owner '$owner'; cap is 4) — cannot spawn $name" >&2
    exit 1
  fi
fi

if [[ -d $wt ]]; then
  if git -C "$repo" worktree list --porcelain | grep -qxF "worktree $wt"; then
    echo "note: $wt already a registered worktree — reusing it (resumed spawn)" >&2
  else
    echo "error: $wt exists but is not a git worktree of $repo — remove it or pick another name" >&2
    exit 1
  fi
else
  git -C "$repo" worktree add -b "quaqua/$name" "$wt" ${base:+"$base"} >&2
fi

# worker rules: rules/_all/*.md + rules/<repo-basename>/*.md -> one file, injected as system prompt
rules_dir="$HOME/quaqua/rules" rf="$HOME/quaqua/runs/$name.rules.md"
mkdir -p "$(dirname "$rf")"   # fresh clones have no runs/ yet — create it before writing
cat "$rules_dir"/_all/*.md "$rules_dir/$(basename "$repo")"/*.md 2>/dev/null > "$rf" || true
codex_rules="developer_instructions=$(jq -Rs . < "$rf")"

# Record owner, pipeline association, and sandbox marker
echo "$owner" > "$runs_dir/$name.owner"
[[ -n "${PIPELINE_TASK:-}" ]] && echo "$PIPELINE_TASK" > "$runs_dir/$name.pipeline"
iso_now=$(date -Iseconds 2>/dev/null || date '+%Y-%m-%dT%H:%M:%S%z')
printf '{"name":"%s","kind":"%s","created":"%s","sandbox":true,"owner":"%s"}\n' \
  "$name" "$kind" "$iso_now" "$owner" > "$runs_dir/$name.srt.json"

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

split=$(herdr pane split --current --direction right --no-focus --cwd "$wt" --env "QUAQUA_SANDBOX=1" --env "QUAQUA_OWNER=$owner")
pane=$(printf '%s\n' "$split" | jq -r '.result.pane.pane_id')

# if the agent never becomes ready, roll back only what this attempt created
# (the pane); the worktree+branch stay so a --resume can reuse them (F4)
spawn_fail() { herdr pane close "$pane" >/dev/null 2>&1 || true; exit "${1:-1}"; }

case $kind in
  tieude|sude|detu)
    case $kind in
      tieude) args=(--model devin/deepseek-v4.1-flash --thinking max) ;;
      sude) args=(--model devin/swe-2 --thinking max) ;;
      detu) args=(--model antigravity/gemini-3.8-flash --thinking high) ;;
    esac
    herdr agent start "$name" --kind pi --pane "$pane" --timeout 60000 -- "${args[@]}" --append-system-prompt "$rf" >/dev/null || spawn_fail ;;
  ni|suhuynh|truonglao)
    case $kind in ni) effort=low ;; suhuynh) effort=medium ;; truonglao) effort=high ;; esac
    herdr agent start "$name" --kind claude --pane "$pane" --timeout 60000 -- --model claude-opus-5-5 --effort "$effort" --append-system-prompt-file "$rf" >/dev/null || spawn_fail ;;
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
  herdr agent wait "$name" --until idle --timeout 60000 >/dev/null || spawn_fail
esac
case $kind in tieude|sude|detu)
  # project MCP config -> pi asks "Allow project MCP server <name>?" once per
  # server while herdr already reports idle; a prompt sent then lands in the
  # dialog and is lost. Answer Allow (anh iu, 2026-10-11) before any prompt.
  if [[ -f $wt/.mcp.json || -f $wt/.pi/mcp.json || -f $wt/.vscode/mcp.json ]]; then
    wait_ms=10000
    for _ in 1 2 3 4 5 6 7 8; do
      herdr pane wait-output "$pane" --source visible --match "Allow project MCP server" \
        --timeout "$wait_ms" >/dev/null 2>&1 || break
      herdr pane send-keys "$pane" down enter >/dev/null
      wait_ms=3000; sleep 1
    done
  fi ;;
esac

# Grid hook (rules/_all/fanout-agent-grid.md): arrange the new worker's pane
# around the caller's pane. Best-effort: a grid failure warns, never fails the
# spawn. 2-4 workers go to tab GRID_TAB (default <repo-basename>-workers).
# GRID_SKIP=1 is honoured only for callers that grid themselves (pipeline.sh,
# PIPELINE_TASK set) or tests (QUAQUA_TEST=1); otherwise it is ignored.
grid_hook() {
  if [[ "${GRID_SKIP:-}" == 1 ]]; then
    [[ -n "${PIPELINE_TASK:-}" || "${QUAQUA_TEST:-}" == 1 ]] && return 0
    echo "warning: GRID_SKIP=1 ignored outside pipeline.sh/tests (rules/_all/fanout-agent-grid.md)" >&2
  fi
  local grid
  grid="$(dirname "$0")/grid-workers.sh"
  [[ -f "$grid" ]] || return 0
  bash "$grid" --tab "${GRID_TAB:-"$(basename "$repo")-workers"}" >&2 \
    || echo "warning: grid-workers.sh failed (exit $?); worker $name spawned, arrange panes manually" >&2
}

echo "worker=$name kind=$kind pane=$pane worktree=$wt"
grid_hook
