#!/usr/bin/env bash
# Usage: cleanup-lead.sh <slug>
#   Tear down the project Lead <slug>-lead once the project is done. Refuses
#   while any of its Peers is still registered with herdr (runs/<peer>.owner
#   naming the Lead) — clean those up first with cleanup-worker.sh. Closes the
#   Lead's team workspace only when it is the one spawn-lead.sh made (label
#   <slug>) and not the caller's; a Lead living anywhere else (older spawns)
#   gets just its pane closed. Branch quaqua/<slug>-integ is kept; the
#   projects/INDEX.md owner cell returns to cos.
set -euo pipefail

usage() { sed -n '2,9p' "$0" >&2; exit 1; }
die()   { echo "error: $*" >&2; exit 1; }

(( $# == 1 )) || usage
slug=$1
[[ $slug =~ ^[a-z0-9][a-z0-9-]*$ ]] || usage

root=$(cd "$(dirname "$0")/.." && pwd)
runs_dir="${QUAQUA_RUNS_DIR:-$root/runs}"
name="$slug-lead"

lead=$(herdr agent get "$name" 2>/dev/null) && jq -e '.result.agent' >/dev/null 2>&1 <<<"$lead" \
  || die "no live Lead '$name'"
pane=$(jq -r '.result.agent.pane_id' <<<"$lead")
ws=$(jq -r '.result.agent.workspace_id // empty' <<<"$lead")
ws=${ws:-${pane%%:*}}

# Peers still registered -> refuse (their worktrees and branches need cleanup-worker.sh)
live=()
for f in "$runs_dir"/*.owner; do
  [[ -e $f ]] || continue
  [[ $(<"$f") == "$name" ]] || continue
  peer=$(basename "$f" .owner)
  herdr agent get "$peer" >/dev/null 2>&1 && live+=("$peer")
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

# projects/INDEX.md: the <slug> row's owner cell (3rd column) -> cos
index="$root/projects/INDEX.md"
if [[ -f $index ]]; then
  tmp=$(mktemp)
  awk -F'|' -v slug="$slug" 'BEGIN { OFS="|" }
    /^[[:space:]]*\|/ {
      s=$2; r=$3
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", s); gsub(/^[[:space:]]+|[[:space:]]+$/, "", r)
      if (s == slug && r != "repo") $4 = " cos "
    }
    { print }' "$index" > "$tmp" && cat "$tmp" > "$index"
  rm -f "$tmp"
fi

echo "cleaned $name ($closed closed; branch quaqua/$slug-integ kept)"
