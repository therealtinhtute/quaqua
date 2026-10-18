#!/usr/bin/env bash
# Sandbox test for spawn-lead.sh (SLP P4): slug validation (charset, the
# projects/INDEX.md row, the projects/<slug>.md card), the one-Lead-per-slug
# refusal via `herdr agent get`, the workspace/start/prompt call sequence (own
# <slug> workspace, cwd = the script's repo root, no worktree), the
# INDEX.md owner-column update, and the LEAD_KIND override.
# herdr and git are stubbed on PATH and log every call; the script runs
# against a temp repo root. No real herdr, INDEX.md, or pane touched.
# Usage: tests/test_spawn_lead.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-spawn-lead.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
unset HERDR_PANE_ID

# ── sandbox: temp quaqua root (script copy under scripts/) + stub herdr/git ─
mkdir -p "$T/bin" "$T/projects" "$T/rules/_all"

cat > "$T/projects/INDEX.md" <<'EOF'
# Projects

| slug | repo | owner | status | one-line goal |
|------|------|-------|--------|---------------|
| mono | ~/Lab/mono | cos | idle | prefix of mono-desk |
| mono-desk | ~/Lab/mono-desk | quaqua-monodesk-orchestrator | PR merged | desktop demo |
| coucou-island | ~/Lab/coucou-island | cos | M12 done | media island |
| no-card | ~/Lab/no-card | cos | idle | row without a card file |
| aaaaaaaaaaaaaaaaaaaaaaaaaaa | ~/Lab/a27 | cos | idle | 27-char slug -> 32-char Lead name |
| aaaaaaaaaaaaaaaaaaaaaaaaaaaa | ~/Lab/a28 | cos | idle | 28-char slug -> 33-char Lead name |
EOF
for s in mono mono-desk coucou-island aaaaaaaaaaaaaaaaaaaaaaaaaaa aaaaaaaaaaaaaaaaaaaaaaaaaaaa; do printf '# %s\n' "$s" > "$T/projects/$s.md"; done
printf '# all rules\n- r1 stay in the project\n' > "$T/rules/_all/all.md"
cp "$T/projects/INDEX.md" "$T/INDEX.orig"

cat > "$T/bin/git" <<'STUB'
#!/usr/bin/env bash
echo "git $*" >> "$CALLS"
case " $* " in
  *"symbolic-ref"*"refs/remotes/origin/HEAD"*)
    [[ -n ${GIT_ORIGIN_HEAD:-} ]] || exit 1
    printf '%s\n' "$GIT_ORIGIN_HEAD"; exit 0 ;;
  *"rev-parse --verify"*"refs/remotes/origin/"*)
    exit "${GIT_ORIGIN_HEAD_LOCAL:-1}" ;;
  *"rev-parse --verify"*"refs/heads/develop"*)  exit "${GIT_DEVELOP_MISSING:-0}" ;;
  *"rev-parse --verify"*"refs/heads/quaqua/"*) exit "${GIT_INTEG_MISSING:-0}" ;;
  *"rev-parse --verify"*"refs/heads/main"*)     exit "${GIT_MAIN_MISSING:-0}" ;;
  *"rev-parse --verify"*"refs/heads/master"*)   exit "${GIT_MASTER_MISSING:-0}" ;;
  *" branch quaqua/"*)                          exit "${GIT_BRANCH_FAIL:-0}" ;;
esac
exit 0
STUB
cat > "$T/bin/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "$CALLS"
case "${1:-} ${2:-}" in
  "agent get")
    # names in $HERDR_LIVE exist; anything else is agent_not_found (rc 1)
    if [[ " ${HERDR_LIVE:-} " == *" ${3:-} "* ]]; then
      printf '{"result":{"agent":{"name":"%s","agent_status":"idle","pane_id":"%%7","workspace_id":"w7"}}}\n' "$3"
    else
      printf '{"error":{"code":"agent_not_found","message":"agent target %s not found"}}\n' "${3:-}"
      exit 1
    fi ;;
  "workspace create") echo '{"result":{"workspace":{"workspace_id":"w7","label":"x"},"tab":{"tab_id":"w7:t1"},"root_pane":{"pane_id":"%7"}}}' ;;
esac
exit 0
STUB
chmod +x "$T/bin/git" "$T/bin/herdr"

spawn() {  # spawn <slug> [VAR=val...]; sets rc and $T/{out,err,calls}
  local slug=$1; shift
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" PATH="$T/bin:$PATH" QUAQUA_ROOT="$T" env "$@" \
    "${QUAQUA_BIN:-quaqua}" lead spawn "$slug" >"$T/out" 2>"$T/err" || rc=$?
}

ln() { grep -n "$1" "$T/calls" | head -1 | cut -d: -f1; }  # first matching call line

# ── happy path: get -> tab -> start -> prompt; INDEX owner updated ────────
spawn coucou-island
[[ $rc -eq 0 ]] || fail "happy: exit $rc, want 0: $(cat "$T/err")"
grep -qx 'lead=coucou-island-lead slug=coucou-island workspace=w7 pane=%7' "$T/out" \
  || fail "happy: no success line: $(cat "$T/out")"
[[ $(ln 'agent get') -lt $(ln 'workspace create') ]] \
  && [[ $(ln 'workspace create') -lt $(ln 'pane report-metadata') ]] \
  && [[ $(ln 'pane report-metadata') -lt $(ln 'agent start') ]] \
  && [[ $(ln 'agent start') -lt $(ln 'agent prompt') ]] \
  || fail "happy: bad call order: $(cat "$T/calls")"
grep -qF 'herdr pane report-metadata %7 --source herdr-grid --token grid_role=orchestrator --token grid_owner=coucou-island-lead' "$T/calls" \
  || fail "happy: Lead pane not tagged as orchestrator with owner: $(cat "$T/calls")"
! grep -q 'grid_ready' "$T/calls" || fail "happy: Lead pane should not have grid_ready token"
echo "ok: herdr-grid -> Lead pane tagged as orchestrator (role=orchestrator, owner=coucou-island-lead), no grid_ready"
grep -q 'agent get coucou-island-lead' "$T/calls" \
  || fail "happy: no duplicate check: $(cat "$T/calls")"
grep -qF "workspace create --label coucou-island --no-focus --cwd $T --env QUAQUA_OWNER=coucou-island-lead" "$T/calls" \
  || fail "happy: Lead not in its own <slug> workspace at the repo root: $(cat "$T/calls")"
grep -q 'agent start coucou-island-lead --kind opencode --pane %7 --timeout 60000 -- --standalone --auto' "$T/calls" \
  || fail "happy: bad agent start: $(cat "$T/calls")"
grep -qF "OPENCODE_CONFIG_CONTENT={\"model\":{\"providerID\":\"opencode-go\",\"model\":\"deepseek-v4.1-flash\",\"variant\":\"max\"},\"instructions\":[\"$T/runs/coucou-island-lead.rules.md\"]}" "$T/calls" \
  || fail "happy: opencode config content missing from workspace create: $(cat "$T/calls")"
grep -q 'r1 stay in the project' "$T/runs/coucou-island-lead.rules.md" \
  || fail "happy: rules file missing rules/_all content"
grep -q 'agent prompt coucou-island-lead' "$T/calls" \
  || fail "happy: no brief prompt: $(cat "$T/calls")"
for frag in 'Roles (SLP)' 'projects/coucou-island.md' 'quaqua pipeline' 'not counted toward the 4-worker cap' 'quaqua/coucou-island-integ' 'reaches you through the CoS'; do
  grep -qF "$frag" "$T/calls" || fail "happy: brief missing '$frag'"
done
! grep -qF 'record steering from anh iu or CoS' "$T/calls" \
  || fail "happy: brief still routes steering from anh iu around the CoS"
echo "ok: brief -> redirects reach the Lead through the CoS only"
# integration-branch check happens before the workspace create; no worktree is made
[[ $(ln 'agent get') -lt $(ln 'rev-parse --verify') ]] \
  && [[ $(ln 'rev-parse --verify') -lt $(ln 'workspace create') ]] \
  || fail "happy: integ check order wrong: $(cat "$T/calls")"
grep -qF "git -C $HOME/Lab/coucou-island rev-parse --verify --quiet refs/heads/quaqua/coucou-island-integ" "$T/calls" \
  || fail "happy: no integration-branch check on the project repo: $(cat "$T/calls")"
! grep -q '^git .*worktree' "$T/calls" || fail "happy: git worktree called — a Lead gets no worktree: $(cat "$T/calls")"
grep -qE '^\| coucou-island \| ~/Lab/coucou-island \| coucou-island-lead \| M12 done \|' \
  "$T/projects/INDEX.md" || fail "happy: owner cell not updated: $(cat "$T/projects/INDEX.md")"
diff <(grep -v '| coucou-island |' "$T/INDEX.orig") \
     <(grep -v '| coucou-island |' "$T/projects/INDEX.md") >/dev/null \
  || fail "happy: lines outside the slug row changed"
echo "ok: happy path -> get/workspace/start/prompt order, claude args, rules file, owner updated"

# ── duplicate: a registered Lead for the slug is refused before side effects ─
cp "$T/projects/INDEX.md" "$T/INDEX.prev"
spawn coucou-island HERDR_LIVE=coucou-island-lead
[[ $rc -ne 0 ]] || fail "duplicate: exit $rc, want non-zero"
grep -q "a Lead for 'coucou-island' already exists" "$T/err" \
  || fail "duplicate: no clear refusal: $(cat "$T/err")"
[[ $(grep -c '^herdr ' "$T/calls") -eq 1 ]] \
  || fail "duplicate: side effects after the refusal: $(cat "$T/calls")"
cmp -s "$T/projects/INDEX.md" "$T/INDEX.prev" \
  || fail "duplicate: INDEX.md touched"
echo "ok: duplicate -> refused after agent get, no workspace/start, INDEX untouched"

# ── bad slugs: usage/die before any herdr call ──────────────────────────────
expect_reject() {  # expect_reject <slug> <label> <stderr-fragment>
  spawn "$1"
  [[ $rc -eq 1 ]] || fail "$2: exit $rc, want 1: $(cat "$T/err")"
  grep -q "$3" "$T/err" || fail "$2: no '$3' on stderr: $(cat "$T/err")"
  [[ ! -s "$T/calls" ]] || fail "$2: herdr/git called before rejection: $(cat "$T/calls")"
  echo "ok: $2 -> exit 1, no herdr/git"
}
expect_reject Foo 'uppercase slug' '^# Usage:'
expect_reject bad_slug 'underscore slug' '^# Usage:'
expect_reject --bogus '--flag arg' '^# Usage:'
expect_reject -x 'single-dash slug' '^# Usage:'
expect_reject ghost 'unknown slug' "unknown project slug 'ghost'"
expect_reject no-card 'row but no card' 'no project card'
expect_reject aaaaaaaaaaaaaaaaaaaaaaaaaaaa 'lead name >32 chars (registered slug)' 'at most 32'

# ── boundary: 27-char slug -> 32-char Lead name is still accepted ───────────
spawn aaaaaaaaaaaaaaaaaaaaaaaaaaa
[[ $rc -eq 0 ]] || fail "boundary: exit $rc, want 0: $(cat "$T/err")"
grep -q 'agent start aaaaaaaaaaaaaaaaaaaaaaaaaaa-lead ' "$T/calls" \
  || fail "boundary: 32-char name not started: $(cat "$T/calls")"
echo "ok: 32-char lead name (27-char slug) accepted"

# ── INDEX update is exact-cell: 'mono' must not touch the mono-desk row ─────
spawn mono
[[ $rc -eq 0 ]] || fail "mono: exit $rc, want 0: $(cat "$T/err")"
grep -qE '^\| mono \| ~/Lab/mono \| mono-lead \|' "$T/projects/INDEX.md" \
  || fail "mono: owner cell not updated: $(cat "$T/projects/INDEX.md")"
grep -qE '^\| mono-desk \| ~/Lab/mono-desk \| quaqua-monodesk-orchestrator \|' "$T/projects/INDEX.md" \
  || fail "mono: mono-desk row was touched: $(cat "$T/projects/INDEX.md")"
echo "ok: 'mono' updates only its own row (mono-desk untouched)"

# ── LEAD_KIND overrides strategy.lead_kind ──────────────────────────────────
spawn coucou-island LEAD_KIND=truonglao
[[ $rc -eq 0 ]] || fail "env: exit $rc, want 0: $(cat "$T/err")"
grep -q 'agent start coucou-island-lead --kind claude --pane %7 --timeout 60000' "$T/calls" \
  || fail "env: LEAD_KIND not passed as claude: $(cat "$T/calls")"
grep -q -- '--model claude-opus-5-5 --effort high' "$T/calls" \
  || fail "env: truonglao model/effort not passed: $(cat "$T/calls")"
grep -qF -- "--append-system-prompt-file $T/runs/coucou-island-lead.rules.md" "$T/calls" \
  || fail "env: claude rules file arg missing: $(cat "$T/calls")"
echo "ok: LEAD_KIND overrides strategy.lead_kind"

# ── integration branch: created from the default branch before side effects ─
# no origin/HEAD, main present -> main
spawn coucou-island GIT_INTEG_MISSING=1
[[ $rc -eq 0 ]] || fail "integ: exit $rc, want 0: $(cat "$T/err")"
grep -qF "git -C $HOME/Lab/coucou-island branch quaqua/coucou-island-integ main" "$T/calls" \
  || fail "integ: branch not created from main: $(cat "$T/calls")"
[[ $(ln 'branch quaqua/') -lt $(ln 'workspace create') ]] \
  || fail "integ: branch creation must precede the workspace create: $(cat "$T/calls")"
echo "ok: missing quaqua/<slug>-integ -> created from main before the workspace"

# master-only repo (no origin/HEAD, no main) -> master
spawn coucou-island GIT_INTEG_MISSING=1 GIT_MAIN_MISSING=1
[[ $rc -eq 0 ]] || fail "master: exit $rc, want 0: $(cat "$T/err")"
grep -qF "git -C $HOME/Lab/coucou-island branch quaqua/coucou-island-integ master" "$T/calls" \
  || fail "master: branch not created from master: $(cat "$T/calls")"
echo "ok: master-only repo -> integration branch created from master"

# origin/HEAD wins over an existing main
expect_origin_head() {  # <label> <base-arg> [VAR=val...]
  local label=$1 basearg=$2; shift 2
  spawn coucou-island GIT_INTEG_MISSING=1 GIT_ORIGIN_HEAD_LOCAL=0 "$@"
  [[ $rc -eq 0 ]] || fail "$label: exit $rc, want 0: $(cat "$T/err")"
  grep -qF "git -C $HOME/Lab/coucou-island branch quaqua/coucou-island-integ $basearg" "$T/calls" \
    || fail "$label: branch not created from $basearg: $(cat "$T/calls")"
  echo "ok: $label -> integration branch created from $basearg"
}
expect_origin_head 'origin/HEAD set' 'develop' GIT_ORIGIN_HEAD=origin/develop
# origin/HEAD names a branch with no local ref -> its remote-tracking ref
expect_origin_head 'origin/HEAD without a local ref' 'origin/develop' \
  GIT_ORIGIN_HEAD=origin/develop GIT_DEVELOP_MISSING=1
# origin/HEAD set but that branch is gone everywhere -> fall back to main
expect_origin_head 'origin/HEAD stale' 'main' \
  GIT_ORIGIN_HEAD=origin/develop GIT_DEVELOP_MISSING=1 GIT_ORIGIN_HEAD_LOCAL=1

echo "ok: origin/HEAD set -> integration branch created from it (precedence over main)"

# no default branch at all -> refuse before any pane/agent side effect
spawn coucou-island GIT_INTEG_MISSING=1 GIT_MAIN_MISSING=1 GIT_MASTER_MISSING=1
[[ $rc -eq 1 ]] || fail "no default: exit $rc, want 1: $(cat "$T/err")"
grep -q 'no default branch' "$T/err" || fail "no default: unclear message: $(cat "$T/err")"
! grep -q 'workspace create' "$T/calls" || fail "no default: workspace create despite failure"
echo "ok: no default branch in the project repo -> refused before side effects"

# ── a stale owner cell does not block respawn when no agent is registered ───
spawn mono-desk
[[ $rc -eq 0 ]] || fail "stale owner: exit $rc, want 0: $(cat "$T/err")"
grep -q 'agent start mono-desk-lead' "$T/calls" \
  || fail "stale owner: start not called: $(cat "$T/calls")"
echo "ok: stale owner cell -> respawn allowed (agent registry is the truth)"

# ── the INDEX '~/repo' cell expands against HOME verbatim ───────────────────
# bash >= 5.2 patsub_replacement gives '&' in an unquoted replacement the
# meaning "the matched text", so a HOME containing '&' used to corrupt the
# path silently ("~/x" -> "<HOME with ~ for &>/x"). The replacement must be
# quoted; HOME is not guaranteed to be a plain path.
amp="$T/home&co"
mkdir -p "$amp"
spawn coucou-island HOME="$amp"
[[ $rc -eq 0 ]] || fail "tilde: exit $rc, want 0: $(cat "$T/err")"
grep -qF "git -C $amp/Lab/coucou-island rev-parse" "$T/calls" \
  || fail "tilde: '~' not expanded to a literal HOME path: $(grep 'git -C' "$T/calls")"
! grep -qF '~' <<<"$(grep 'git -C' "$T/calls")" \
  || fail "tilde: a '~' survived expansion in a git path: $(grep 'git -C' "$T/calls")"
echo "ok: INDEX '~/repo' expands to a literal HOME (no patsub_replacement corruption)"

# ── concurrent spawn vs cleanup on different INDEX rows must not lose updates ─
# Spawn and cleanup run concurrently on different rows. With the shared mkdir lock
# (projects/INDEX.md.lock.d), writes serialize without data loss or file corruption.
mkdir -p "$T/runs"; : > "$T/calls"
cp "$T/INDEX.orig" "$T/projects/INDEX.md"
ce=(CALLS="$T/calls" PATH="$T/bin:$PATH" QUAQUA_ROOT="$T")
env "${ce[@]}" "${QUAQUA_BIN:-quaqua}" lead spawn coucou-island >"$T/out.s" 2>"$T/err.s" & p1=$!
env "${ce[@]}" HERDR_LIVE=mono-desk-lead HERDR_PANE_ID=w9:p13 \
  "${QUAQUA_BIN:-quaqua}" lead cleanup mono-desk >"$T/out.c" 2>"$T/err.c" & p2=$!
rc1=0 rc2=0; wait $p1 || rc1=$?; wait $p2 || rc2=$?
[[ $rc1 -eq 0 && $rc2 -eq 0 ]] || fail "concurrent: exits $rc1/$rc2: $(cat "$T/err.s" "$T/err.c")"
grep -qE '^\| coucou-island \| ~/Lab/coucou-island \| coucou-island-lead \|' "$T/projects/INDEX.md" \
  || fail "concurrent: spawn update lost: $(cat "$T/projects/INDEX.md")"
grep -qE '^\| mono-desk \| ~/Lab/mono-desk \| cos \|' "$T/projects/INDEX.md" \
  || fail "concurrent: cleanup update lost: $(cat "$T/projects/INDEX.md")"
find "$T/projects" -name 'INDEX.md.*' | grep -q . \
  && fail "concurrent: leftover tmp/lock files: $(find "$T/projects" -name 'INDEX.md.*')"
echo "ok: concurrent spawn vs cleanup on different rows -> both updates kept"

echo "PASS"
