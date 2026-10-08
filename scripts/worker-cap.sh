#!/usr/bin/env bash
# Usage: worker-cap.sh <owner> [<exclude-worker>]
#   Print the number of active worker slots used by <owner> (cos or <slug>-lead).
#   Each live task worker in herdr counts as 1.
#   Each active pipeline (finished=0 and alive) reserves 2 slots (builder + reviewer).
#   Project Leads (<slug>-lead) do not count toward the cap.
#   Max allowed slots per owner is 4.
set -euo pipefail

owner=${1:-cos}
exclude_worker=${2:-}
runs_dir="${QUAQUA_RUNS_DIR:-$HOME/quaqua/runs}"

# 1. Query live agents from herdr
agents=$(herdr agent list 2>/dev/null || true)
live_workers=()
if jq -e . >/dev/null 2>&1 <<<"$agents"; then
  while IFS= read -r a_name; do
    [[ -n $a_name ]] || continue
    [[ $a_name == "$exclude_worker" ]] && continue
    live_workers+=("$a_name")
  done < <(jq -r '.result.agents[]?
    | select((.agent_status // "") != "dead")
    | select((.name // "") | test("^(tieude|tieunhi|sude|ni|suhuynh|truonglao|bacdau|quansu)-") and (test("-lead$") | not))
    | .name' <<<"$agents" 2>/dev/null || true)
fi

# 2. Count live workers belonging to $owner
owner_live=0
owner_workers=()
for w in ${live_workers[@]+"${live_workers[@]}"}; do
  w_owner=$(cat "$runs_dir/$w.owner" 2>/dev/null || true)
  w_owner=${w_owner:-cos}
  if [[ "$w_owner" == "$owner" ]]; then
    (( owner_live++ )) || true
    owner_workers+=("$w")
  fi
done

# 3. Check active pipelines belonging to $owner (each reserves 2 slots)
reserved_slots=0
for s in "$runs_dir"/pipeline-*.state; do
  [[ -f $s ]] || continue
  grep -q '^finished=0$' "$s" || continue
  p_lead=$(sed -n 's/^lead=\(.*\)/\1/p' "$s" | head -1)
  p_owner=${p_lead:-cos}
  [[ "$p_owner" == "$owner" ]] || continue

  p_pid=$(sed -n 's/^pipeline_pid=\([0-9][0-9]*\).*/\1/p' "$s" | head -1)
  p_b=$(sed -n 's/^builder=\(.*\)/\1/p' "$s" | head -1)
  p_t=$(sed -n 's/^task=\(.*\)/\1/p' "$s" | head -1)
  p_bname="$p_b-$p_t"

  p_alive=0
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
    # Count how many live workers this pipeline currently has
    p_cur_live=0
    for w in ${owner_workers[@]+"${owner_workers[@]}"}; do
      w_pipe=$(cat "$runs_dir/$w.pipeline" 2>/dev/null || true)
      if [[ "$w_pipe" == "$p_t" || "$w" == "$p_bname" || "$w" == *-pr[0-9]* || "$w" == *-${p_t}r[0-9]* ]]; then
        (( p_cur_live++ )) || true
      fi
    done
    if (( p_cur_live < 2 )); then
      (( reserved_slots += (2 - p_cur_live) )) || true
    fi
  fi
done

total_slots=$(( owner_live + reserved_slots ))
printf '%d\n' "$total_slots"
