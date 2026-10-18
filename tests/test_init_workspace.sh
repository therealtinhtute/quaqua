#!/usr/bin/env bash
# Sandbox test for init-workspace.sh: a fresh clone must boot a working CoS
# workspace (AGENTS.md '## Memory' dirs + the files it always reads), and
# re-running must never clobber local content.
#
# The sandbox is a COPY of this checkout's skeleton (templates/,
# USER.example.md, .gitignore, quaqua init) in a temp dir — the
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

# The entrypoint under test is a shim over the Rust binary after the R6
# cutover, so the fresh-clone boot must be able to exec `quaqua` (tests/run.sh
# exports the same PATH/QUAQUA_BIN). Build it if this is a standalone run.
bin="$root/target/debug/quaqua"
[[ -x $bin ]] || (cd "$root" && cargo build --locked --quiet) >/dev/null 2>&1 || true
export PATH="$root/target/debug:$PATH"
export QUAQUA_BIN="${QUAQUA_BIN:-$bin}"

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
# pre-commit hook is embedded in binary and generated into .git/quaqua-hooks/
cp "$root/USER.example.md" "$R/USER.example.md"
cp -R "$root/templates" "$R/templates"
git -C "$R" init -q  # step 0's check-ignore guard needs a repo to consult

# herdr stub on PATH so init-workspace.sh never touches real herdr even if run in a live pane
mkdir -p "$T/bin"
cat > "$T/bin/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "${CALLS:-/dev/null}"
# pane metadata the caller asks for: PANE_CURRENT_ID (pane current),
# PANE_AGENT/PANE_CWD (pane get), PA_PANE_ID/PA_AGENT/PA_AGENT_NAME (agent list;
# no PA_AGENT_NAME = the pane holds an unregistered agent)
case "${1:-} ${2:-}" in
  "pane current")
    [ -n "${PANE_CURRENT_ID:-}" ] && echo "{\"result\":{\"pane\":{\"pane_id\":\"$PANE_CURRENT_ID\"}}}"
    ;;
  "pane get")
    printf '{"result":{"pane":{"pane_id":"%s","agent":"%s","cwd":"%s"}}}\n' \
      "${3:-}" "${PANE_AGENT:-}" "${PANE_CWD:-}"
    ;;
  "agent list")
    if [ -n "${PA_PANE_ID:-}" ]; then
      if [ -n "${PA_AGENT_NAME:-}" ]; then
        printf '{"result":{"agents":[{"pane_id":"%s","agent":"%s","name":"%s"}]}}\n' \
          "$PA_PANE_ID" "${PA_AGENT:-claude}" "$PA_AGENT_NAME"
      else
        printf '{"result":{"agents":[{"pane_id":"%s","agent":"%s"}]}}\n' \
          "$PA_PANE_ID" "${PA_AGENT:-claude}"
      fi
    else
      echo '{"result":{"agents":[]}}'
    fi
    ;;
esac
exit 0
STUB
chmod +x "$T/bin/herdr"
export PATH="$T/bin:$PATH"
unset HERDR_PANE_ID

run() { ( unset HERDR_PANE_ID; cd "$R" && "${QUAQUA_BIN:-quaqua}" init); }

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
cp "$R/templates/memory-rules.md" "$T/memory-rules.bak"   # restored below
rm -f "$R/templates/memory-rules.md"
if out=$(run 2>&1); then fail "missing template: expected failure"; fi
[[ "$out" == *"missing template"* ]] || fail "unclear failure: $out"
cp "$T/memory-rules.bak" "$R/templates/memory-rules.md"
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

# ── 8. git hooks wired: core.hooksPath -> scripts/hooks, never overwritten ──
# The pre-commit portable-paths gate only runs if init-workspace.sh points git
# at scripts/hooks/. Three states must hold: fresh repo -> wired; already ours
# -> silent no-op; a different value (local or global) -> warned and left.
exp="$R/.git/quaqua-hooks"
[[ -d $exp ]] || fail "sandbox has no .git/quaqua-hooks dir to wire"
[[ "$(git -C "$R" config --get core.hooksPath)" == "$exp" ]] \
  || fail "init-workspace.sh did not set core.hooksPath (want $exp)"
[[ -x $exp/pre-commit ]] || fail ".git/quaqua-hooks/pre-commit is not executable"
echo "ok: fresh clone -> core.hooksPath wired to .git/quaqua-hooks"

# idempotent when it is already ours (and quiet about it)
out=$(run 2>&1)
[[ "$(git -C "$R" config --get core.hooksPath)" == "$exp" ]] || fail "re-run changed core.hooksPath"
[[ $out != *hooksPath* ]] || fail "re-run should stay quiet about a value it already owns: $out"
echo "ok: re-run leaves an already-correct core.hooksPath untouched and silent"

# a DIFFERENT local value is policy init-workspace.sh does not own: warn + leave
git -C "$R" config core.hooksPath /somewhere/else
out=$(run 2>&1)
[[ "$(git -C "$R" config --get core.hooksPath)" == "/somewhere/else" ]] \
  || fail "init-workspace.sh overwrote an existing core.hooksPath"
[[ $out == *"already set"* ]] || fail "no warning about the foreign core.hooksPath: $out"
[[ $out == *"/somewhere/else"* ]] || fail "warning does not name the existing value: $out"
echo "ok: foreign local core.hooksPath -> warned, not overwritten"
git -C "$R" config --unset core.hooksPath

# a different GLOBAL value is left alone too (local scope must stay unset)
G="$T/global.gitconfig"
printf '[core]\n\thooksPath = /global/hooks\n' > "$G"
out=$(GIT_CONFIG_GLOBAL="$G" run 2>&1) || fail "run failed with a global core.hooksPath set"
[[ -z "$(GIT_CONFIG_GLOBAL="$G" git -C "$R" config --local --get core.hooksPath 2>/dev/null || true)" ]] \
  || fail "init-workspace.sh set a local core.hooksPath over a global one"
[[ $out == *"already set"* ]] || fail "no warning about the foreign global core.hooksPath: $out"
[[ $(GIT_CONFIG_GLOBAL="$G" git -C "$R" config --get core.hooksPath) == "/global/hooks" ]] \
  || fail "the global core.hooksPath value was modified"
echo "ok: foreign global core.hooksPath -> warned, not overwritten"

# legacy migration: scripts/hooks -> .git/quaqua-hooks
git -C "$R" config core.hooksPath "$R/scripts/hooks"
out=$(run 2>&1)
[[ "$(git -C "$R" config --get core.hooksPath)" == "$exp" ]] \
  || fail "legacy core.hooksPath was not migrated"
[[ $out == *"migrated"* ]] || fail "no migration log: $out"
echo "ok: legacy core.hooksPath -> migrated to .git/quaqua-hooks"

# not a git repo -> the wiring block is skipped, everything else still boots
N="$T/notarepo"
mkdir -p "$N"
cp -R "$R/templates" "$R/USER.example.md" "$N/"
mkdir -p "$N/scripts/hooks"
( unset HERDR_PANE_ID; cd "$N" && "${QUAQUA_BIN:-quaqua}" init ) >/dev/null \
  || fail "init-workspace failed outside a git repo"
[[ -f $N/memory/rules.md ]] || fail "non-repo workspace was not booted"
echo "ok: outside a git repo -> hook wiring skipped, workspace still booted"

# ── 9. herdr-quaqua: ONLY the real CoS pane is tagged orchestrator (R5) ───────
# init-workspace.sh runs from whatever pane drives the clone, and every worker
# worktree of this repo ships a copy of it, so the caller's pane is not the CoS.
# Regression this covers (2026-10-14): an opencode pane came out of here tagged
# grid_role=orchestrator/grid_owner=cos while the real CoS pane had no token, so
# grid-workers.sh anchored the grid beside the wrong pane and timed out.
tag='herdr pane report-metadata w1:p1 --source herdr-quaqua --token grid_role=orchestrator --token grid_owner=cos'
mkdir -p "$T/wt" "$T/elsewhere"   # a worker worktree and a non-checkout cwd

# tag_as <pane> <agent> <cwd> <agent-name>: run init-workspace.sh as if called
# from <pane>, which herdr reports as <agent> in <cwd>, registered as
# <agent-name> ('' = unregistered). Prints the herdr calls the script made.
tag_as() {
  : > "$T/calls"
  ( export CALLS="$T/calls" HERDR_PANE_ID="$1" PANE_CURRENT_ID="$1" \
           PANE_AGENT="$2" PANE_CWD="$3" PA_PANE_ID="$1" PA_AGENT="$2" PA_AGENT_NAME="$4"
    cd "$R" && "${QUAQUA_BIN:-quaqua}" init ) >/dev/null \
    || fail "init-workspace.sh failed for pane $1 ($2/${4:-unnamed})"
  cat "$T/calls"
}

# the CoS itself — an unregistered Claude agent in this checkout — is tagged
out=$(tag_as w1:p1 claude "$R" "")
grep -qF "$tag" <<<"$out" \
  || fail "the CoS pane (claude, unregistered, in the checkout) was not tagged: $out"
! grep -q 'grid_ready' <<<"$out" || fail "CoS pane should not have grid_ready token"
echo "ok: herdr-quaqua -> the CoS pane is tagged orchestrator/owner=cos, no grid_ready"

# every other caller stays untagged: a stray orchestrator token anchors the grid
# beside the wrong pane
out=$(tag_as w1:p2 opencode /elsewhere sude-scan)
! grep -q 'report-metadata' <<<"$out" || fail "opencode pane tagged as the CoS orchestrator: $out"
out=$(tag_as w1:p3 claude "$T/wt" sude-fix)          # a claude worker in its worktree
! grep -q 'report-metadata' <<<"$out" || fail "claude worker pane tagged as the CoS orchestrator: $out"
out=$(tag_as w1:p4 claude "$R" acme-lead)            # a Lead: same cwd as the CoS
! grep -q 'report-metadata' <<<"$out" || fail "Lead pane tagged as the CoS orchestrator: $out"
out=$(tag_as w1:p5 claude "$T/elsewhere" "")         # unregistered, but not this checkout
! grep -q 'report-metadata' <<<"$out" || fail "unregistered claude pane outside the checkout tagged as the CoS: $out"
echo "ok: herdr-quaqua -> opencode / claude-worker / Lead / foreign-dir panes stay untagged"

# with HERDR_PANE_ID unset the pane still comes from herdr pane current
: > "$T/calls"
( export CALLS="$T/calls" PANE_CURRENT_ID="w1:p1" PANE_AGENT=claude PANE_CWD="$R" \
         PA_PANE_ID="w1:p1" PA_AGENT=claude
  unset HERDR_PANE_ID PA_AGENT_NAME
  cd "$R" && "${QUAQUA_BIN:-quaqua}" init ) >/dev/null \
  || fail "init-workspace.sh failed with herdr pane current"
grep -qF "$tag" "$T/calls" \
  || fail "the CoS pane was not tagged via herdr pane current: $(cat "$T/calls")"
echo "ok: herdr-quaqua -> herdr pane current fallback still tags the CoS pane"

# a herdr that cannot say who the pane is -> no tag, and no failure
: > "$T/calls"
( export CALLS="$T/calls" HERDR_PANE_ID="w1:p1" PANE_CURRENT_ID="w1:p1"
  unset PANE_AGENT PANE_CWD PA_PANE_ID PA_AGENT_NAME
  cd "$R" && "${QUAQUA_BIN:-quaqua}" init ) >/dev/null \
  || fail "init-workspace.sh failed when herdr reports no pane metadata"
! grep -q 'report-metadata' "$T/calls" || fail "tagged an unidentifiable pane: $(cat "$T/calls")"
echo "ok: herdr-quaqua -> an unidentifiable pane fails closed (no tag, no error)"

echo "PASS"
