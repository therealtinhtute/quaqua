#!/usr/bin/env bash
# Sandbox test for spawn-worker.sh name validation (models.md Naming:
# <kind>-<task>, ascii [a-z0-9-], <=20 chars), the optional --base <ref>
# worktree start point, the --review read-only mode for codex and claude kinds,
# and its best-effort grid hook.
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
unset HERDR_PANE_ID

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
# FAKE_BARE / FAKE_WORKTREE: make the common config look like the degenerate
# repos the push block must refuse (core.bare=true / core.worktree set)
case " $* " in
  *" config --local --bool --get core.bare "*) [ -n "${FAKE_BARE:-}" ] && echo true; exit 0 ;;
  *" config --local --list "*) [ -n "${FAKE_WORKTREE:-}" ] && echo 'core.worktree=/tmp/fake'; exit 0 ;;
esac
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
  CALLS="$T/calls" GRID_CALLS="$T/gridcalls" HOME="$T/home" PATH="$T/bin:$PATH" LC_ALL="${SPAWN_LC_ALL:-C}" QUAQUA_OWNER=cos \
    env "$@" bash "$T/scripts/worker/spawn-worker.sh" ${spawn_args+"${spawn_args[@]}"} "$name" "$kind" "$T/repo" >"$T/out" 2>"$T/err" || rc=$?
}

ln_call() { grep -n "$1" "$T/calls" | head -1 | cut -d: -f1; }
ln_last() { grep -n "$1" "$T/calls" | tail -1 | cut -d: -f1; }

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

# ── tieude/detu: opencode with deepseek-v4.1-flash in the pane env ──────────
# (models.md) — the model travels in OPENCODE_CONFIG_CONTENT; no omp/gemini
for k in tieude detu; do
  spawn "$k-model" "$k"
  [[ $rc -eq 0 ]] || fail "$k: exit $rc, want 0: $(cat "$T/err")"
  grep -qF -- '--kind opencode' "$T/calls" \
    || fail "$k: not an opencode start: $(cat "$T/calls")"
  grep -qF -- 'OPENCODE_CONFIG_CONTENT={"model":"opencode-go/deepseek-v4.1-flash","instructions":' "$T/calls" \
    || fail "$k: model not in OPENCODE_CONFIG_CONTENT: $(cat "$T/calls")"
  ! grep -q -- '--kind omp' "$T/calls" || fail "$k: still spawned as omp: $(cat "$T/calls")"
  ! grep -q 'gemini' "$T/calls" || fail "$k: gemini flag leaked: $(cat "$T/calls")"
  echo "ok: $k -> --kind opencode, model opencode-go/deepseek-v4.1-flash"
done

# ── sude: omp with antigravity gemini-3.8-flash high, worker rules appended ──
# (models.md) — omp takes the model, the thinking level and the rules file on
# the command line (--append-system-prompt reads a file path), not in pane env
spawn sude-impl sude
[[ $rc -eq 0 ]] || fail "sude: exit $rc, want 0: $(cat "$T/err")"
grep -qF -- '--kind omp' "$T/calls" || fail "sude: not an omp start: $(cat "$T/calls")"
grep -qF -- '--model google-antigravity/gemini-3.8-flash' "$T/calls" \
  || fail "sude: model not google-antigravity/gemini-3.8-flash: $(cat "$T/calls")"
grep -qF -- '--thinking high' "$T/calls" || fail "sude: thinking level not high: $(cat "$T/calls")"
grep -qF -- '--auto-approve' "$T/calls" \
  || fail "sude: no --auto-approve (unattended spawn): $(cat "$T/calls")"
grep -qF -- "--append-system-prompt $T/home/quaqua/runs/sude-impl.rules.md" "$T/calls" \
  || fail "sude: worker rules file not appended: $(cat "$T/calls")"
! grep -q -- '--kind opencode' "$T/calls" || fail "sude: still an opencode start: $(cat "$T/calls")"
! grep -q 'OPENCODE_CONFIG_CONTENT' "$T/calls" \
  || fail "sude: opencode pane env leaked into an omp start: $(cat "$T/calls")"
echo "ok: sude -> --kind omp, google-antigravity/gemini-3.8-flash high, rules appended"

# ── grid hook: a successful spawn arranges the new worker's pane ─────────────
spawn tieude-gridhook tieude
[[ $rc -eq 0 ]] || fail "grid hook: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/gridcalls")" == "grid --check --tab repo-workers" ]] || fail "grid hook: not called: $(cat "$T/gridcalls")"
grep -q "worker=tieude-gridhook" "$T/out" || fail "grid hook: no success line"
[[ "$(cat "$T/home/quaqua/runs/tieude-gridhook.grid")" == repo-workers ]] \
  || fail "grid hook: runs/<name>.grid not <repo>-workers"
echo "ok: grid hook -> called after a successful spawn, group recorded"

spawn tieude-gridtab tieude GRID_TAB=coucou-workers
[[ $rc -eq 0 ]] || fail "GRID_TAB: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/gridcalls")" == "grid --check --tab coucou-workers" ]] \
  || fail "GRID_TAB: not passed as --tab: $(cat "$T/gridcalls")"
[[ "$(cat "$T/home/quaqua/runs/tieude-gridtab.grid")" == coucou-workers ]] \
  || fail "GRID_TAB: runs/<name>.grid not coucou-workers"
echo "ok: GRID_TAB -> grid-workers.sh --check --tab coucou-workers, group recorded"

spawn tieude-gridskip tieude QUAQUA_TEST=1 GRID_SKIP=1
[[ $rc -eq 0 ]] || fail "GRID_SKIP: exit $rc, want 0: $(cat "$T/err")"
[[ ! -s "$T/gridcalls" ]] || fail "GRID_SKIP=1: grid called: $(cat "$T/gridcalls")"
spawn tieude-gridskip tieude PIPELINE_TASK=t GRID_SKIP=1
[[ ! -s "$T/gridcalls" ]] || fail "pipeline GRID_SKIP=1: grid called: $(cat "$T/gridcalls")"
echo "ok: GRID_SKIP=1 under tests/pipeline -> grid not called"

spawn tieude-gridskip tieude GRID_SKIP=1
[[ $rc -eq 0 ]] || fail "bare GRID_SKIP: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/gridcalls")" == "grid --check --tab repo-workers" ]] || fail "bare GRID_SKIP=1: grid not called: $(cat "$T/gridcalls")"
grep -q 'GRID_SKIP=1 ignored' "$T/err" || fail "bare GRID_SKIP=1: no warning"
echo "ok: bare GRID_SKIP=1 -> ignored with a warning, grid called"

# ── opencode + project MCP config: no startup dialog wait needed ─────────────
spawn tieude-mcp tieude MKMCP=1
[[ $rc -eq 0 ]] || fail "mcp: exit $rc, want 0: $(cat "$T/err")"
! grep -q 'wait-output' "$T/calls" || fail "mcp: dialog wait ran"
echo "ok: opencode with project MCP config -> no dialog wait"
spawn tieude-nomcp tieude
! grep -q 'wait-output' "$T/calls" || fail "no .mcp.json: dialog wait ran"
echo "ok: opencode without project MCP config -> no dialog wait"

spawn tieude-gridfail tieude GRID_FAKE_RC=1
[[ $rc -eq 0 ]] || fail "grid failure: exit $rc, want 0: $(cat "$T/err")"
grep -q "warning: grid-workers.sh failed" "$T/err" || fail "grid failure: no warning"
grep -q "worker=tieude-gridfail" "$T/out" || fail "grid failure: no success line"
echo "ok: grid failure -> warning only, spawn succeeds"

# ── herdr-grid pane tokens (R5): role/group/owner on split, grid_ready=1 last ─
spawn tieude-tokens tieude
[[ $rc -eq 0 ]] || fail "tokens: exit $rc, want 0: $(cat "$T/err")"
grep -qF 'herdr pane report-metadata %1 --source herdr-grid --token grid_role=worker --token grid_group=repo --token grid_owner=cos' "$T/calls" \
  || fail "tokens: initial tokens missing: $(cat "$T/calls")"
grep -qF 'herdr pane report-metadata %1 --source herdr-grid --token grid_ready=1' "$T/calls" \
  || fail "tokens: grid_ready=1 missing: $(cat "$T/calls")"
[[ $(ln_call 'pane split') -lt $(ln_call 'grid_role=worker') ]] \
  && [[ $(ln_call 'grid_role=worker') -lt $(ln_call 'agent start') ]] \
  && [[ $(ln_call 'agent start') -lt $(ln_call 'grid_ready=1') ]] \
  || fail "tokens: bad call order: $(cat "$T/calls")"
echo "ok: pane tokens -> grid_role=worker/grid_group/grid_owner on split, grid_ready=1 after agent start"

# custom owner via QUAQUA_OWNER
spawn tieude-customowner tieude QUAQUA_OWNER=custom-lead
[[ $rc -eq 0 ]] || fail "custom owner: exit $rc, want 0: $(cat "$T/err")"
grep -qF 'herdr pane report-metadata %1 --source herdr-grid --token grid_role=worker --token grid_group=repo --token grid_owner=custom-lead' "$T/calls" \
  || fail "custom owner: grid_owner not set to custom-lead: $(cat "$T/calls")"
echo "ok: pane tokens -> grid_owner reflects QUAQUA_OWNER"


# ── --review: codex + claude kinds, repo read-only, runs writable ────────────
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
[[ $(ln_call 'agent wait') -lt $(ln_call 'grid_ready=1') ]] \
  || fail "--review quansu: grid_ready=1 set before codex idle wait: $(cat "$T/calls")"
echo "ok: --review quansu -> read-only profile + runs writable, no --approve-for-me/--add-dir, grid_ready=1 after idle wait"

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

# --review on an opencode/omp kind: refused before any git/herdr call
expect_reject_review() {  # expect_reject_review <name> <kind> <label>
  spawn_args=(--review)
  spawn "$1" "$2"
  unset spawn_args
  [[ $rc -eq 1 ]] || fail "$3: exit $rc, want 1"
  grep -q '^# Usage:' "$T/err" || fail "$3: no usage on stderr"
  grep -qF "error: --review is only supported for codex kinds (bacdau, quansu) and claude kinds" "$T/err" \
    || fail "$3: no clear refusal: $(cat "$T/err")"
  [[ ! -s "$T/calls" ]] || fail "$3: git/herdr called before rejection: $(cat "$T/calls")"
  echo "ok: $3 -> exit 1, no git/herdr"
}
expect_reject_review tieude-revbad tieude "--review on tieude"
expect_reject_review sude-revbad sude "--review on sude"

# ── --review claude kinds: repo read-only, $HOME/quaqua/runs writable ────────
# claude has no filesystem sandbox, so the codex quaqua-review profile is
# mirrored by --permission-mode dontAsk ("Auto-denies every call that would
# otherwise prompt ... tools pre-approved via permissions.allow rules still
# run"), so the unattended reviewer never stalls on a permission dialog, plus
# Edit() path rules — the only file rules the file tools consult (a Write(...)
# path rule is accepted but never checked). Read-only Bash (git diff/log/show,
# cat, grep) stays prompt-free in every mode, so the reviewer keeps its
# evidence commands; mutating Bash is denied wholesale.
for k in ni suhuynh truonglao; do
  case $k in ni) eff=low ;; suhuynh) eff=medium ;; truonglao) eff=high ;; esac
  spawn_args=(--review)
  spawn "$k-revread" "$k"
  unset spawn_args
  [[ $rc -eq 0 ]] || fail "--review $k: exit $rc, want 0: $(cat "$T/err")"
  grep -qF -- '--kind claude' "$T/calls" || fail "--review $k: not a claude start: $(cat "$T/calls")"
  grep -qF -- '--permission-mode dontAsk' "$T/calls" \
    || fail "--review $k: no dontAsk permission mode: $(cat "$T/calls")"
  grep -qF -- "--add-dir $T/home/quaqua/runs" "$T/calls" \
    || fail "--review $k: runs dir not a working directory: $(cat "$T/calls")"
  grep -qF -- "Edit(//$T/home/quaqua/runs/**)" "$T/calls" \
    || fail "--review $k: runs not writable (no Edit allow rule): $(cat "$T/calls")"
  grep -qF -- "Edit(//$T/.wt/$k-revread/**)" "$T/calls" \
    || fail "--review $k: worktree not read-only (no Edit deny rule): $(cat "$T/calls")"
  grep -qF -- '--model claude-opus-5-5' "$T/calls" || fail "--review $k: model changed: $(cat "$T/calls")"
  grep -qF -- "--effort $eff" "$T/calls" || fail "--review $k: effort not $eff: $(cat "$T/calls")"
  ! grep -qF -- '--approve-for-me' "$T/calls" || fail "--review $k: codex approval flag leaked: $(cat "$T/calls")"
  ! grep -qF -- 'default_permissions' "$T/calls" || fail "--review $k: codex profile leaked: $(cat "$T/calls")"
  grep -qF "worker=$k-revread" "$T/out" || fail "--review $k: no success line"
  echo "ok: --review $k -> dontAsk + Edit() rules (runs writable, worktree denied), model/effort unchanged"
done

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
# pushurl is a LIST and a push goes to every entry, so each remote gets an
# empty entry first (clearing any inherited shared-config pushurl) and the
# blocked value is added after it.
spawn tieude-pblock tieude FAKE_REMOTES='origin upstream'
[[ $rc -eq 0 ]] || fail "pblock spawn: exit $rc, want 0: $(cat "$T/err")"
grep -q 'config extensions.worktreeConfig true' "$T/calls" \
  || fail "pblock: extensions.worktreeConfig not enabled: $(cat "$T/calls")"
[[ $(grep -c 'config --worktree --replace-all remote\.\(origin\|upstream\)\.pushurl' "$T/calls") -eq 2 ]] \
  || fail "pblock: inherited pushurl not cleared per remote: $(cat "$T/calls")"
[[ $(grep -c 'config --worktree --add remote\.\(origin\|upstream\)\.pushurl PUSH DISABLED' "$T/calls") -eq 2 ]] \
  || fail "pblock: pushurl not set per remote: $(cat "$T/calls")"
echo "ok: push block -> worktreeConfig + per-remote pushurl clear+block on spawn"

# ── fail closed: core.bare=true / core.worktree in the common config ────────
# Enabling extensions.worktreeConfig there drops the exception that keeps
# those keys applying to the main worktree only (git-worktree(1),
# CONFIGURATION FILE), breaking every worktree of the repo — refuse instead.
spawn tieude-bareguard tieude FAKE_BARE=1
[[ $rc -eq 1 ]] || fail "bare guard: exit $rc, want 1: $(cat "$T/err")"
grep -q 'core.bare=true or core.worktree' "$T/err" \
  || fail "bare guard: no clear message: $(cat "$T/err")"
! grep -q 'extensions.worktreeConfig true' "$T/calls" \
  || fail "bare guard: worktreeConfig enabled anyway: $(cat "$T/calls")"
! grep -qE 'pane split|agent start' "$T/calls" \
  || fail "bare guard: pane/agent created anyway: $(cat "$T/calls")"
echo "ok: core.bare=true in common config -> spawn refused before any pane"

spawn tieude-wtguard tieude FAKE_WORKTREE=1
[[ $rc -eq 1 ]] || fail "worktree guard: exit $rc, want 1: $(cat "$T/err")"
grep -q 'core.bare=true or core.worktree' "$T/err" \
  || fail "worktree guard: no clear message: $(cat "$T/err")"
! grep -q 'extensions.worktreeConfig true' "$T/calls" \
  || fail "worktree guard: worktreeConfig enabled anyway: $(cat "$T/calls")"
! grep -qE 'pane split|agent start' "$T/calls" \
  || fail "worktree guard: pane/agent created anyway: $(cat "$T/calls")"
echo "ok: core.worktree in common config -> spawn refused before any pane"

# zero remotes: no pushurl to set, spawn still succeeds
spawn tieude-noremote tieude FAKE_REMOTES=''
[[ $rc -eq 0 ]] || fail "zero remotes: exit $rc, want 0: $(cat "$T/err")"
! grep -q 'pushurl' "$T/calls" || fail "zero remotes: pushurl written: $(cat "$T/calls")"
echo "ok: zero remotes -> no pushurl written, spawn succeeds"

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

# ── real git: an inherited shared-config pushurl must NOT survive the block ───
# pushurl is a list and a push goes to every entry, so a repo that already
# sets remote.origin.pushurl (fork workflow) would otherwise still receive a
# worker's push even though the worktree's own entry says PUSH DISABLED.
# Use a real bare repo as the shared push target: if the block leaks, the ref
# actually lands there.
shared="$T/shared.git"; mkdir -p "$T/rgpushurl"
git init -q --bare "$shared"
git -C "$T/rgpushurl" init -q
git -C "$T/rgpushurl" -c user.email=t@t -c user.name=t commit -qm init --allow-empty
git -C "$T/rgpushurl" remote add origin git@example.com:o/r.git
git -C "$T/rgpushurl" config remote.origin.pushurl "$shared"
: > "$T/calls"
rc=0
CALLS="$T/calls" HOME="$T/home" PATH="$T/bin2:$PATH" QUAQUA_TEST=1 GRID_SKIP=1 \
  bash "$T/scripts/worker/spawn-worker.sh" sude-inherit sude "$T/rgpushurl" >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 0 ]] || fail "inherit spawn: exit $rc: $(cat "$T/err")"
wt_inh="$T/.wt/sude-inherit"
rc=0
git -C "$wt_inh" push origin HEAD >"$T/inherit.out" 2>&1 || rc=$?
[[ $rc -ne 0 ]] || fail "inherit: git push succeeded despite the block"
grep -q 'PUSH DISABLED' "$T/inherit.out" \
  || fail "inherit: push error is not a clear refusal: $(cat "$T/inherit.out")"
[[ -z $(git -C "$shared" for-each-ref --format='%(refname)') ]] \
  || fail "inherit: push landed in the inherited target: $(git -C "$shared" for-each-ref --format='%(refname)')"
[[ $(git -C "$T/rgpushurl" remote get-url --push origin) == "$shared" ]] \
  || fail "inherit: main checkout pushurl changed: $(git -C "$T/rgpushurl" remote get-url --push origin)"
echo "ok: real git -> inherited shared pushurl cleared, push refused, target empty"

# ── real git: the bare/worktree guard refuses instead of breaking the repo ───
# Enabling extensions.worktreeConfig with core.bare=true in the common config
# makes every worktree report "this operation must be run in a work tree".
# The spawn must refuse and leave the config untouched (fail closed).
for guard_case in bare worktree; do
  gr="$T/rgguard-$guard_case"; mkdir -p "$gr"
  git -C "$gr" init -q
  git -C "$gr" -c user.email=t@t -c user.name=t commit -qm init --allow-empty
  git -C "$gr" remote add origin git@example.com:o/r.git
  if [[ $guard_case == bare ]]; then git -C "$gr" config core.bare true
  else git -C "$gr" config core.worktree "$gr"; fi
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin2:$PATH" QUAQUA_TEST=1 GRID_SKIP=1 \
    bash "$T/scripts/worker/spawn-worker.sh" "sude-g$guard_case" sude "$gr" >"$T/out" 2>"$T/err" || rc=$?
  [[ $rc -eq 1 ]] || fail "guard $guard_case: exit $rc, want 1: $(cat "$T/err")"
  grep -q 'core.bare=true or core.worktree' "$T/err" \
    || fail "guard $guard_case: no clear message: $(cat "$T/err")"
  [[ -z $(git -C "$gr" config --get extensions.worktreeConfig) ]] \
    || fail "guard $guard_case: extensions.worktreeConfig was enabled anyway"
  ! grep -q 'agent start' "$T/calls" || fail "guard $guard_case: agent started"
  # the guard leaves the repo usable: the main checkout still works
  git -C "$gr" rev-parse --git-dir >/dev/null || fail "guard $guard_case: repo left broken"
done
echo "ok: real git -> core.bare/core.worktree refused, config untouched, repo usable"

echo "PASS"
