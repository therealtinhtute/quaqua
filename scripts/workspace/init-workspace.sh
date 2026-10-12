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

echo "workspace ready in $root"
