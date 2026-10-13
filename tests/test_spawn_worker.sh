#!/usr/bin/env bash
# Sandbox test for spawn-worker.sh name validation (models.md Naming:
# <kind>-<task>, ascii [a-z0-9-], <=20 chars), the optional --base <ref>
# worktree start point, the --review read-only mode for codex kinds, and its
# best-effort grid hook.
# git+herdr are stubbed on PATH; the grid hook is stubbed next to a copy of the
# script. Bad names must exit 1 before either is called; good names proceed.
# The accented case runs under C and, when installed, a collating UTF-8 locale.
# No real worktree, home, or herdr touched.
# Usage: tests/test_spawn_worker.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-spawn-worker.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── sandbox: stub git/herdr that log every call, temp HOME, dummy repo ───────
mkdir -p "$T/bin" "$T/repo" "$T/home/quaqua/runs" "$T/scripts/worker"
cp "$src/worker/spawn-worker.sh" "$T/scripts/worker/spawn-worker.sh"
cp -R "$src/lib" "$T/scripts/"
cat > "$T/scripts/worker/grid-workers.sh" <<'STUB'
#!/usr/bin/env bash
printf 'grid%s\n' "${*:+ $*}" >> "$GRID_CALLS"
exit "${GRID_FAKE_RC:-0}"
STUB
cat > "$T/bin/git" <<'STUB'
#!/usr/bin/env bash
echo "git $*" >> "$CALLS"
# a ref containing "badref" does not exist: rev-parse --verify fails, and a
# worktree-add start point with it would too
case " $* " in *badref*) exit 1 ;; esac
# a bare `git [-C <r>] remote` lists the fake remotes (FAKE_REMOTES, space-sep)
if [ "${@: -1}" = remote ]; then printf '%s\n' ${FAKE_REMOTES:-}; fi
# MKMCP=1: the new worktree ships a project .mcp.json
if [ -n "${MKMCP:-}" ] && [ "${3:-} ${4:-}" = "worktree add" ]; then
  for a in "$@"; do case $a in */.wt/*) mkdir -p "$a" && echo '{}' > "$a/.mcp.json" ;; esac; done
fi
exit 0
STUB
cat > "$T/bin/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "$CALLS"
[ "${1:-} ${2:-}" = "pane split" ] && echo '{"result":{"pane":{"pane_id":"%1"}}}'
if [ "${1:-} ${2:-}" = "pane wait-output" ]; then  # $DIALOGS = dialogs left to show
  n=$(cat "${DIALOGS:-/dev/null}" 2>/dev/null || echo 0); [ "${n:-0}" -gt 0 ] || exit 1
  echo $((n - 1)) > "$DIALOGS"
fi
exit 0
STUB
chmod +x "$T/bin/git" "$T/bin/herdr"

spawn() {  # spawn <name> <kind> [VAR=val...]; script opts via array $spawn_args.
  # Sets rc, $T/out, $T/err, $T/calls, $T/gridcalls
  local name=$1 kind=$2; shift 2
  : > "$T/calls"; : > "$T/gridcalls"
  rc=0
  CALLS="$T/calls" GRID_CALLS="$T/gridcalls" HOME="$T/home" PATH="$T/bin:$PATH" LC_ALL="${SPAWN_LC_ALL:-C}" \
    env "$@" bash "$T/scripts/worker/spawn-worker.sh" ${spawn_args+"${spawn_args[@]}"} "$name" "$kind" "$T/repo" >"$T/out" 2>"$T/err" || rc=$?
}

expect_reject() {  # expect_reject <name> <kind> <label> [locale]
  SPAWN_LC_ALL="${4:-C}" spawn "$1" "$2"
  [[ $rc -eq 1 ]] || fail "$3: exit $rc, want 1"
  grep -q '^# Usage:' "$T/err" || fail "$3: no usage on stderr"
  [[ ! -s "$T/calls" ]] || fail "$3: git/herdr called before rejection: $(cat "$T/calls")"
  echo "ok: $3 -> exit 1, no git/herdr"
}

expect_accept() {  # expect_accept <name> <kind> <label>
  spawn "$1" "$2"
  [[ $rc -eq 0 ]] || fail "$3: exit $rc, want 0: $(cat "$T/err")"
  grep -q '^git ' "$T/calls" || fail "$3: git not called"
  grep -q '^herdr ' "$T/calls" || fail "$3: herdr not called"
  grep -q "worker=$1 kind=$2" "$T/out" || fail "$3: no success line"
  echo "ok: $3 -> accepted, git+herdr called"
}

# ── bad names: rejected before any git/herdr call ────────────────────────────
expect_reject mdinput tieude "no kind prefix"
expect_reject tieude_x tieude "underscore"
expect_reject sude-foo tieude "kind mismatch"
expect_reject tieude-Foo tieude "uppercase"
expect_reject tieude-abcdefghijklmn tieude "21 chars"
expect_reject sude-lead sude "-lead suffix (reserved for project Leads)"
expect_reject ni-x-lead ni "task ending in -lead"

# ── accented names: rejected under C and under a collating UTF-8 locale ──────
# bash's [a-z] range follows the locale collation; the guard must not.
expect_reject tieude-é tieude "accented name (C)"
utf8=$(locale -a 2>/dev/null | grep -i -m1 '^en_US\.utf-\?8$' || true)
if [ -n "$utf8" ]; then
  expect_reject tieude-é tieude "accented name ($utf8)" "$utf8"
else
  echo "skip: no collating UTF-8 locale (en_US.utf8) installed"
fi

# ── good names: pass the check and proceed to git/herdr ──────────────────────
expect_accept tieude-spawnguard tieude "17 chars"
expect_accept tieude-abcdefghijklm tieude "20 chars (boundary)"
expect_accept sude-leadname sude "leadname suffix is not -lead"

# ── detu model must be provider-qualified (bare name -> venice, no key) ────
spawn detu-lookup detu
[[ $rc -eq 0 ]] || fail "detu: exit $rc, want 0: $(cat "$T/err")"
grep -q -- '--kind pi .*--model antigravity/gemini-3.8-flash' "$T/calls" \
  || fail "detu: model not antigravity/gemini-3.8-flash: $(cat "$T/calls")"
echo "ok: detu -> --model antigravity/gemini-3.8-flash"

# ── grid hook: a successful spawn arranges the new worker's pane ─────────────
spawn tieude-gridhook tieude
[[ $rc -eq 0 ]] || fail "grid hook: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/gridcalls")" == "grid --tab repo-workers" ]] || fail "grid hook: not called: $(cat "$T/gridcalls")"
grep -q "worker=tieude-gridhook" "$T/out" || fail "grid hook: no success line"
[[ "$(cat "$T/home/quaqua/runs/tieude-gridhook.grid")" == repo-workers ]] \
  || fail "grid hook: runs/<name>.grid not <repo>-workers"
echo "ok: grid hook -> called after a successful spawn, group recorded"

spawn tieude-gridtab tieude GRID_TAB=coucou-workers
[[ $rc -eq 0 ]] || fail "GRID_TAB: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/gridcalls")" == "grid --tab coucou-workers" ]] \
  || fail "GRID_TAB: not passed as --tab: $(cat "$T/gridcalls")"
[[ "$(cat "$T/home/quaqua/runs/tieude-gridtab.grid")" == coucou-workers ]] \
  || fail "GRID_TAB: runs/<name>.grid not coucou-workers"
echo "ok: GRID_TAB -> grid-workers.sh --tab coucou-workers, group recorded"

spawn tieude-gridskip tieude QUAQUA_TEST=1 GRID_SKIP=1
[[ $rc -eq 0 ]] || fail "GRID_SKIP: exit $rc, want 0: $(cat "$T/err")"
[[ ! -s "$T/gridcalls" ]] || fail "GRID_SKIP=1: grid called: $(cat "$T/gridcalls")"
spawn tieude-gridskip tieude PIPELINE_TASK=t GRID_SKIP=1
[[ ! -s "$T/gridcalls" ]] || fail "pipeline GRID_SKIP=1: grid called: $(cat "$T/gridcalls")"
echo "ok: GRID_SKIP=1 under tests/pipeline -> grid not called"

spawn tieude-gridskip tieude GRID_SKIP=1
[[ $rc -eq 0 ]] || fail "bare GRID_SKIP: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/gridcalls")" == "grid --tab repo-workers" ]] || fail "bare GRID_SKIP=1: grid not called: $(cat "$T/gridcalls")"
grep -q 'GRID_SKIP=1 ignored' "$T/err" || fail "bare GRID_SKIP=1: no warning"
echo "ok: bare GRID_SKIP=1 -> ignored with a warning, grid called"

# ── pi + project MCP config: each "Allow project MCP server" dialog -> Allow ─
echo 2 > "$T/dialogs"
spawn tieude-mcp tieude DIALOGS="$T/dialogs" MKMCP=1
[[ $rc -eq 0 ]] || fail "mcp dialog: exit $rc, want 0: $(cat "$T/err")"
[[ $(grep -c 'pane send-keys %1 down enter' "$T/calls") -eq 2 ]] \
  || fail "mcp dialog: want 2 Allow answers: $(grep send-keys "$T/calls")"
echo "ok: pi + .mcp.json -> each MCP approval dialog answered Allow"
spawn tieude-nomcp tieude
! grep -q 'wait-output' "$T/calls" || fail "no .mcp.json: dialog wait ran"
echo "ok: pi without project MCP config -> no dialog wait"

spawn tieude-gridfail tieude GRID_FAKE_RC=1
[[ $rc -eq 0 ]] || fail "grid failure: exit $rc, want 0: $(cat "$T/err")"
grep -q "warning: grid-workers.sh failed" "$T/err" || fail "grid failure: no warning"
grep -q "worker=tieude-gridfail" "$T/out" || fail "grid failure: no success line"
echo "ok: grid failure -> warning only, spawn succeeds"

# ── --review: codex kinds only, repo read-only, $HOME/quaqua/runs writable ───
# The repo must stay read-only while runs stays writable; codex only honours
# --add-dir under workspace-write, so the guard uses a permission profile
# extending the built-in :read-only one.
spawn_args=(--review)
spawn quansu-revread quansu
unset spawn_args
[[ $rc -eq 0 ]] || fail "--review quansu: exit $rc, want 0: $(cat "$T/err")"
grep -q -- '--kind codex' "$T/calls" || fail "--review quansu: not a codex start: $(cat "$T/calls")"
grep -q -- '-a never' "$T/calls" || fail "--review quansu: no -a never: $(cat "$T/calls")"
grep -q -- 'default_permissions="quaqua-review"' "$T/calls" \
  || fail "--review quansu: no read-only profile: $(cat "$T/calls")"
grep -q -- 'permissions.quaqua-review.extends=":read-only"' "$T/calls" \
  || fail "--review quansu: profile does not extend :read-only: $(cat "$T/calls")"
grep -qF -- "permissions.quaqua-review.filesystem={\"$T/home/quaqua/runs\"=\"write\"}" "$T/calls" \
  || fail "--review quansu: runs not writable: $(cat "$T/calls")"
! grep -q -- '--approve-for-me' "$T/calls" \
  || fail "--review quansu: --approve-for-me conflicts with read-only: $(cat "$T/calls")"
! grep -q -- '--add-dir' "$T/calls" \
  || fail "--review quansu: --add-dir ignored under read-only: $(cat "$T/calls")"
! grep -q -- ' -s read-only' "$T/calls" \
  || fail "--review quansu: -s read-only would block runs writes: $(cat "$T/calls")"
grep -q "worker=quansu-revread" "$T/out" || fail "--review quansu: no success line"
echo "ok: --review quansu -> read-only profile + runs writable, no --approve-for-me/--add-dir"

# --review is accepted for both codex kinds and leaves non-review spawns untouched
spawn_args=(--review)
spawn bacdau-revread bacdau
unset spawn_args
[[ $rc -eq 0 ]] || fail "--review bacdau: exit $rc, want 0: $(cat "$T/err")"
grep -q -- 'default_permissions="quaqua-review"' "$T/calls" \
  || fail "--review bacdau: no read-only profile: $(cat "$T/calls")"
echo "ok: --review bacdau -> read-only profile"

spawn quansu-write quansu
[[ $rc -eq 0 ]] || fail "no --review quansu: exit $rc, want 0: $(cat "$T/err")"
grep -q -- '--approve-for-me' "$T/calls" || fail "no --review quansu: no --approve-for-me"
grep -q -- "--add-dir $T/home/quaqua/runs" "$T/calls" \
  || fail "no --review quansu: no --add-dir: $(cat "$T/calls")"
! grep -q -- 'default_permissions' "$T/calls" \
  || fail "no --review quansu: review profile leaked: $(cat "$T/calls")"
echo "ok: no --review -> --approve-for-me + --add-dir, unchanged"

# --review on a non-codex kind: refused before any git/herdr call
expect_reject_review() {  # expect_reject_review <name> <kind> <label>
  spawn_args=(--review)
  spawn "$1" "$2"
  unset spawn_args
  [[ $rc -eq 1 ]] || fail "$3: exit $rc, want 1"
  grep -q '^# Usage:' "$T/err" || fail "$3: no usage on stderr"
  grep -q "error: --review is only supported for codex kinds" "$T/err" \
    || fail "$3: no clear refusal: $(cat "$T/err")"
  [[ ! -s "$T/calls" ]] || fail "$3: git/herdr called before rejection: $(cat "$T/calls")"
  echo "ok: $3 -> exit 1, no git/herdr"
}
expect_reject_review tieude-revbad tieude "--review on tieude"
expect_reject_review sude-revbad sude "--review on sude"
expect_reject_review ni-revbad ni "--review on ni"
expect_reject_review suhuynh-revbad suhuynh "--review on suhuynh"
expect_reject_review truonglao-revbad truonglao "--review on truonglao"

# ── --base <ref>: branch starts at <ref>, validated before any side effect ──
spawn_args=(--base quaqua/tieude-m12bt)
spawn tieude-basefix tieude
unset spawn_args
[[ $rc -eq 0 ]] || fail "--base: exit $rc, want 0: $(cat "$T/err")"
grep -q "rev-parse --verify .*quaqua/tieude-m12bt" "$T/calls" \
  || fail "--base: ref not verified before worktree: $(cat "$T/calls")"
grep -q "worktree add -b quaqua/tieude-basefix .* quaqua/tieude-m12bt\$" "$T/calls" \
  || fail "--base: worktree add missing start point: $(cat "$T/calls")"
echo "ok: --base -> rev-parse --verify, worktree add starts at quaqua/tieude-m12bt"

# no flag: worktree add keeps taking repo HEAD (no start point, no rev-parse)
spawn tieude-nobase tieude
[[ $rc -eq 0 ]] || fail "no --base: exit $rc, want 0: $(cat "$T/err")"
! grep -q "rev-parse" "$T/calls" || fail "no --base: rev-parse ran: $(cat "$T/calls")"
grep -q "worktree add -b quaqua/tieude-nobase .*/tieude-nobase\$" "$T/calls" \
  || fail "no --base: worktree add has a start point: $(cat "$T/calls")"
echo "ok: no --base -> worktree add <wt> only (HEAD), no rev-parse"

# unknown ref: clear error, non-zero, no worktree or pane
spawn_args=(--base badref)
spawn tieude-badbase tieude
unset spawn_args
[[ $rc -ne 0 ]] || fail "--base badref: exit $rc, want non-zero"
grep -q "error: --base 'badref'" "$T/err" || fail "--base badref: no clear error: $(cat "$T/err")"
! grep -q "worktree add" "$T/calls" || fail "--base badref: worktree created: $(cat "$T/calls")"
! grep -q "^herdr " "$T/calls" || fail "--base badref: herdr called: $(cat "$T/calls")"
echo "ok: --base badref -> rejected before worktree/pane"

# --base without a value: usage error, nothing called
spawn_args=(--base=)
spawn tieude-noval tieude
unset spawn_args
[[ $rc -eq 1 ]] || fail "--base= empty: exit $rc, want 1"
grep -q '^# Usage:' "$T/err" || fail "--base= empty: no usage on stderr"
[[ ! -s "$T/calls" ]] || fail "--base= empty: git/herdr called: $(cat "$T/calls")"
echo "ok: --base= empty -> exit 1, no git/herdr"

: > "$T/calls"; : > "$T/gridcalls"
rc=0
CALLS="$T/calls" GRID_CALLS="$T/gridcalls" HOME="$T/home" PATH="$T/bin:$PATH" LC_ALL=C \
  bash "$T/scripts/worker/spawn-worker.sh" tieude-noval tieude "$T/repo" --base >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 1 ]] || fail "trailing --base: exit $rc, want 1"
grep -q '^# Usage:' "$T/err" || fail "trailing --base: no usage on stderr"
[[ ! -s "$T/calls" ]] || fail "trailing --base: git/herdr called: $(cat "$T/calls")"
echo "ok: trailing --base w/o value -> exit 1, no git/herdr"

# ── fresh clone: no pre-existing ~/quaqua/runs — spawn must still succeed ────
rm -rf "$T/home/quaqua"
spawn tieude-fresh tieude
[[ $rc -eq 0 ]] || fail "fresh clone: exit $rc, want 0: $(cat "$T/err")"
[[ -f $T/home/quaqua/runs/tieude-fresh.rules.md ]] \
  || fail "fresh clone: rules dump not written under a brand-new runs/"
echo "ok: fresh clone -> runs/ created before the rules dump, spawn succeeds"

# ── push block: spawn rewires each remote's pushurl in the worktree only ────
spawn tieude-pblock tieude FAKE_REMOTES='origin upstream'
[[ $rc -eq 0 ]] || fail "pblock spawn: exit $rc, want 0: $(cat "$T/err")"
grep -q 'config extensions.worktreeConfig true' "$T/calls" \
  || fail "pblock: extensions.worktreeConfig not enabled: $(cat "$T/calls")"
[[ $(grep -c 'config --worktree remote\.\(origin\|upstream\)\.pushurl PUSH DISABLED' "$T/calls") -eq 2 ]] \
  || fail "pblock: pushurl not set per remote: $(cat "$T/calls")"
echo "ok: push block -> worktreeConfig + per-remote --worktree pushurl on spawn"

# ── real git: a surviving worktree is reused, a foreign dir is refused ────────
mkdir -p "$T/bin2" "$T/rgrepo"
git -C "$T/rgrepo" init -q
git -C "$T/rgrepo" -c user.email=t@t -c user.name=t commit -qm init --allow-empty
git -C "$T/rgrepo" remote add origin git@example.com:o/r.git
cat > "$T/bin2/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "$CALLS"
[ "${1:-} ${2:-}" = "pane split" ] && echo '{"result":{"pane":{"pane_id":"%1"}}}'
exit 0
STUB
chmod +x "$T/bin2/herdr"

# first spawn creates the real worktree
: > "$T/calls"
rc=0
CALLS="$T/calls" HOME="$T/home" PATH="$T/bin2:$PATH" QUAQUA_TEST=1 GRID_SKIP=1 \
  bash "$T/scripts/worker/spawn-worker.sh" sude-rw1 sude "$T/rgrepo" >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 0 ]] || fail "rw1 spawn: exit $rc: $(cat "$T/err")"
[[ -d $T/.wt/sude-rw1 ]] || fail "rw1: no worktree created"

# a second spawn reuses the registered worktree instead of failing on '-b'
rm -f "$T/failed-sude-rw1"
: > "$T/calls"
rc=0
CALLS="$T/calls" HOME="$T/home" PATH="$T/bin2:$PATH" QUAQUA_TEST=1 GRID_SKIP=1 \
  bash "$T/scripts/worker/spawn-worker.sh" sude-rw1 sude "$T/rgrepo" >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 0 ]] || fail "rw1 respawn: exit $rc, want 0: $(cat "$T/err")"
grep -q 'reusing it' "$T/err" || fail "rw1 respawn: no reuse note: $(cat "$T/err")"
grep -q 'agent start sude-rw1' "$T/calls" || fail "rw1 respawn: agent start not retried"
echo "ok: respawn -> surviving worktree reused, agent start retried"

# a foreign dir at the worktree path is refused, never reused
mkdir -p "$T/.wt/sude-rw2"
: > "$T/calls"
rc=0
CALLS="$T/calls" HOME="$T/home" PATH="$T/bin2:$PATH" QUAQUA_TEST=1 GRID_SKIP=1 \
  bash "$T/scripts/worker/spawn-worker.sh" sude-rw2 sude "$T/rgrepo" >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 1 ]] || fail "rw2 foreign dir: exit $rc, want 1"
grep -q 'not a git worktree' "$T/err" || fail "rw2: no refusal message: $(cat "$T/err")"
! grep -q 'agent start' "$T/calls" || fail "rw2: agent started over a foreign dir"
echo "ok: foreign dir at worktree path -> refused before any agent start"

# ── real git push block: worker worktree cannot push, main checkout can ──────
# sude-rw1 went through the reuse-respawn path above — proof the push block
# also applies on a resumed spawn, not only on a fresh worktree add
[[ $(git -C "$T/.wt/sude-rw1" remote get-url --push origin) == PUSH\ DISABLED* ]] \
  || fail "rw1: pushurl not blocked on a reused worktree: $(git -C "$T/.wt/sude-rw1" remote get-url --push origin)"
rc=0
CALLS="$T/calls" HOME="$T/home" PATH="$T/bin2:$PATH" QUAQUA_TEST=1 GRID_SKIP=1 \
  bash "$T/scripts/worker/spawn-worker.sh" sude-nopush sude "$T/rgrepo" >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 0 ]] || fail "nopush spawn: exit $rc: $(cat "$T/err")"
wt_np="$T/.wt/sude-nopush"
[[ $(git -C "$wt_np" remote get-url --push origin) == 'PUSH DISABLED - task workers never push (AGENTS.md handoff rule)' ]] \
  || fail "nopush: worktree pushurl not blocked: $(git -C "$wt_np" remote get-url --push origin)"
[[ $(git -C "$T/rgrepo" remote get-url --push origin) == 'git@example.com:o/r.git' ]] \
  || fail "nopush: main checkout pushurl changed: $(git -C "$T/rgrepo" remote get-url --push origin)"
rc=0
git -C "$wt_np" push origin HEAD >"$T/push.out" 2>&1 || rc=$?
[[ $rc -ne 0 ]] || fail "nopush: git push from a worker worktree succeeded"
grep -q 'PUSH DISABLED' "$T/push.out" \
  || fail "nopush: push error is not a clear refusal: $(cat "$T/push.out")"
# fetch-side config is untouched, and enabling worktreeConfig left the shared
# config's push target alone
[[ $(git -C "$wt_np" remote get-url origin) == 'git@example.com:o/r.git' ]] \
  || fail "nopush: fetch url changed: $(git -C "$wt_np" remote get-url origin)"
echo "ok: real git -> worktree push fails with a clear error, main pushurl unchanged"

echo "PASS"
