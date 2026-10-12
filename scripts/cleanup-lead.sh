#!/usr/bin/env bash
# Usage: cleanup-lead.sh <slug>
#   Tear down the project Lead <slug>-lead once the project is done. Refuses
#   while any of its Peers is still registered with herdr (runs/<peer>.owner
#   naming the Lead) — clean those up first with cleanup-worker.sh. Closes the
#   Lead's team workspace only when it is the one spawn-lead.sh made (label
#   <slug>) and not the caller's; a Lead living anywhere else (older spawns)
#   gets just its pane closed. Branch quaqua/<slug>-integ is kept; the
#   projects/INDEX.md owner cell returns to cos (serialized with spawn-lead.sh
#   via projects/INDEX.md.lock.d, published by atomic rename). A Peer or Lead
#   whose `herdr agent get` fails for any reason other than a definite
#   agent_not_found is of unknown liveness: cleanup aborts rather than close
#   a workspace it cannot prove is dead.
set -euo pipefail

usage() { sed -n '2,13p' "$0" >&2; exit 1; }
die()   { echo "error: $*" >&2; exit 1; }

(( $# == 1 )) || usage
slug=$1
[[ $slug =~ ^[a-z0-9][a-z0-9-]*$ ]] || usage

root=$(cd "$(dirname "$0")/.." && pwd)
runs_dir="${QUAQUA_RUNS_DIR:-$root/runs}"
name="$slug-lead"

# The Lead must be positively gone or positively present: a failed
# `herdr agent get` that is not a definite agent_not_found leaves its state
# unknown — abort rather than clean up blindly (F1). herdr reports errors
# (agent_not_found included) as JSON on stderr, not stdout.
lead=$(herdr agent get "$name" 2>/dev/null) || true
if ! jq -e '.result.agent' >/dev/null 2>&1 <<<"$lead"; then
  [[ -z $lead ]] && lead=$(herdr agent get "$name" 2>&1 >/dev/null) || true
  jq -e '.error.code == "agent_not_found"' <<<"$lead" >/dev/null 2>&1 \
    || [[ $lead == *agent_not_found* ]] \
    || die "cannot determine the state of Lead '$name' (herdr agent get failed) — refusing to clean up"
  die "no live Lead '$name'"
fi
pane=$(jq -r '.result.agent.pane_id' <<<"$lead")
ws=$(jq -r '.result.agent.workspace_id // empty' <<<"$lead")
ws=${ws:-${pane%%:*}}

# Peers still registered -> refuse (their worktrees and branches need
# cleanup-worker.sh). A Peer counts as gone only on a positively identified
# agent_not_found; any other `herdr agent get` failure (transport, permission,
# service, unparseable body) leaves its liveness unknown — closing the team
# workspace could kill live work, so abort instead (F1).
live=()
for f in "$runs_dir"/*.owner; do
  [[ -e $f ]] || continue
  [[ $(<"$f") == "$name" ]] || continue
  peer=$(basename "$f" .owner)
  get=$(herdr agent get "$peer" 2>/dev/null) || true
  if jq -e '.result.agent' >/dev/null 2>&1 <<<"$get"; then
    live+=("$peer"); continue
  fi
  [[ -z $get ]] && get=$(herdr agent get "$peer" 2>&1 >/dev/null) || true
  jq -e '.error.code == "agent_not_found"' <<<"$get" >/dev/null 2>&1 \
    || [[ $get == *agent_not_found* ]] \
    || die "cannot determine whether Peer '$peer' is still live (herdr agent get failed) — refusing to clean up '$name'; retry, or run cleanup-worker.sh if it is gone"
done
(( ${#live[@]} == 0 )) || die "$name still owns live Peers: ${live[*]} — run cleanup-worker.sh first"

caller_pane=${HERDR_PANE_ID:-$(herdr pane current 2>/dev/null | jq -r '.result.pane.pane_id // empty' || true)}
caller_ws=${caller_pane%%:*}
label=$(herdr workspace get "$ws" 2>/dev/null | jq -r '.result.workspace.label // empty' || true)

if [[ $label == "$slug" && $ws != "$caller_ws" ]]; then
  herdr workspace close "$ws" >/dev/null
  closed="workspace $ws"
else
  herdr pane close "$pane" >/dev/null
  closed="pane $pane"
fi
rm -f "$runs_dir/$name.rules.md"

# projects/INDEX.md: the <slug> row's owner cell (3rd column) -> cos.
# Serialized with spawn-lead.sh through a mkdir lock (portable — flock is
# absent on macOS); the new INDEX is built beside it and published by atomic
# rename, so no reader sees a torn file and concurrent writers cannot lose
# each other's update (F4).
index="$root/projects/INDEX.md"
if [[ -f $index ]]; then
  lock="$index.lock.d" i=0 tmp=
  until mkdir "$lock" 2>/dev/null; do
    (( ++i <= 200 )) || die "$index is locked (stale $lock? remove it manually)"
    sleep 0.05
  done
  trap '[[ -z $tmp ]] || rm -f "$tmp"; rmdir "$lock"' EXIT
  tmp=$(mktemp "$index.XXXXXX")
  chmod "$(stat -c %a "$index" 2>/dev/null || stat -f %Lp "$index" 2>/dev/null || echo 644)" "$tmp" 2>/dev/null || true
  awk -F'|' -v slug="$slug" 'BEGIN { OFS="|" }
    /^[[:space:]]*\|/ {
      s=$2; r=$3
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", s); gsub(/^[[:space:]]+|[[:space:]]+$/, "", r)
      if (s == slug && r != "repo") $4 = " cos "
    }
    { print }' "$index" > "$tmp" && mv -f "$tmp" "$index"
  rmdir "$lock"
  trap - EXIT
fi

echo "cleaned $name ($closed closed; branch quaqua/$slug-integ kept)"
