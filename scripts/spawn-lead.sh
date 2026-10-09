#!/usr/bin/env bash
# Usage: spawn-lead.sh <slug>
#   Start the project Lead for <slug> (AGENTS.md '## Roles (SLP)'): a claude
#   session named <slug>-lead running in the quaqua checkout this script
#   belongs to (normally ~/quaqua). The Lead is not a worker — no worktree,
#   no slot in the 4-worker cap, no grid hook: its pane splits right beside
#   the caller's (CoS) pane and stays there. rules/_all/*.md are the
#   appended system prompt (the spawn-worker convention); the first prompt
#   is a short lead brief pointing at AGENTS.md, projects/<slug>.md and
#   scripts/pipeline.sh. The project integration branch quaqua/<slug>-integ
#   is created from the project repo's default branch when missing —
#   origin/HEAD if set, else main, else master (Peers branch from it --base
#   and are merged back into it at acceptance). Refuses when
#   <slug> is unknown (no row in projects/INDEX.md or no projects/<slug>.md
#   card), when its Lead is already registered with herdr, or when
#   <slug>-lead would exceed herdr's 32-char agent-name limit; on success
#   the row's owner column becomes the Lead name.
#   Env: LEAD_MODEL (default claude-opus-5-5) · LEAD_EFFORT (default high).
set -euo pipefail

usage() { sed -n '2,15p' "$0" >&2; exit 1; }
die()   { echo "error: $*" >&2; exit 1; }

(( $# == 1 )) || usage
slug=$1
case $slug in -*) usage ;; esac
[[ $slug =~ ^[a-z0-9-]+$ ]] || usage

root=$(cd "$(dirname "$0")/.." && pwd)
index="$root/projects/INDEX.md" card="$root/projects/$slug.md"
name="$slug-lead"

# herdr rejects agent names >32 chars (invalid_agent_name) — fail before
# any side effect; <slug>-lead <=32 means slug <=27.
(( ${#name} <= 32 )) || die "lead name '$name' is ${#name} chars — herdr allows at most 32; use a shorter slug"

# <slug> must be a registered project: a row in INDEX.md (first cell; the
# header row is excluded by the 'repo' cell) and a card to own.
[[ -f $index ]] || die "no projects/INDEX.md in $root"
awk -F'|' -v slug="$slug" '
  /^[[:space:]]*\|/ {
    s=$2; r=$3
    gsub(/^[[:space:]]+|[[:space:]]+$/, "", s); gsub(/^[[:space:]]+|[[:space:]]+$/, "", r)
    if (s == slug && r != "repo") found = 1
  }
  END { exit !found }' "$index" \
  || die "unknown project slug '$slug' — no row in $index"
[[ -f $card ]] || die "no project card $card"

# one Lead per slug: a registered herdr agent with the Lead name
if herdr agent get "$name" 2>/dev/null | jq -e '.result.agent' >/dev/null 2>&1; then
  die "a Lead for '$slug' already exists: $name"
fi

# Integration branch convention (AGENTS.md '## Roles (SLP)'): Peers branch
# from quaqua/<slug>-integ (--base) and merge back into it at acceptance.
# Create it from the project repo's default branch when missing — origin/HEAD
# if set, else main, else master — before any pane/agent side effect, so a repo
# problem fails fast. The INDEX repo cell may carry a leading ~/ which git -C
# does not expand.
repo=$(awk -F'|' -v slug="$slug" '
  /^[[:space:]]*\|/ {
    s=$2; r=$3
    gsub(/^[[:space:]]+|[[:space:]]+$/, "", s); gsub(/^[[:space:]]+|[[:space:]]+$/, "", r)
    if (s == slug && r != "repo") { print r; exit }
  }' "$index")
repo=${repo/#\~/"$HOME"}
integ="quaqua/$slug-integ"
if ! git -C "$repo" rev-parse --verify --quiet "refs/heads/$integ" >/dev/null 2>&1; then
  # the repo's default branch: origin/HEAD if set (local ref, else its
  # remote-tracking ref), else main, else master
  base=""
  origin_head=$(git -C "$repo" symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null | sed 's|^origin/||') || true
  for cand in "$origin_head" main master; do
    [[ -n $cand ]] || continue
    if git -C "$repo" rev-parse --verify --quiet "refs/heads/$cand" >/dev/null 2>&1; then
      base=$cand; break
    fi
    if [[ $cand == "$origin_head" ]] \
        && git -C "$repo" rev-parse --verify --quiet "refs/remotes/origin/$cand" >/dev/null 2>&1; then
      base="origin/$cand"; break
    fi
  done
  [[ -n $base ]] \
    || die "cannot create $integ: $repo has no default branch (no origin/HEAD, main, or master)"
  git -C "$repo" branch "$integ" "$base" >/dev/null \
    || die "cannot create $integ in $repo"
fi

# lead rules: rules/_all/*.md -> one file, injected as the system prompt
mkdir -p "$root/runs"
rf="$root/runs/$name.rules.md"
cat "$root"/rules/_all/*.md > "$rf" 2>/dev/null || true

split=$(herdr pane split --current --direction right --no-focus --cwd "$root" --env "QUAQUA_OWNER=$name")
pane=$(printf '%s\n' "$split" | jq -r '.result.pane.pane_id')

herdr agent start "$name" --kind claude --pane "$pane" --timeout 60000 -- \
  --model "${LEAD_MODEL:-claude-opus-5-5}" --effort "${LEAD_EFFORT:-high}" \
  --append-system-prompt-file "$rf" >/dev/null

# projects/INDEX.md: the <slug> row's owner cell (3rd column) -> the Lead name
tmp=$(mktemp)
awk -F'|' -v slug="$slug" -v owner="$name" 'BEGIN { OFS="|" }
  /^[[:space:]]*\|/ {
    s=$2; r=$3
    gsub(/^[[:space:]]+|[[:space:]]+$/, "", s); gsub(/^[[:space:]]+|[[:space:]]+$/, "", r)
    if (s == slug && r != "repo") $4 = " " owner " "
  }
  { print }' "$index" > "$tmp" && cat "$tmp" > "$index"
rm -f "$tmp"

herdr agent prompt "$name" "You are the Lead for project '$slug' (QUAQUA_OWNER=$name) — role and authority: AGENTS.md '## Roles (SLP)' (this repo's root doc; read it, plus models.md for the worker roster).
- Own the project card projects/$slug.md: read it now, then keep its '## Now' at the current truth after every outcome.
- Pipeline-size work runs through scripts/pipeline.sh (usage in its header). Peers: scripts/spawn-worker.sh + scripts/grid-workers.sh with GRID_TAB=$slug-workers, at most 4 at once. Approve each builder's plan before code: a pipeline pauses with exit 5 'plan-ready', '--resume' continues it.
- You are not a worker: no worktree, not counted toward the 4-worker cap, and you never edit the project repo yourself — Peers build and commit on their branches; acceptance = you merge Peer branches into the project integration branch $integ (created from the repo's default branch if it was missing — spawn Peers --base $integ so diffs stay against it). quaqua internals (scripts/, rules/, memory/) are CoS-only.
- Resolve any '## Challenge' in runs/<worker>.md before acceptance; every redirect (from anh iu, the Auditor, or any report) reaches you through the CoS — record CoS steering in runs/pipeline-<task>.md; escalate to CoS only on goal/cost changes.
Reply 'ready' once the card is read." >/dev/null

echo "lead=$name slug=$slug pane=$pane"
