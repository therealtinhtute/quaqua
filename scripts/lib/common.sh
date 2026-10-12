#!/usr/bin/env bash
# scripts/lib/common.sh — helpers shared by the scripts under scripts/*/.
# Sourced, never executed:  . "$(dirname "$0")/../lib/common.sh"
# No side effects beyond the definitions below (safe under `set -euo pipefail`).
#
# Phase 3 of docs/plans/active/scripts-restructure.md: the copies collapsed here
# were verified identical (or parameterised only by variable name / runs dir)
# before extraction — no behaviour change. Helpers with a single copy stay in
# their script (prompt-worker.sh's line-number-free usage(), grid-workers.sh's
# heredoc usage()).

# die <message...> — "<prefix>: <message>" on stderr, exit 1.
# Prefix: $PROG when the script sets it (grid-workers.sh keeps its program
# name), else "error" — the lead scripts' long-standing format.
die() { printf '%s: %s\n' "${PROG:-error}" "$*" >&2; exit 1; }

# usage — print the calling script's leading comment block (lines 2..$USAGE_LAST)
# on stderr and exit 1. Each script sets USAGE_LAST beside its source line, so
# the range stays visible next to the header it prints.
usage() { sed -n "2,${USAGE_LAST:?usage() requires USAGE_LAST}p" "$0" >&2; exit 1; }

# count_owner_slots <runs-dir> <owner> [<exclude-worker>] — worker slots in use
# by <owner> (cos or <slug>-lead): every live task worker in herdr counts 1, an
# active pipeline reserves 2 (builder + reviewer). <slug>-lead agents are never
# workers and never count. Prints the total. <runs-dir> is the caller's runs
# directory (its own default applies: $HOME/quaqua/runs, $root/runs, ...).
count_owner_slots() {
  local runs_dir=$1 o=${2:-cos} exclude=${3:-}
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
