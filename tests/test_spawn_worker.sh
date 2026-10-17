#!/usr/bin/env bash
# Sandbox test for the spawn-worker.sh shim (-> `quaqua worker spawn`) name
# validation (models.md Naming: <kind>-<task>, ascii [a-z0-9-], <=20 chars),
# the optional --base <ref> worktree start point, the same-name retry that
# reuses a branch kept by cleanup, the --review read-only mode for the claude
# kinds (including the narrow claude Bash allows for `cargo test` and the
# atomic report `mv`), the claude workspace-trust dialog the spawn answers
# before sending a brief, and its best-effort grid check.
# git+herdr are stubbed on PATH (the in-process grid check drives the herdr
# stub's `api snapshot`). Bad names must exit 1 before either is called; good
# names proceed.
# The accented case runs under C and, when installed, a collating UTF-8 locale.
# No real worktree, home, or herdr touched.
# Usage: tests/test_spawn_worker.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
root="$(cd "$here/.." && pwd)"
bin="${QUAQUA_BIN:-$root/target/debug/quaqua}"
[[ -x "$bin" ]] || (cd "$root" && cargo build --locked --quiet) || fail "cannot build quaqua"
export QUAQUA_BIN="$bin"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-spawn-worker.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
unset HERDR_PANE_ID HERDR_STUB_SNAPSHOT

# ── sandbox: stub git/herdr that log every call, temp HOME, dummy repo ───────
mkdir -p "$T/bin" "$T/repo" "$T/home/quaqua/runs" "$T/scripts/worker"
cp "$src/worker/spawn-worker.sh" "$T/scripts/worker/spawn-worker.sh"
cp -R "$src/lib" "$T/scripts/"
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
# FAKE_BRANCH: branches that already exist (space-sep short names, e.g.
# "quaqua/sude-x"); show-ref --verify succeeds for those and fails otherwise,
# so a same-name retry can be told from a fresh spawn
case " $* " in
  *" show-ref "*)
    for b in ${FAKE_BRANCH:-}; do
      case " $* " in *" refs/heads/$b "*) exit 0 ;; esac
    done
    exit 1 ;;
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
if [ "${1:-} ${2:-}" = "pane move" ]; then
  if [ -n "${GRID_AFTER_MOVE_SNAPSHOT:-}" ] && [ -f "$GRID_AFTER_MOVE_SNAPSHOT" ]; then
    cp "$GRID_AFTER_MOVE_SNAPSHOT" "$GRID_SNAPSHOT"
  fi
  if [ -n "${GRID_MOVE_FAIL:-}" ]; then
    printf '{"result":{"move_result":{"changed":false,"reason":"same_tab"}}}\n'
  else
    printf '{"result":{"move_result":{"changed":true,"pane":{"pane_id":"%s","tab_id":"w1:t_workers"}}}}\n' "${3:-%1}"
  fi
  exit 0
fi
if [ "${1:-} ${2:-}" = "pane rename" ]; then
  printf '{"result":{}}\n'
  exit 0
fi
# the in-process grid check reads one snapshot: GRID_FAIL simulates an
# unreachable herdr, GRID_SNAPSHOT serves a fixture, default = converged
if [ "${1:-} ${2:-}" = "api snapshot" ]; then
  [ -n "${GRID_CALLS:-}" ] && echo "grid check" >> "$GRID_CALLS"
  if [ -n "${GRID_FAIL:-}" ]; then printf '{"error":{"message":"backend down"}}\n' >&2; exit 1; fi
  if [ -n "${GRID_SNAPSHOT:-}" ]; then cat "$GRID_SNAPSHOT"; else printf '{"snapshot":{"workspaces":[],"tabs":[],"panes":[],"layouts":[],"agents":[]}}\n'; fi
  exit 0
fi
# PANE_TEXT is what `pane read` shows (the trust-dialog clear check greps it)
[ "${1:-} ${2:-}" = "pane read" ] && { printf '%s\n' "${PANE_TEXT:-}"; exit 0; }
# CLAUDE_DIALOG = claude workspace-trust dialogs left to show (--match trust)
if [ "${1:-} ${2:-}" = "pane wait-output" ]; then
  m=""; prev=""
  for a in "$@"; do [ "$prev" = "--match" ] && m=$a; prev=$a; done
  case $m in
    trust) f=${CLAUDE_DIALOG:-/dev/null} ;;
    *) f=/dev/null ;;
  esac
  n=$(cat "$f" 2>/dev/null || echo 0); [ "${n:-0}" -gt 0 ] || exit 1
  echo $((n - 1)) > "$f"
fi
# START_FAIL=1 (any kind) or START_FAIL_KIND=<kind>: `agent start` reports the
# startup-blocked error a slow (or dialog-held) start hits — herdr prints it on
# stdout and exits non-zero. START_FAIL_KIND fails only that --kind, so a
# fallback to a different harness can be exercised.
if [ "${1:-} ${2:-}" = "agent start" ]; then
  fail_kind=0
  if [ -n "${START_FAIL_KIND:-}" ]; then
    case " $* " in *" --kind $START_FAIL_KIND "*) fail_kind=1 ;; esac
  elif [ -n "${START_FAIL:-}" ]; then
    fail_kind=1
  fi
  if [ "$fail_kind" = 1 ]; then
    echo '{"error":{"code":"agent_not_ready","message":"agent is blocked during startup and is not ready for prompts"}}'
    exit 1
  fi
fi
# WAIT_IDLE_FAIL=1: `agent wait ... --until idle` fails, so a claude start that
# herdr could not confirm is a hard failure (no fallback configured -> error)
if [ "${1:-} ${2:-}" = "agent wait" ] && [ -n "${WAIT_IDLE_FAIL:-}" ]; then exit 1; fi
# grid check: the in-process reconcile reads one snapshot. GRID_FAIL simulates
# an unreachable herdr; GRID_SNAPSHOT serves a fixture; default = converged.
if [ "${1:-} ${2:-}" = "api snapshot" ]; then
  if [ -n "${GRID_FAIL:-}" ]; then printf '{"error":{"message":"backend down"}}\n' >&2; exit 1; fi
  if [ -n "${GRID_SNAPSHOT:-}" ]; then cat "$GRID_SNAPSHOT"; else printf '{"snapshot":{"workspaces":[],"tabs":[],"panes":[],"layouts":[],"agents":[]}}\n'; fi
  exit 0
fi
exit 0
STUB
chmod +x "$T/bin/git" "$T/bin/herdr"

# a pending move for group "coucou": only --tab coucou-workers scopes it in
cat > "$T/coucou.json" <<'EOF'
{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"repo"}],
 "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","pane_count":1},
         {"tab_id":"w1:t2","workspace_id":"w1","label":"2","pane_count":1}],
 "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","agent_status":"idle",
           "tokens":{"grid_role":"orchestrator","grid_owner":"cos","grid_ready":"1"}},
          {"pane_id":"w1:p5","tab_id":"w1:t2","workspace_id":"w1","agent_status":"idle",
           "tokens":{"grid_role":"worker","grid_owner":"cos","grid_group":"coucou","grid_ready":"1"}}],
 "layouts":[],"agents":[]}}
EOF

spawn() {  # spawn <name> <kind> [VAR=val...]; script opts via array $spawn_args.
  # Sets rc, $T/out, $T/err, $T/calls, $T/gridcalls
  local name=$1 kind=$2; shift 2
  : > "$T/calls"; : > "$T/gridcalls"
  rc=0
  CALLS="$T/calls" GRID_CALLS="$T/gridcalls" HOME="$T/home" PATH="$T/bin:$PATH" HERDR_BIN_PATH="$T/bin/herdr" LC_ALL="${SPAWN_LC_ALL:-C}" QUAQUA_OWNER=cos \
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

# ── tieude: opencode with deepseek-v4.1-flash at effort max in the env ──
# (team.toml) — the interactive TUI has no --model, so model+effort travel as
# `provider/model#variant` in OPENCODE_CONFIG_CONTENT; no omp/gemini
spawn "tieude-model" "tieude"
[[ $rc -eq 0 ]] || fail "tieude: exit $rc, want 0: $(cat "$T/err")"
grep -qF -- '--kind opencode' "$T/calls" \
  || fail "tieude: not an opencode start: $(cat "$T/calls")"
grep -qF -- 'OPENCODE_CONFIG_CONTENT={"model":"opencode-go/deepseek-v4.1-flash#max","instructions":' "$T/calls" \
  || fail "tieude: model+effort not in OPENCODE_CONFIG_CONTENT: $(cat "$T/calls")"
! grep -q -- '--kind omp' "$T/calls" || fail "tieude: still spawned as omp: $(cat "$T/calls")"
! grep -q 'gemini' "$T/calls" || fail "tieude: gemini flag leaked: $(cat "$T/calls")"
echo "ok: tieude -> --kind opencode, model opencode-go/deepseek-v4.1-flash#max"

# ── sude/detu: omp with antigravity gemini-3.8-flash high, worker rules appended ──
# (team.toml) — omp takes the model, the thinking level and the rules file on
# the command line (--append-system-prompt reads a file path), not in pane env
for k in sude detu; do
  spawn "$k-impl" "$k"
  [[ $rc -eq 0 ]] || fail "$k: exit $rc, want 0: $(cat "$T/err")"
  grep -qF -- '--kind omp' "$T/calls" || fail "$k: not an omp start: $(cat "$T/calls")"
  grep -qF -- '--model google-antigravity/gemini-3.8-flash' "$T/calls" \
    || fail "$k: model not google-antigravity/gemini-3.8-flash: $(cat "$T/calls")"
  grep -qF -- '--thinking high' "$T/calls" || fail "$k: thinking level not high: $(cat "$T/calls")"
  grep -qF -- '--auto-approve' "$T/calls" \
    || fail "$k: no --auto-approve (unattended spawn): $(cat "$T/calls")"
  grep -qF -- "--append-system-prompt $T/home/quaqua/runs/$k-impl.rules.md" "$T/calls" \
    || fail "$k: worker rules file not appended: $(cat "$T/calls")"
  ! grep -q -- '--kind opencode' "$T/calls" || fail "$k: still an opencode start: $(cat "$T/calls")"
  ! grep -q 'OPENCODE_CONFIG_CONTENT' "$T/calls" \
    || fail "$k: opencode pane env leaked into an omp start: $(cat "$T/calls")"
  echo "ok: $k -> --kind omp, google-antigravity/gemini-3.8-flash high, rules appended"
done

# ── fallback: a failed agent start takes one hop to the kind's fallback ───────
# sude (omp) falls back to tieude (opencode); the worker name keeps the sude prefix,
# only the harness/model/effort change, and the fallback is a WARN + run metadata.
spawn sude-fb sude START_FAIL_KIND=omp
[[ $rc -eq 0 ]] || fail "fallback: exit $rc, want 0: $(cat "$T/err")"
grep -qF -- '--kind omp' "$T/calls" || fail "fallback: primary omp not tried: $(cat "$T/calls")"
grep -qF -- '--kind opencode' "$T/calls" || fail "fallback: fallback opencode not tried: $(cat "$T/calls")"
[[ $(grep -c 'herdr agent start' "$T/calls") -eq 2 ]] \
  || fail "fallback: expected exactly 2 starts: $(cat "$T/calls")"
grep -qF 'WARN: fallback: sude→tieude (agent_not_ready)' "$T/err" \
  || fail "fallback: no WARN with reason: $(cat "$T/err")"
grep -qF '"fallback":{"from":"sude","to":"tieude","reason":"agent_not_ready"}' \
  "$T/home/quaqua/runs/sude-fb.srt.json" \
  || fail "fallback: not recorded in srt.json: $(cat "$T/home/quaqua/runs/sude-fb.srt.json")"
grep -qF '"running_kind":"tieude"' "$T/home/quaqua/runs/sude-fb.srt.json" \
  || fail "fallback: running_kind tieude not recorded in srt.json: $(cat "$T/home/quaqua/runs/sude-fb.srt.json")"
grep -qF '"harness":"opencode"' "$T/home/quaqua/runs/sude-fb.srt.json" \
  || fail "fallback: harness opencode not recorded in srt.json: $(cat "$T/home/quaqua/runs/sude-fb.srt.json")"
grep -qF '"kind":"sude"' "$T/home/quaqua/runs/sude-fb.srt.json" \
  || fail "fallback: original kind sude not preserved in srt.json: $(cat "$T/home/quaqua/runs/sude-fb.srt.json")"
grep -q "worker=sude-fb" "$T/out" || fail "fallback: no success line"
echo "ok: fallback -> omp failed, opencode (tieude) started, WARN + srt fallback recorded"

# one hop max: tieude (opencode) falls back to detu (omp); when both attempts
# fail -> hard error after exactly two attempts, never a third hop
spawn tieude-fb2 tieude START_FAIL=1
[[ $rc -ne 0 ]] || fail "one-hop: exit $rc, want non-zero: $(cat "$T/err")"
[[ $(grep -c 'herdr agent start' "$T/calls") -eq 2 ]] \
  || fail "one-hop: expected exactly 2 attempts: $(cat "$T/calls")"
grep -qF 'WARN: fallback: tieude→detu (agent_not_ready)' "$T/err" \
  || fail "one-hop: no fallback WARN: $(cat "$T/err")"
echo "ok: one-hop -> fallback start failure errors, no third attempt"

# a kind with no fallback fails closed on a start failure (ni = claude): herdr
# cannot confirm the start and idle-wait also fails -> error, nothing taken
spawn ni-nofb ni START_FAIL_KIND=claude WAIT_IDLE_FAIL=1
[[ $rc -ne 0 ]] || fail "no-fallback: exit $rc, want non-zero: $(cat "$T/err")"
! grep -q 'WARN: fallback' "$T/err" || fail "no-fallback: fell back anyway: $(cat "$T/err")"
[[ $(grep -c 'herdr agent start' "$T/calls") -eq 1 ]] \
  || fail "no-fallback: expected 1 attempt: $(cat "$T/calls")"
echo "ok: no fallback configured -> start failure errors, no fallback taken"

# ── grid check: a successful spawn arranges the new worker's pane ───────────
spawn tieude-gridhook tieude
[[ $rc -eq 0 ]] || fail "grid check: exit $rc, want 0: $(cat "$T/err")"
grep -q "grid check" "$T/gridcalls" || fail "grid check: not called: $(cat "$T/gridcalls")"
grep -q "worker=tieude-gridhook" "$T/out" || fail "grid check: no success line"
[[ "$(cat "$T/home/quaqua/runs/tieude-gridhook.grid")" == repo-workers ]] \
  || fail "grid check: runs/<name>.grid not <repo>-workers"
echo "ok: grid check -> called after a successful spawn, group recorded"

# GRID_TAB scopes the check: a pending move for group "coucou" only trips the
# tab that names it (--tab coucou-workers), not the derived repo-workers.
# A grid check timeout exits non-zero.
spawn tieude-gridtab tieude GRID_TAB=coucou-workers GRID_SNAPSHOT="$T/coucou.json" GRID_CHECK_TIMEOUT_SEC=0
[[ $rc -ne 0 ]] || fail "GRID_TAB: exit $rc, want non-zero on grid check timeout"
grep -qE 'grid check failed|grid check timed out' "$T/err" || fail "GRID_TAB: pending coucou move not detected: $(cat "$T/err")"
[[ "$(cat "$T/home/quaqua/runs/tieude-gridtab.grid")" == coucou-workers ]] \
  || fail "GRID_TAB: runs/<name>.grid not coucou-workers"
spawn tieude-gridtab2 tieude GRID_SNAPSHOT="$T/coucou.json" GRID_CHECK_TIMEOUT_SEC=0
[[ $rc -eq 0 ]] || fail "derived tab: exit $rc, want 0: $(cat "$T/err")"
! grep -qE 'grid check failed|grid check timed out' "$T/err" \
  || fail "repo-workers must not scope in the coucou move: $(cat "$T/err")"
echo "ok: GRID_TAB -> the check is scoped to that tab, group recorded; timeout exits non-zero"

# ── 2 unarranged workers + GRID_TAB -> both arranged into GRID_TAB with no manual reconcile
cat > "$T/unarranged2.json" <<'EOF'
{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"repo"}],
 "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","pane_count":3}],
 "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","agent_status":"idle",
           "tokens":{"grid_role":"orchestrator","grid_owner":"cos","grid_ready":"1"}},
          {"pane_id":"w1:p5","tab_id":"w1:t1","workspace_id":"w1","agent_status":"idle","label":"tieude-w1",
           "tokens":{"grid_role":"worker","grid_owner":"cos","grid_group":"myrepo","grid_ready":"1"}},
          {"pane_id":"w1:p6","tab_id":"w1:t1","workspace_id":"w1","agent_status":"idle","label":"tieude-w2",
           "tokens":{"grid_role":"worker","grid_owner":"cos","grid_group":"myrepo","grid_ready":"1"}}],
 "layouts":[],"agents":[{"name":"tieude-w1","pane_id":"w1:p5","agent_status":"idle"},
                        {"name":"tieude-w2","pane_id":"w1:p6","agent_status":"idle"}]}}
EOF
cat > "$T/converged2.json" <<'EOF'
{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"repo"}],
 "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","pane_count":1},
         {"tab_id":"w1:t_workers","workspace_id":"w1","label":"myrepo-workers","pane_count":2}],
 "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","agent_status":"idle",
           "tokens":{"grid_role":"orchestrator","grid_owner":"cos","grid_ready":"1"}},
          {"pane_id":"w1:p5","tab_id":"w1:t_workers","workspace_id":"w1","agent_status":"idle","label":"tieude-w1",
           "tokens":{"grid_role":"worker","grid_owner":"cos","grid_group":"myrepo","grid_tab":"myrepo-workers","grid_ready":"1"}},
          {"pane_id":"w1:p6","tab_id":"w1:t_workers","workspace_id":"w1","agent_status":"idle","label":"tieude-w2",
           "tokens":{"grid_role":"worker","grid_owner":"cos","grid_group":"myrepo","grid_tab":"myrepo-workers","grid_ready":"1"}}],
 "layouts":[],"agents":[{"name":"tieude-w1","pane_id":"w1:p5","agent_status":"idle"},
                        {"name":"tieude-w2","pane_id":"w1:p6","agent_status":"idle"}]}}
EOF
cp "$T/unarranged2.json" "$T/snap_live.json"
spawn tieude-twoarr tieude GRID_TAB=myrepo-workers GRID_SNAPSHOT="$T/snap_live.json" GRID_AFTER_MOVE_SNAPSHOT="$T/converged2.json"
[[ $rc -eq 0 ]] || fail "2 unarranged workers spawn: exit $rc, want 0: $(cat "$T/err")"
grep -qF 'herdr pane move w1:p5 --new-tab --label myrepo-workers --no-focus' "$T/calls" \
  || fail "worker 1 not moved to new tab: $(cat "$T/calls")"
grep -qF 'herdr pane move w1:p6 --tab w1:t_workers --split right --target-pane w1:p5 --ratio 0.5 --no-focus' "$T/calls" \
  || fail "worker 2 not split into workers tab: $(cat "$T/calls")"
echo "ok: 2 unarranged workers + GRID_TAB -> both arranged into GRID_TAB with no manual reconcile"

spawn tieude-gridskip tieude QUAQUA_TEST=1 GRID_SKIP=1
[[ $rc -eq 0 ]] || fail "GRID_SKIP: exit $rc, want 0: $(cat "$T/err")"
[[ ! -s "$T/gridcalls" ]] || fail "GRID_SKIP=1: grid checked: $(cat "$T/gridcalls")"
spawn tieude-gridskip tieude PIPELINE_TASK=t GRID_SKIP=1
[[ ! -s "$T/gridcalls" ]] || fail "pipeline GRID_SKIP=1: grid checked: $(cat "$T/gridcalls")"
echo "ok: GRID_SKIP=1 under tests/pipeline -> grid not checked"

spawn tieude-gridskip tieude GRID_SKIP=1
[[ $rc -eq 0 ]] || fail "bare GRID_SKIP: exit $rc, want 0: $(cat "$T/err")"
grep -q "grid check" "$T/gridcalls" || fail "bare GRID_SKIP=1: grid not checked: $(cat "$T/gridcalls")"
grep -q 'GRID_SKIP=1 ignored' "$T/err" || fail "bare GRID_SKIP=1: no warning"
echo "ok: bare GRID_SKIP=1 -> ignored with a warning, grid checked"

# ── opencode + project MCP config: no startup dialog wait needed ─────────────
spawn tieude-mcp tieude MKMCP=1
[[ $rc -eq 0 ]] || fail "mcp: exit $rc, want 0: $(cat "$T/err")"
! grep -q 'wait-output' "$T/calls" || fail "mcp: dialog wait ran"
echo "ok: opencode with project MCP config -> no dialog wait"
spawn tieude-nomcp tieude
! grep -q 'wait-output' "$T/calls" || fail "no .mcp.json: dialog wait ran"
echo "ok: opencode without project MCP config -> no dialog wait"

spawn tieude-gridfail tieude GRID_FAIL=1 GRID_CHECK_TIMEOUT_SEC=0
[[ $rc -ne 0 ]] || fail "grid failure: exit $rc, want non-zero: $(cat "$T/err")"
grep -qE "grid repair failed|grid check failed" "$T/err" || fail "grid failure: no error: $(cat "$T/err")"
echo "ok: grid failure -> non-zero exit, spawn fails"

# ── herdr-grid pane tokens (R5): role/group/owner on split, grid_ready=1 last ─
spawn tieude-tokens tieude
[[ $rc -eq 0 ]] || fail "tokens: exit $rc, want 0: $(cat "$T/err")"
grep -qF 'herdr pane report-metadata %1 --source herdr-grid --token grid_role=worker --token grid_group=repo --token grid_owner=cos --token grid_tab=repo-workers' "$T/calls" \
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
grep -qF 'herdr pane report-metadata %1 --source herdr-grid --token grid_role=worker --token grid_group=repo --token grid_owner=custom-lead --token grid_tab=repo-workers' "$T/calls" \
  || fail "custom owner: grid_owner not set to custom-lead: $(cat "$T/calls")"
echo "ok: pane tokens -> grid_owner reflects QUAQUA_OWNER"



# --review on an opencode/omp kind: refused before any git/herdr call
expect_reject_review() {  # expect_reject_review <name> <kind> <label>
  spawn_args=(--review)
  spawn "$1" "$2"
  unset spawn_args
  [[ $rc -eq 1 ]] || fail "$3: exit $rc, want 1"
  grep -q '^# Usage:' "$T/err" || fail "$3: no usage on stderr"
  grep -qF "error: --review is only supported for claude kinds (ni, suhuynh, truonglao)" "$T/err" \
    || fail "$3: no clear refusal: $(cat "$T/err")"
  [[ ! -s "$T/calls" ]] || fail "$3: git/herdr called before rejection: $(cat "$T/calls")"
  echo "ok: $3 -> exit 1, no git/herdr"
}
expect_reject_review tieude-revbad tieude "--review on tieude"
expect_reject_review sude-revbad sude "--review on sude"

# ── --review claude kinds: repo read-only, $HOME/quaqua/runs writable ────────
# claude has no filesystem sandbox, so the reviewer is held read-only by
# --permission-mode dontAsk ("Auto-denies every call that would
# otherwise prompt ... tools pre-approved via permissions.allow rules still
# run"), so the unattended reviewer never stalls on a permission dialog, plus
# Edit() path rules — the only file rules the file tools consult (a Write(...)
# path rule is accepted but never checked). Read-only Bash (git diff/log/show,
# cat, grep) stays prompt-free in every mode, so the reviewer keeps its
# evidence commands; mutating Bash is denied except two narrow allows the
# reviewer needs to finish a review: `cargo test` (its suite) and the
# runs/.<name>.tmp -> runs/<name>.md atomic report `mv`.
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
  grep -qF -- "Edit(//${T#/}/home/quaqua/runs/**)" "$T/calls" \
    || fail "--review $k: runs not writable (no Edit allow rule): $(cat "$T/calls")"
  grep -qF -- "Edit(//${T#/}/.wt/$k-revread/**)" "$T/calls" \
    || fail "--review $k: worktree not read-only (no Edit deny rule): $(cat "$T/calls")"
  grep -qF -- 'Bash(cargo test:*)' "$T/calls" \
    || fail "--review $k: cargo test not allowed: $(cat "$T/calls")"
  grep -qF -- 'Bash(mv *runs/.*.tmp *runs/*.md)' "$T/calls" \
    || fail "--review $k: atomic report mv not allowed: $(cat "$T/calls")"
  ! grep -qF -- '"Bash"' "$T/calls" || fail "--review $k: bare Bash allow leaked: $(cat "$T/calls")"
  ! grep -qF -- 'Bash(*)' "$T/calls" || fail "--review $k: Bash(*) allow leaked: $(cat "$T/calls")"
  ! grep -qF -- 'Bash(mv:*' "$T/calls" || fail "--review $k: unbounded mv allow leaked: $(cat "$T/calls")"
  grep -qF -- '--model claude-opus-5-5' "$T/calls" || fail "--review $k: model changed: $(cat "$T/calls")"
  grep -qF -- "--effort $eff" "$T/calls" || fail "--review $k: effort not $eff: $(cat "$T/calls")"
  ! grep -qF -- '--approve-for-me' "$T/calls" || fail "--review $k: approval flag leaked: $(cat "$T/calls")"
  ! grep -qF -- 'default_permissions' "$T/calls" || fail "--review $k: foreign profile leaked: $(cat "$T/calls")"
  grep -qF "worker=$k-revread" "$T/out" || fail "--review $k: no success line"
  echo "ok: --review $k -> dontAsk + Edit() rules (runs writable, worktree denied) + narrow Bash allows (cargo test, atomic mv), model/effort unchanged"
done

# the review Bash allows are review-only: a plain claude spawn keeps ambient settings
spawn ni-plain ni
[[ $rc -eq 0 ]] || fail "plain ni: exit $rc, want 0: $(cat "$T/err")"
! grep -qF -- '--permission-mode dontAsk' "$T/calls" || fail "plain ni: review mode leaked: $(cat "$T/calls")"
! grep -qF -- 'Bash(cargo test:*)' "$T/calls" || fail "plain ni: review cargo rule leaked: $(cat "$T/calls")"
! grep -qF -- 'Bash(mv *runs/.*.tmp *runs/*.md)' "$T/calls" || fail "plain ni: review mv rule leaked: $(cat "$T/calls")"
echo "ok: plain ni (no --review) -> ambient settings, no review mode/Bash rules"

# ── claude workspace-trust dialog: accept it, then let it clear ──────────────
# Claude Code's first session in a repo it has not been trusted in shows a
# workspace-trust dialog ("Accessing workspace: ... / Quick safety check: is
# this a project you created or one you trust?") whose "No, exit" button holds
# focus and which would swallow the brief. herdr's own dialog rules do not
# match that text (it can even report the agent ready with the dialog up), so
# the spawn answers it the way a person would: Down onto "Yes, I trust this
# folder", then Enter, and only then reports grid_ready=1.
echo 1 > "$T/claude-dialog"
spawn_args=(--review)
spawn suhuynh-trust suhuynh CLAUDE_DIALOG="$T/claude-dialog"
unset spawn_args
[[ $rc -eq 0 ]] || fail "trust dialog: exit $rc, want 0: $(cat "$T/err")"
grep -qF 'herdr pane wait-output %1 --match trust --timeout 15000' "$T/calls" \
  || fail "trust dialog: not waited for: $(cat "$T/calls")"
grep -qF 'herdr pane send-keys %1 down' "$T/calls" \
  || fail "trust dialog: focus not moved off 'No, exit': $(cat "$T/calls")"
grep -qF 'herdr pane send-keys %1 enter' "$T/calls" \
  || fail "trust dialog: not confirmed: $(cat "$T/calls")"
[[ $(ln_call 'wait-output') -lt $(ln_call 'send-keys %1 down') ]] \
  && [[ $(ln_call 'send-keys %1 down') -lt $(ln_call 'send-keys %1 enter') ]] \
  && [[ $(ln_call 'send-keys %1 enter') -lt $(ln_call 'grid_ready=1') ]] \
  || fail "trust dialog: bad order: $(cat "$T/calls")"
! grep -qE 'bypassPermissions|dangerously-skip|approve-for-me' "$T/calls" \
  || fail "trust dialog: review boundary weakened: $(cat "$T/calls")"
grep -qF -- '--permission-mode dontAsk' "$T/calls" \
  || fail "trust dialog: review permission mode lost: $(cat "$T/calls")"
grep -q "worker=suhuynh-trust" "$T/out" || fail "trust dialog: no success line"
echo "ok: claude trust dialog -> wait-output trust, Down+Enter before grid_ready, boundary untouched"

# an already-trusted repo (no dialog): nothing is sent, the spawn is unchanged
spawn suhuynh-notrust suhuynh
[[ $rc -eq 0 ]] || fail "no trust dialog: exit $rc, want 0: $(cat "$T/err")"
! grep -qF 'pane send-keys' "$T/calls" || fail "no trust dialog: keys sent anyway: $(cat "$T/calls")"
echo "ok: no trust dialog -> waited, nothing sent"

# a start herdr could not confirm (blocked during startup, or a slow hook) is
# not proof of failure: the spawn waits for idle instead of abandoning it
spawn suhuynh-blocked suhuynh START_FAIL=1
[[ $rc -eq 0 ]] || fail "blocked start: exit $rc, want 0: $(cat "$T/err")"
grep -qF 'herdr agent wait suhuynh-blocked --until idle --timeout 120000' "$T/calls" \
  || fail "blocked start: no idle wait: $(cat "$T/calls")"
[[ $(ln_call 'agent start') -lt $(ln_call 'agent wait suhuynh-blocked') ]] \
  && [[ $(ln_call 'agent wait suhuynh-blocked') -lt $(ln_call 'grid_ready=1') ]] \
  || fail "blocked start: bad order: $(cat "$T/calls")"
grep -q "worker=suhuynh-blocked" "$T/out" || fail "blocked start: no success line"
echo "ok: blocked claude start -> trust wait, idle wait, then grid_ready=1"

# a dialog that never clears is a loud failure, never a stalled worker
echo 1 > "$T/claude-dialog"
spawn suhuynh-stuck suhuynh CLAUDE_DIALOG="$T/claude-dialog" \
  PANE_TEXT='❯ No, exit
  Yes, I trust this folder'
[[ $rc -eq 1 ]] || fail "stuck dialog: exit $rc, want 1: $(cat "$T/err")"
grep -q 'workspace-trust dialog did not clear' "$T/err" \
  || fail "stuck dialog: no clear error: $(cat "$T/err")"
grep -qF 'herdr pane close %1' "$T/calls" || fail "stuck dialog: pane not closed: $(cat "$T/calls")"
! grep -q 'worker=suhuynh-stuck' "$T/out" || fail "stuck dialog: reported success anyway"
echo "ok: stuck trust dialog -> exit 1 with a clear error, pane closed"

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

# ── same-name retry: a branch kept by a cleanup is reused, not re-created ────
# cleanup-worker.sh removes the worker's worktree but keeps quaqua/<name> for
# the Lead to merge, so `worktree add -b` would die on
# "a branch named 'quaqua/<name>' already exists" — the retry must reuse it.
spawn tieude-branchfix tieude FAKE_BRANCH='quaqua/tieude-branchfix'
[[ $rc -eq 0 ]] || fail "same-name retry: exit $rc, want 0: $(cat "$T/err")"
grep -q 'branch quaqua/tieude-branchfix already exists' "$T/err" \
  || fail "same-name retry: no reuse note: $(cat "$T/err")"
grep -q "show-ref --verify --quiet refs/heads/quaqua/tieude-branchfix" "$T/calls" \
  || fail "same-name retry: kept branch not looked up: $(cat "$T/calls")"
grep -q "worktree add .*/tieude-branchfix quaqua/tieude-branchfix\$" "$T/calls" \
  || fail "same-name retry: kept branch not reused: $(cat "$T/calls")"
! grep -q "worktree add -b" "$T/calls" \
  || fail "same-name retry: -b on an existing branch: $(cat "$T/calls")"
echo "ok: same-name retry -> kept branch reused, no -b"

# an unrelated existing branch does not change a fresh spawn
spawn tieude-branchother tieude FAKE_BRANCH='quaqua/sude-other'
[[ $rc -eq 0 ]] || fail "unrelated branch: exit $rc, want 0: $(cat "$T/err")"
grep -q "worktree add -b quaqua/tieude-branchother .*/tieude-branchother\$" "$T/calls" \
  || fail "same-name retry: unrelated branch hijacked the add: $(cat "$T/calls")"
echo "ok: another branch exists -> fresh -b still used"

# --base on a retry cannot apply to the kept branch: say so, don't drop it
spawn_args=(--base quaqua/tieude-m12bt)
spawn tieude-branchbase tieude FAKE_BRANCH='quaqua/tieude-branchbase'
unset spawn_args
[[ $rc -eq 0 ]] || fail "same-name retry + --base: exit $rc, want 0: $(cat "$T/err")"
grep -q 'branch quaqua/tieude-branchbase already exists.*--base quaqua/tieude-m12bt ignored' "$T/err" \
  || fail "same-name retry + --base: --base not reported as ignored: $(cat "$T/err")"
! grep -q "worktree add -b" "$T/calls" \
  || fail "same-name retry + --base: -b on an existing branch: $(cat "$T/calls")"
echo "ok: same-name retry + --base -> branch reused, --base reported as ignored"

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

# a cleaned-up worker (worktree gone, branch kept) retried under the same name:
# real git proves the retry reuses the kept branch — the recreated worktree
# carries the commit the first attempt left behind — where `worktree add -b`
# would die with "a branch named 'quaqua/<name>' already exists"
git -C "$T/.wt/sude-rw1" -c user.email=t@t -c user.name=t commit -qm retry-marker --allow-empty
marker=$(git -C "$T/.wt/sude-rw1" rev-parse HEAD)
git -C "$T/rgrepo" worktree remove --force "$T/.wt/sude-rw1"
[[ -n $(git -C "$T/rgrepo" branch --list quaqua/sude-rw1) ]] || fail "rw1 retry: branch gone after cleanup"
[[ ! -d $T/.wt/sude-rw1 ]] || fail "rw1 retry: worktree not removed"
: > "$T/calls"
rc=0
CALLS="$T/calls" HOME="$T/home" PATH="$T/bin2:$PATH" QUAQUA_TEST=1 GRID_SKIP=1 \
  bash "$T/scripts/worker/spawn-worker.sh" sude-rw1 sude "$T/rgrepo" >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 0 ]] || fail "rw1 retry: exit $rc, want 0: $(cat "$T/err")"
grep -q 'branch quaqua/sude-rw1 already exists' "$T/err" \
  || fail "rw1 retry: no reuse note: $(cat "$T/err")"
[[ -d $T/.wt/sude-rw1 ]] || fail "rw1 retry: worktree not recreated"
[[ $(git -C "$T/.wt/sude-rw1" rev-parse HEAD) == "$marker" ]] \
  || fail "rw1 retry: kept branch not reused (HEAD $(git -C "$T/.wt/sude-rw1" rev-parse HEAD) != $marker)"
[[ $(git -C "$T/.wt/sude-rw1" rev-parse --abbrev-ref HEAD) == quaqua/sude-rw1 ]] \
  || fail "rw1 retry: branch changed: $(git -C "$T/.wt/sude-rw1" rev-parse --abbrev-ref HEAD)"
grep -q "worker=sude-rw1" "$T/out" || fail "rw1 retry: no success line"
echo "ok: real git -> retry after cleanup reuses the kept branch (marker commit kept), spawn succeeds"

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
