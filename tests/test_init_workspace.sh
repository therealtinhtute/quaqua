#!/usr/bin/env bash
# Sandbox test for init-workspace.sh: a fresh clone must boot a working CoS
# workspace (AGENTS.md '## Memory' dirs + the files it always reads), and
# re-running must never clobber local content.
#
# The sandbox is a COPY of this checkout's skeleton (templates/,
# USER.example.md, .gitignore, scripts/workspace/init-workspace.sh) in a temp dir — the
# real workspace is never touched, so running this test on a populated
# ~/quaqua is safe.
# Usage: tests/test_init_workspace.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
root=$(cd "$here/.." && pwd -P)
T=$(mktemp -d "${TMPDIR:-/tmp}/test-init-workspace.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── sandbox: a true fresh clone — tracked files only, personal state absent ──
# `git ls-files` is the definition of what a clone ships; anything gitignored
# (memory/, projects/, raw/, runs/, reports/, USER.md) is deliberately absent.
# init-workspace.sh + templates/ are copied explicitly so this test also runs
# before they are committed.
R="$T/quaqua"
mkdir -p "$R"
git -C "$root" ls-files -z | while IFS= read -r -d '' f; do
  mkdir -p "$R/$(dirname "$f")"
  cp "$root/$f" "$R/$f"
done
mkdir -p "$R/scripts/workspace"
cp "$src/workspace/init-workspace.sh" "$R/scripts/workspace/init-workspace.sh"
cp "$root/USER.example.md" "$R/USER.example.md"
cp -R "$root/templates" "$R/templates"
git -C "$R" init -q  # step 0's check-ignore guard needs a repo to consult
run() { (cd / && bash "$R/scripts/workspace/init-workspace.sh"); }  # foreign cwd

# the sandbox must really model a fresh clone: personal state absent up front
for f in memory projects raw runs reports USER.md; do
  [[ -e $R/$f ]] && fail "sandbox is not a fresh clone: $f already exists"
done
echo "ok: sandbox models a fresh clone (tracked files only; no personal state)"

# ── 0. templates stay trackable: .gitignore's `memory/` and `projects/` match
#       at ANY depth, so a template under templates/memory/ would be ignored
#       and could never ship in the clone this script depends on. ───────────
git -C "$R" rev-parse --git-dir >/dev/null 2>&1 || fail "sandbox is not a git repo — the check-ignore guard would be vacuous"
for t in "$R"/templates/*; do
  if out=$(git -C "$R" check-ignore -v "$t" 2>&1); then
    fail "template is gitignored and could never ship: ${t#"$R"/} ($out)"
  fi
done
# the guard must actually fire — a nested template is the trap this guards
touch "$R/templates/memory-rules.md.probe"
mkdir -p "$R/templates/memory" && touch "$R/templates/memory/rules.md"
if git -C "$R" check-ignore -q "$R/templates/memory/rules.md"; then
  : # guard works: the nested form IS ignored
else
  fail "guard is vacuous: templates/memory/ is not ignored — the check proves nothing"
fi
rm -rf "$R/templates/memory" "$R/templates/memory-rules.md.probe"
echo "ok: every templates/* file is trackable (depth-safe names; guard verified)"

# ── 1. a fresh clone boots: dirs + the files AGENTS.md always reads ─────────
run >/dev/null || fail "init-workspace.sh failed on a fresh clone"
for d in memory projects raw runs reports; do
  [[ -d $R/$d ]] || fail "missing dir: $d/"
done
for f in memory/rules.md projects/INDEX.md projects/_template.md USER.md; do
  [[ -f $R/$f ]] || fail "missing file: $f"
done
echo "ok: fresh clone -> memory/ projects/ raw/ runs/ reports/ + rules.md, INDEX.md, _template.md, USER.md"

# ── 2. seeded content is usable, not empty ─────────────────────────────────
# AGENTS.md always reads memory/rules.md + projects/INDEX.md, and spawn-lead.sh
# parses the INDEX table (awk, header row excluded by the 'repo' cell) and
# requires the _example_ row to point at a real card.
grep -q '^# ' "$R/memory/rules.md" || fail "memory/rules.md has no heading"
grep -q '| *slug *| *repo *| *owner *|' "$R/projects/INDEX.md" \
  || fail "projects/INDEX.md has no table header"
awk -F'|' -v slug="_example_" '
  /^[[:space:]]*\|/ {
    s=$2; r=$3
    gsub(/^[[:space:]]+|[[:space:]]+$/, "", s); gsub(/^[[:space:]]+|[[:space:]]+$/, "", r)
    if (s == slug && r != "repo") found = 1
  }
  END { exit !found }' "$R/projects/INDEX.md" \
  || fail "projects/INDEX.md _example_ row is not parsable by spawn-lead.sh"
# AGENTS.md requires a ## Now section (rewritten after every outcome)
grep -q '^## Now$' "$R/projects/_template.md" || fail "projects/_template.md has no '## Now'"
grep -q '^## Vision$' "$R/projects/_template.md" || fail "projects/_template.md has no '## Vision'"
grep -q '^# ' "$R/USER.md" || fail "USER.md has no heading"
# AGENTS.md caps memory/rules.md at 2200 chars and USER.md at 1375
for f in memory/rules.md USER.md; do
  n=$(wc -c < "$R/$f")
  cap=2200; [[ $f == USER.md ]] && cap=1375
  (( n <= cap )) || fail "$f is $n chars — over the $cap cap from AGENTS.md"
done
echo "ok: seeded INDEX parses (spawn-lead.sh awk), _template.md carries ## Now, caps respected"

# ── 3. USER.md comes from the single existing source (no duplicate copy) ───
cmp -s "$R/USER.md" "$R/USER.example.md" || fail "USER.md is not USER.example.md content"
echo "ok: USER.md seeded from USER.example.md (no duplicate template)"

# ── 4. idempotent + never clobbers local content ───────────────────────────
echo '# my custom rule' >> "$R/memory/rules.md"
echo '| mine | ~/Lab/mine | cos | idle | keep me |' >> "$R/projects/INDEX.md"
echo '# edited by owner' > "$R/USER.md"
echo 'mine' > "$R/projects/_template.md"
echo 'keep' > "$R/raw/evidence.md"
echo 'keep' > "$R/runs/some-worker.md"
run >/dev/null || fail "re-run failed"
grep -q '^# my custom rule$' "$R/memory/rules.md" || fail "re-run clobbered memory/rules.md"
grep -q 'keep me' "$R/projects/INDEX.md" || fail "re-run clobbered projects/INDEX.md"
[[ "$(cat "$R/USER.md")" == "# edited by owner" ]] || fail "re-run clobbered USER.md"
[[ "$(cat "$R/projects/_template.md")" == "mine" ]] || fail "re-run clobbered projects/_template.md"
[[ "$(cat "$R/raw/evidence.md")" == "keep" ]] || fail "re-run clobbered raw/"
[[ "$(cat "$R/runs/some-worker.md")" == "keep" ]] || fail "re-run clobbered runs/"
echo "ok: re-run is a no-op on existing files (local content survives)"

# ── 5. a partial workspace is completed, missing pieces only ───────────────
rm -rf "$R/reports" "$R/USER.md"
run >/dev/null || fail "re-run failed on a partial workspace"
[[ -d $R/reports ]] || fail "reports/ not restored"
[[ -f $R/USER.md ]] || fail "USER.md not restored"
grep -q '^# my custom rule$' "$R/memory/rules.md" || fail "partial re-run clobbered memory/rules.md"
echo "ok: partial workspace -> only missing pieces are recreated"

# ── 6. a missing template fails loudly instead of seeding nothing ──────────
rm -f "$R/templates/memory-rules.md"
if out=$(run 2>&1); then fail "missing template: expected failure"; fi
[[ "$out" == *"missing template"* ]] || fail "unclear failure: $out"
echo "ok: missing template -> loud failure naming the template"

# ── 7. coverage: every file AGENTS.md says to always read is seeded ────────
# Derived from AGENTS.md, not hardcoded — a new mandatory read without a
# template fails here instead of surfacing on the next fresh clone.
#   '## Identity' pulls files in with @-includes; '## Memory' names the two
#   always-reads in backticks.
includes=$(grep -oE '^@[A-Za-z0-9._-]+' "$root/AGENTS.md" | sed 's/^@//') || true
[[ -n $includes ]] || fail "AGENTS.md has no @-includes — coverage check is vacuous"
for f in $includes; do
  [[ -f $R/$f ]] || fail "AGENTS.md includes @$f but init-workspace.sh does not seed it"
done
# shellcheck disable=SC2016  # the backticks are literal markdown, not expansion
always=$(grep -oE '^Always read.*' "$root/AGENTS.md" | grep -oE '`[A-Za-z0-9._/-]+`' | tr -d '`')
[[ -n $always ]] || fail "AGENTS.md has no 'Always read' line — coverage check is vacuous"
for f in $always; do
  [[ -f $R/$f ]] || fail "AGENTS.md always reads '$f' but init-workspace.sh does not seed it"
done
# shellcheck disable=SC2086  # $includes/$always are newline-separated lists, split on purpose
echo "ok: AGENTS.md always-reads seeded ($(printf '%s\n' $includes $always | sort -u | tr '\n' ' '))"

echo "PASS"
