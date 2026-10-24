#!/usr/bin/env bash
# Sandbox test for `quaqua init`: a fresh workspace boots from the template
# embedded in the binary — the target is an empty temp dir and the cwd is
# another empty temp dir, so no checkout (and no templates/ on disk) is needed.
#
# What it proves:
#   - every embedded file lands byte-identical to templates/, exec bits kept;
#   - the seeded personal state is usable and capped (AGENTS.md Memory rules);
#   - re-running never clobbers local content, a partial workspace is completed;
#   - engine-only paths never leak into the workspace;
#   - a git target gets core.hooksPath wired to the embedded pre-commit;
#   - only the real CoS pane is tagged orchestrator (R5).
# The real workspace is never touched, so running this on a populated
# ~/quaqua is safe. Usage: tests/test_init_workspace.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root=$(cd "$here/.." && pwd -P)
templates="$root/templates"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-init-workspace.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# The binary embeds the template, so it must be rebuilt whenever templates/
# changes — otherwise the byte-equality check compares against a stale table.
(cd "$root" && cargo build --locked --quiet) >/dev/null 2>&1 \
  || fail "cargo build --locked failed"
bin="$root/target/debug/quaqua"
[[ -x $bin ]] || fail "no quaqua binary at $bin"
export PATH="$root/target/debug:$PATH"
export QUAQUA_BIN="${QUAQUA_BIN:-$bin}"

# herdr stub on PATH so init never touches a real herdr; it answers the pane
# queries the CoS tag asks (PANE_AGENT/PANE_CWD are filled in per case).
mkdir -p "$T/bin"
cat > "$T/bin/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "${CALLS:-/dev/null}"
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
export HERDR_BIN_PATH="$T/bin/herdr"
export HERDR_SOCKET_PATH="$T/herdr.sock"
export PATH="$T/bin:$PATH"
unset HERDR_PANE_ID
# `init` without a <dir> defaults to workspace_root(); pin it so a stray call
# can never reach anh iu's real ~/quaqua.
export QUAQUA_HOME="$T/home"

ws="$T/ws"          # the target: does not exist yet — init creates it
empty="$T/empty"    # the cwd: stays empty, init reads nothing from it
[[ ! -e $ws ]] || fail "target already exists — the fresh-dir proof would be vacuous"
mkdir -p "$empty"

# ── 1. an empty temp dir boots into a workspace ────────────────────────────
out=$( ( unset HERDR_PANE_ID; cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$ws" ) ) \
  || fail "quaqua init failed on an empty target: $out"
[[ -d $ws ]] || fail "init did not create the target dir"
[[ -z "$(ls -A "$empty")" ]] || fail "init wrote into the caller's cwd: $(ls -A "$empty")"
[[ -f $ws/.quaqua/template.lock ]] || fail "no .quaqua/template.lock"
grep -q 'workspace ready in' <<<"$out" || fail "no ready line: $out"
grep -q 'cargo install --path <checkout>/crates/quaqua --locked' <<<"$out" \
  || fail "install line missing or stale: $out"
for d in memory projects raw runs reports; do
  [[ -d $ws/$d ]] || fail "missing dir: $d/"
done
echo "ok: empty target dir -> workspace booted (no checkout, empty cwd stays empty)"

# ── 2. the embedded template is complete and byte-identical to templates/ ──
# This is the guard for the plan's B2 risk: an embedded table that misses a
# file (or its exec bit) would boot a broken workspace silently.
files=0
while IFS= read -r rel; do
  src="$templates/workspace/$rel"
  [[ -f $ws/$rel ]] || fail "embedded table missed templates/workspace/$rel"
  cmp -s "$src" "$ws/$rel" || fail "not byte-identical: $rel (vs templates/workspace/$rel)"
  if [[ -x $src && ! -x $ws/$rel ]] || [[ ! -x $src && -x $ws/$rel ]]; then
    fail "exec bit differs: $rel"
  fi
  files=$((files + 1))
done < <(cd "$templates/workspace" && find . -type f | sed 's|^\./||' | sort)
[[ $files -gt 0 ]] || fail "templates/workspace holds no files — the equality check is vacuous"
[[ -x $ws/scripts/hooks/herdr-gate.sh ]] || fail "herdr-gate.sh is not executable"
[[ -x $ws/scripts/auditor/install-auditor.sh ]] || fail "install-auditor.sh is not executable"
# the seeded files keep their own destination names (templates/seed/ is flat)
seeds=0
while IFS= read -r crate; do
  case $crate in
    memory-rules.md) dst=memory/rules.md ;;
    projects-INDEX.md) dst=projects/INDEX.md ;;
    projects-_template.md) dst=projects/_template.md ;;
    USER.example.md) dst=USER.md ;;
    *) fail "templates/seed/$crate has no destination — update SEED_TARGETS in init.rs and this test" ;;
  esac
  cmp -s "$templates/seed/$crate" "$ws/$dst" \
    || fail "seed $crate is not byte-identical at $dst"
  files=$((files + 1)); seeds=$((seeds + 1))
done < <(cd "$templates/seed" && find . -type f | sed 's|^\./||' | sort)
[[ $seeds -gt 0 ]] || fail "templates/seed holds no files — the seed check is vacuous"
grep -qF '$CLAUDE_PROJECT_DIR' "$ws/.claude/settings.json" \
  && grep -qF '/scripts/hooks/herdr-gate.sh' "$ws/.claude/settings.json" \
  && ! grep -qF 'templates/workspace' "$ws/.claude/settings.json" \
  || fail "workspace .claude/settings.json does not point at its own gate"
echo "ok: $files embedded files byte-identical to templates/ ($seeds seeds remapped, exec bits kept)"

# ── 3. seeded content is usable, not empty ─────────────────────────────────
# AGENTS.md always reads memory/rules.md + projects/INDEX.md, and spawn-lead.sh
# parses the INDEX table (awk, header row excluded by the 'repo' cell) and
# requires the _example_ row to point at a real card.
grep -q '^# ' "$ws/memory/rules.md" || fail "memory/rules.md has no heading"
grep -q '| *slug *| *repo *| *owner *|' "$ws/projects/INDEX.md" \
  || fail "projects/INDEX.md has no table header"
awk -F'|' -v slug="_example_" '
  /^[[:space:]]*\|/ {
    s=$2; r=$3
    gsub(/^[[:space:]]+|[[:space:]]+$/, "", s); gsub(/^[[:space:]]+|[[:space:]]+$/, "", r)
    if (s == slug && r != "repo") found = 1
  }
  END { exit !found }' "$ws/projects/INDEX.md" \
  || fail "projects/INDEX.md _example_ row is not parsable by spawn-lead.sh"
# AGENTS.md requires a ## Now section (rewritten after every outcome)
grep -q '^## Now$' "$ws/projects/_template.md" || fail "projects/_template.md has no '## Now'"
grep -q '^## Vision$' "$ws/projects/_template.md" || fail "projects/_template.md has no '## Vision'"
grep -q '^# ' "$ws/USER.md" || fail "USER.md has no heading"
# AGENTS.md caps memory/rules.md at 2200 chars and USER.md at 1375
for f in memory/rules.md USER.md; do
  n=$(wc -c < "$ws/$f")
  cap=2200; [[ $f == USER.md ]] && cap=1375
  (( n <= cap )) || fail "$f is $n chars — over the $cap cap from AGENTS.md"
done
echo "ok: seeded INDEX parses (spawn-lead.sh awk), _template.md carries ## Now, caps respected"

# ── 4. idempotent + never clobbers local content ───────────────────────────
echo '# my custom rule' >> "$ws/memory/rules.md"
echo '| mine | ~/Lab/mine | cos | idle | keep me |' >> "$ws/projects/INDEX.md"
echo '# edited by owner' > "$ws/USER.md"
echo 'mine' > "$ws/projects/_template.md"
echo 'keep' > "$ws/raw/evidence.md"
echo 'keep' > "$ws/runs/some-worker.md"
echo '# edited' >> "$ws/AGENTS.md"
( unset HERDR_PANE_ID; cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$ws" ) >/dev/null \
  || fail "re-run failed"
grep -q '^# my custom rule$' "$ws/memory/rules.md" || fail "re-run clobbered memory/rules.md"
grep -q 'keep me' "$ws/projects/INDEX.md" || fail "re-run clobbered projects/INDEX.md"
[[ "$(cat "$ws/USER.md")" == "# edited by owner" ]] || fail "re-run clobbered USER.md"
[[ "$(cat "$ws/projects/_template.md")" == "mine" ]] || fail "re-run clobbered projects/_template.md"
[[ "$(cat "$ws/raw/evidence.md")" == "keep" ]] || fail "re-run clobbered raw/"
[[ "$(cat "$ws/runs/some-worker.md")" == "keep" ]] || fail "re-run clobbered runs/"
grep -q '^# edited$' "$ws/AGENTS.md" || fail "re-run clobbered a managed file"
echo "ok: re-run is a no-op on existing files (local content survives)"

# ── 5. a partial workspace is completed, missing pieces only ───────────────
rm -rf "$ws/reports" "$ws/USER.md" "$ws/rules/_all/handoff.md"
( unset HERDR_PANE_ID; cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$ws" ) >/dev/null \
  || fail "re-run failed on a partial workspace"
[[ -d $ws/reports ]] || fail "reports/ not restored"
[[ -f $ws/USER.md ]] || fail "USER.md not restored"
[[ -f $ws/rules/_all/handoff.md ]] || fail "managed rule not restored"
grep -q '^# my custom rule$' "$ws/memory/rules.md" || fail "partial re-run clobbered memory/rules.md"
echo "ok: partial workspace -> only missing pieces are recreated"

# ── 6. engine-only paths never leak into the workspace ────────────────────
for p in templates crates plugins tests docs .github Cargo.toml Cargo.lock \
         README.md SETUP-PROMPT.md .gitignore; do
  [[ -e $ws/$p ]] && fail "engine-only path leaked into the workspace: $p"
done
[[ -e $ws/.git ]] && fail "a non-git target came out as a git repo"
echo "ok: engine-only paths stay out of the workspace"

# ── 7. coverage: every file AGENTS.md says to always read is seeded ────────
# Derived from templates/workspace/AGENTS.md, not hardcoded — a new mandatory
# read without an embedded file fails here instead of on the next boot.
#   '## Identity' pulls files in with @-includes; '## Memory' names the two
#   always-reads in backticks.
cos="$templates/workspace/AGENTS.md"
includes=$(grep -oE '^@[A-Za-z0-9._-]+' "$cos" | sed 's/^@//') || true
[[ -n $includes ]] || fail "AGENTS.md has no @-includes — coverage check is vacuous"
for f in $includes; do
  [[ -f $ws/$f ]] || fail "AGENTS.md includes @$f but init does not seed it"
done
# shellcheck disable=SC2016  # the backticks are literal markdown, not expansion
always=$(awk '/^## Memory$/{f=1;next} f&&/^## /{exit} f' "$cos" \
  | grep -m1 'Always read' | sed -E 's/`\. .*$/`/' \
  | grep -oE '`[A-Za-z0-9._/-]+`' | tr -d '`' || true)
[[ -n $always ]] || fail "AGENTS.md '## Memory' has no 'Always read' line — coverage check is vacuous"
for f in $always; do
  [[ -f $ws/$f ]] || fail "AGENTS.md always reads '$f' but init does not seed it"
done
# shellcheck disable=SC2086  # $includes/$always are newline-separated lists, split on purpose
echo "ok: AGENTS.md always-reads seeded ($(printf '%s\n' $includes $always | sort -u | tr '\n' ' '))"

# ── 8. every template stays trackable in the engine repo ───────────────────
# .gitignore's `memory/` and `projects/` match at ANY depth, so a template
# under templates/memory/ would be ignored and never ship. Guard: the exact
# files build.rs walks must all be committable.
git -C "$root" rev-parse --git-dir >/dev/null 2>&1 \
  || fail "engine repo is not a git repo — the check-ignore guard would be vacuous"
while IFS= read -r rel; do
  if out=$(git -C "$root" check-ignore -v "$rel" 2>&1); then
    fail "template is gitignored and could never ship: $rel ($out)"
  fi
done < <(cd "$root" && find templates -type f | sort)
[[ $(cd "$root" && find templates -type f | wc -l | tr -d ' ') -gt 0 ]] \
  || fail "no templates/ under $root — nothing to guard"
# the guard must actually fire — a nested template is the trap this guards
probe="$T/probe"
mkdir -p "$probe/templates/memory"
cp "$root/.gitignore" "$probe/.gitignore"
git -C "$probe" init -q
: > "$probe/templates/memory/rules.md"
git -C "$probe" check-ignore -q "templates/memory/rules.md" \
  || fail "guard is vacuous: a nested templates/memory/ is not ignored — the check proves nothing"
echo "ok: every templates/ file is trackable (depth-safe names; guard verified)"

# ── 9. git hooks wired: core.hooksPath -> .git/quaqua-hooks, never over────
# The pre-commit portable-paths gate only runs if init points git at
# .git/quaqua-hooks. Three states must hold: fresh repo -> wired; already ours
# -> silent no-op; a different value (local or global) -> warned and left.
G="$T/repo-ws"
git -C "$T" init -q repo-ws
exp="$G/.git/quaqua-hooks"
( unset HERDR_PANE_ID; cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$G" ) >/dev/null \
  || fail "init failed on a git target"
[[ -d $exp ]] || fail "git target has no .git/quaqua-hooks dir to wire"
[[ "$(git -C "$G" config --get core.hooksPath)" == "$exp" ]] \
  || fail "init did not set core.hooksPath (want $exp)"
[[ -x $exp/pre-commit ]] || fail ".git/quaqua-hooks/pre-commit is not executable"
echo "ok: fresh git target -> core.hooksPath wired to .git/quaqua-hooks"

# idempotent when it is already ours (and quiet about it)
out=$( ( unset HERDR_PANE_ID; cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$G" ) 2>&1 )
[[ "$(git -C "$G" config --get core.hooksPath)" == "$exp" ]] || fail "re-run changed core.hooksPath"
[[ $out != *hooksPath* ]] || fail "re-run should stay quiet about a value it already owns: $out"
echo "ok: re-run leaves an already-correct core.hooksPath untouched and silent"

# a DIFFERENT local value is policy init does not own: warn + leave
git -C "$G" config core.hooksPath /somewhere/else
out=$( ( unset HERDR_PANE_ID; cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$G" ) 2>&1 )
[[ "$(git -C "$G" config --get core.hooksPath)" == "/somewhere/else" ]] \
  || fail "init overwrote an existing core.hooksPath"
[[ $out == *"already set"* ]] || fail "no warning about the foreign core.hooksPath: $out"
[[ $out == *"/somewhere/else"* ]] || fail "warning does not name the existing value: $out"
echo "ok: foreign local core.hooksPath -> warned, not overwritten"
git -C "$G" config --unset core.hooksPath

# a different GLOBAL value is left alone too (local scope must stay unset)
GC="$T/global.gitconfig"
printf '[core]\n\thooksPath = /global/hooks\n' > "$GC"
out=$( export GIT_CONFIG_GLOBAL="$GC"; unset HERDR_PANE_ID
       cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$G" 2>&1 ) \
  || fail "run failed with a global core.hooksPath set"
[[ -z "$(GIT_CONFIG_GLOBAL="$GC" git -C "$G" config --local --get core.hooksPath 2>/dev/null || true)" ]] \
  || fail "init set a local core.hooksPath over a global one"
[[ $out == *"already set"* ]] || fail "no warning about the foreign global core.hooksPath: $out"
[[ $(GIT_CONFIG_GLOBAL="$GC" git -C "$G" config --get core.hooksPath) == "/global/hooks" ]] \
  || fail "the global core.hooksPath value was modified"
echo "ok: foreign global core.hooksPath -> warned, not overwritten"

# legacy migration: scripts/hooks -> .git/quaqua-hooks
git -C "$G" config core.hooksPath "$G/scripts/hooks"
out=$( ( unset HERDR_PANE_ID; cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$G" ) 2>&1 )
[[ "$(git -C "$G" config --get core.hooksPath)" == "$exp" ]] \
  || fail "legacy core.hooksPath was not migrated"
[[ $out == *"migrated"* ]] || fail "no migration log: $out"
echo "ok: legacy core.hooksPath -> migrated to .git/quaqua-hooks"

# ── 10. herdr-quaqua: ONLY the real CoS pane is tagged orchestrator (R5) ───
# init runs from whatever pane drives the workspace, and every worker worktree
# of an engine repo ships a copy of it, so the caller's pane is not the CoS.
# Regression this covers (2026-10-14): an opencode pane came out of here tagged
# grid_role=orchestrator/grid_owner=cos while the real CoS pane had no token, so
# grid-workers.sh anchored the grid beside the wrong pane and timed out.
tag='herdr pane report-metadata w1:p1 --source herdr-quaqua --token grid_role=cos --token grid_owner=cos'
mkdir -p "$T/wt" "$T/elsewhere"   # a worker worktree and a non-workspace cwd

# tag_as <pane> <agent> <cwd> <agent-name>: run init as if called from <pane>,
# which herdr reports as <agent> in <cwd>, registered as <agent-name>
# ('' = unregistered). Prints the herdr calls init made.
tag_as() {
  : > "$T/calls"
  ( export CALLS="$T/calls" HERDR_PANE_ID="$1" PANE_CURRENT_ID="$1" \
           PANE_AGENT="$2" PANE_CWD="$3" PA_PANE_ID="$1" PA_AGENT="$2" PA_AGENT_NAME="$4"
    cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$ws" ) >/dev/null \
    || fail "init failed for pane $1 ($2/${4:-unnamed})"
  cat "$T/calls"
}

# the CoS itself — an unregistered Claude agent in the workspace — is tagged
out=$(tag_as w1:p1 claude "$ws" "")
grep -qF "$tag" <<<"$out" \
  || fail "the CoS pane (claude, unregistered, in the workspace) was not tagged: $out"
! grep -q 'grid_ready' <<<"$out" || fail "CoS pane should not have grid_ready token"
echo "ok: herdr-quaqua -> the CoS pane is tagged cos/owner=cos, no grid_ready"

# every other caller stays untagged: a stray orchestrator token anchors the grid
# beside the wrong pane
out=$(tag_as w1:p2 opencode /elsewhere sude-scan)
! grep -q 'report-metadata' <<<"$out" || fail "opencode pane tagged as the CoS orchestrator: $out"
out=$(tag_as w1:p3 claude "$T/wt" sude-fix)          # a claude worker in its worktree
! grep -q 'report-metadata' <<<"$out" || fail "claude worker pane tagged as the CoS orchestrator: $out"
out=$(tag_as w1:p4 claude "$ws" acme-lead)           # a Lead: same cwd as the CoS
! grep -q 'report-metadata' <<<"$out" || fail "Lead pane tagged as the CoS orchestrator: $out"
out=$(tag_as w1:p5 claude "$T/elsewhere" "")         # unregistered, but not this workspace
! grep -q 'report-metadata' <<<"$out" || fail "unregistered claude pane outside the workspace tagged as the CoS: $out"
echo "ok: herdr-quaqua -> opencode / claude-worker / Lead / foreign-dir panes stay untagged"

# with HERDR_PANE_ID unset the pane still comes from herdr pane current
: > "$T/calls"
( export CALLS="$T/calls" PANE_CURRENT_ID="w1:p1" PANE_AGENT=claude PANE_CWD="$ws" \
         PA_PANE_ID="w1:p1" PA_AGENT=claude
  unset HERDR_PANE_ID PA_AGENT_NAME
  cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$ws" ) >/dev/null \
  || fail "init failed with herdr pane current"
grep -qF "$tag" "$T/calls" \
  || fail "the CoS pane was not tagged via herdr pane current: $(cat "$T/calls")"
echo "ok: herdr-quaqua -> herdr pane current fallback still tags the CoS pane"

# a herdr that cannot say who the pane is -> no tag, and no failure
: > "$T/calls"
( export CALLS="$T/calls" HERDR_PANE_ID="w1:p1" PANE_CURRENT_ID="w1:p1"
  unset PANE_AGENT PANE_CWD PA_PANE_ID PA_AGENT_NAME
  cd "$empty" && "${QUAQUA_BIN:-quaqua}" init "$ws" ) >/dev/null \
  || fail "init failed when herdr reports no pane metadata"
! grep -q 'report-metadata' "$T/calls" || fail "tagged an unidentifiable pane: $(cat "$T/calls")"
echo "ok: herdr-quaqua -> an unidentifiable pane fails closed (no tag, no error)"

echo "PASS"
