#!/usr/bin/env bash
# Usage: init-workspace.sh
#   Boots the per-workspace personal state a fresh clone needs (AGENTS.md
#   '## Memory'): the gitignored dirs memory/ projects/ raw/ runs/ reports/,
#   plus USER.md, projects/INDEX.md and memory/rules.md seeded from templates/.
#
#   Idempotent: an existing target is NEVER overwritten — every file is copied
#   only when missing, so re-running after a pull (or on a populated
#   workspace) is a no-op. Safe on a populated workspace; run it after every
#   clone.
#
#   Also wires the repo's git hooks: `core.hooksPath` -> <this checkout>/scripts/hooks
#   (the pre-commit portable-paths gate). Absolute, so the path stays valid when
#   git runs the hook from another directory, and it lands in the shared config
#   so every worktree of this repo is covered. A guardrail, not a boundary: a
#   worker can still bypass it (--no-verify, its own config). Configuration this
#   script does not own is never overwritten: an existing different value —
#   local or global — is left in place with a warning.
#
#   Templates live in templates/ under names that cannot collide with the
#   gitignored dirs: .gitignore's `memory/` and `projects/` patterns match at
#   ANY depth, so a literal templates/memory/rules.md would be untrackable.
#   tests/test_init_workspace.sh enforces that every template stays trackable.
#
#   The repo is resolved from this script's location (scripts/ is at the repo
#   root), never from cwd. Prints one line per action; nothing to do is a
#   silent success.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
[[ -d $root/templates ]] || { echo "init-workspace: no templates/ in $root" >&2; exit 1; }

# gitignored personal-state dirs (AGENTS.md '## Memory')
for d in memory projects raw runs reports; do
  if [[ -d $root/$d ]]; then
    continue
  fi
  mkdir -p "$root/$d"
  echo "created $d/"
done

# seed <repo-relative-source> <repo-relative-target>: copy only when the
# target is missing, never clobber an existing local file.
seed() {
  local src=$root/$1 dst=$root/$2
  [[ -f $src ]] || { echo "init-workspace: missing template: $1" >&2; exit 1; }
  if [[ -e $dst ]]; then
    return 0
  fi
  cp "$src" "$dst"
  echo "created $2"
}

seed templates/memory-rules.md        memory/rules.md
seed templates/projects-INDEX.md      projects/INDEX.md
seed templates/projects-_template.md  projects/_template.md
seed USER.example.md                  USER.md

# hooks: only on a real git repo, and only when nothing else owns core.hooksPath
if git -C "$root" rev-parse --git-dir >/dev/null 2>&1; then
  want=$root/scripts/hooks
  have=$(git -C "$root" config --get core.hooksPath 2>/dev/null || true)
  if [[ $have == "$want" ]]; then
    : # already ours — silent, like the rest of this script
  elif [[ -n $have ]]; then
    echo "init-workspace: core.hooksPath already set to '$have' — left unchanged" \
         "(unset it to use $want)" >&2
  else
    git -C "$root" config core.hooksPath "$want"
    echo "hooks: core.hooksPath=$want"
  fi
fi

# herdr-grid: tag the CoS orchestrator pane if running inside herdr (R5)
cos_pane=${HERDR_PANE_ID:-}
if [[ -z $cos_pane ]] && command -v herdr >/dev/null 2>&1; then
  cos_pane=$(herdr pane current 2>/dev/null | jq -r '.result.pane.pane_id // empty' 2>/dev/null || true)
fi
if [[ -n $cos_pane ]] && command -v herdr >/dev/null 2>&1; then
  herdr pane report-metadata "$cos_pane" --source herdr-grid \
    --token grid_role=orchestrator \
    --token grid_owner=cos >/dev/null 2>&1 || true
fi

echo "workspace ready in $root"
