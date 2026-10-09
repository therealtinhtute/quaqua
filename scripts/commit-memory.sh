#!/usr/bin/env bash
# Usage: commit-memory.sh "<message>"
# Commits the CoS's own notes — changes under memory/, projects/, raw/ — on the
# current branch of ~/quaqua, no side branch needed. The repo is resolved from
# this script's location (scripts/ is at the repo root), never from cwd.
# Stages ONLY those dirs; refuses (exit 1) when anything else is already
# staged or when there is nothing to commit. Appends the attribution line to
# the message. Never pushes, never amends.
set -euo pipefail

die() { echo "commit-memory: $*" >&2; exit 1; }

(( $# == 1 )) && [[ -n $1 ]] || die "usage: scripts/commit-memory.sh \"<message>\""

repo="$(cd "$(dirname "$0")/.." && pwd -P)"
git -C "$repo" rev-parse --git-dir >/dev/null 2>&1 || die "not a git repository: $repo"

is_note() {  # is_note <repo-relative path> → under memory/, projects/ or raw/
  case $1 in memory/* | projects/* | raw/*) return 0 ;; *) return 1 ;; esac
}

staged=()
collect_staged() {  # fills $staged with repo-relative staged paths
  staged=()
  local p
  # -z: NUL-delimited, safe for quoted names. --no-renames: a rename is split
  # into delete+add, so a disguised "scripts/x -> memory/x" rename still
  # exposes scripts/x to the allowlist (gitguard finding 5).
  while IFS= read -r -d '' p; do staged+=("$p"); done \
    < <(git -C "$repo" diff --cached --name-only --no-renames -z)
}

# 1. Refuse if anything outside the note dirs is already staged. Checked
#    BEFORE we stage anything, so a refusal leaves the index untouched.
collect_staged
for p in ${staged[@]+"${staged[@]}"}; do
  is_note "$p" || die "refusing: staged path outside memory/ projects/ raw/: $p"
done

# 2. Stage note changes, dir by dir. Missing dirs are skipped (raw/ is not
#    on every checkout). `add -u` covers tracked edits + deletions and never
#    errors on an ignored pathspec, but requires ≥1 tracked file; untracked
#    files come from `ls-files -o --exclude-standard`, which honors .gitignore
#    — so a gitignored dir (raw/) is never force-added, but a tracked file
#    inside one is still picked up.
for d in memory projects raw; do
  [[ -d $repo/$d ]] || continue
  tracked=(); untracked=()
  while IFS= read -r -d '' p; do tracked+=("$p"); done \
    < <(git -C "$repo" ls-files -z -- "$d")
  while IFS= read -r -d '' p; do untracked+=("$p"); done \
    < <(git -C "$repo" ls-files --others --exclude-standard -z -- "$d")
  (( ${#tracked[@]} > 0 ))   && git -C "$repo" add -u -- "$d"
  (( ${#untracked[@]} > 0 )) && git -C "$repo" add -- "${untracked[@]}"
done

# 3. Recheck: the staged set must be non-empty and still notes-only.
collect_staged
(( ${#staged[@]} > 0 )) || die "nothing to commit (no changes under memory/ projects/ raw/)"
for p in "${staged[@]}"; do
  is_note "$p" || die "refusing: staged path outside memory/ projects/ raw/: $p"
done

git -C "$repo" commit -q -m "$1" -m "👾 · from therealTINHTUTE with love"
echo "committed $(git -C "$repo" rev-parse --short HEAD): $1"
