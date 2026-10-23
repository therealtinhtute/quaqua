#!/usr/bin/env bash
# Sandbox test for cleanup-worker.sh's worktree removal and grid hook. No real
# herdr/runs touched: temp git repo and a fake herdr (the grid check runs
# in-process against its `api snapshot`).
# Usage: tests/test_cleanup_worker.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-cleanup-worker.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
root="$(cd "$here/.." && pwd)"
bin="${QUAQUA_BIN:-$root/target/debug/quaqua}"
[[ -x "$bin" ]] || (cd "$root" && cargo build --locked --quiet) || fail "cannot build quaqua"
export QUAQUA_BIN="$bin"

# ── sandbox: temp worker repo + fake herdr ──────────────────────────────────
mkdir -p "$T/quaqua" "$T/bin" "$T/home" "$T/runs"

cat > "$T/bin/herdr" <<'EOF'
#!/usr/bin/env bash
echo "herdr $*" >> "${EVENTS:-/dev/null}"
if [[ "$1" == agent && "$2" == get ]]; then
  if [[ "${HERDR_GONE:-}" == 1 || " ${HERDR_GONE:-} " == *" $3 "* ]]; then
    printf '{"error":{"code":"agent_not_found","message":"agent target %s not found"}}\n' "$3" >&2
    exit 1
  fi
  if [[ -n "${HERDR_ERR:-}" ]]; then
    printf '{"error":{"code":"internal_error","message":"backend down"}}\n' >&2
    exit 1
  fi
  echo '{"result":{"agent":{"pane_id":"pane-1"}}}'
  exit 0
fi
# liveness second check: agent_not_found counts only when the list omits it.
if [[ "$1" == agent && "$2" == list ]]; then
  echo '{"result":{"agents":[{"name":"cos"}]}}'
  exit 0
fi
# grid check: in-process reconcile reads one snapshot. GRID_FAIL simulates an
# unreachable herdr; GRID_SNAPSHOT serves a fixture; default = converged.
if [[ "$1" == api && "$2" == snapshot ]]; then
  if [[ -n "${GRID_FAIL:-}" ]]; then printf '{"error":{"message":"backend down"}}\n' >&2; exit 1; fi
  if [[ -n "${GRID_SNAPSHOT:-}" ]]; then cat "$GRID_SNAPSHOT"; else printf '{"snapshot":{"workspaces":[],"tabs":[],"panes":[],"layouts":[],"agents":[]}}\n'; fi
  exit 0
fi
if [[ "$1" == pane && "$2" == move ]]; then
  if [[ -n "${GRID_MOVE_FAIL:-}" ]]; then
    printf '{"result":{"move_result":{"changed":false,"reason":"same_tab"}}}\n'
  else
    printf '{"result":{"move_result":{"changed":true,"pane":{"pane_id":"%s","tab_id":"w1:t_workers"}}}}\n' "${3:-%1}"
  fi
  exit 0
fi
if [[ "$1" == pane && "$2" == rename ]]; then
  printf '{"result":{}}\n'
  exit 0
fi
exit 0
EOF
chmod +x "$T/bin/herdr"
# the in-process input grid check shells out to `herdr api snapshot` and scopes
# by $QUAQUA_OWNER: pin the stub and drop the leaked owner so a live herdr or a
# foreign owner can never change the outcome
export HERDR_BIN_PATH="$T/bin/herdr"
export HERDR_SOCKET_PATH="$T/herdr.sock"
export PATH="$T/bin:$PATH"
unset HERDR_STUB_SNAPSHOT QUAQUA_OWNER GRID_CHECK_TIMEOUT_SEC

# a pending move for group "coucou": only --tab coucou-workers/--tab coucou scopes it in
cat > "$T/coucou.json" <<'EOF'
{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"quaqua"}],
 "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","pane_count":1},
         {"tab_id":"w1:t2","workspace_id":"w1","label":"2","pane_count":1}],
 "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","agent_status":"idle",
           "tokens":{"grid_role":"orchestrator","grid_owner":"cos","grid_ready":"1"}},
          {"pane_id":"w1:p5","tab_id":"w1:t2","workspace_id":"w1","agent_status":"idle",
           "tokens":{"grid_role":"worker","grid_owner":"cos","grid_group":"coucou","grid_ready":"1"}}],
 "layouts":[],"agents":[]}}
EOF

git -C "$T/quaqua" init -q
git -C "$T/quaqua" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init

worktree() {  # worktree <repo> <name>; cleanup keeps the branch, so reuse it
  local repo=$1 name=$2 wt="$(dirname "$1")/.wt/$2"
  [[ -d "$wt" ]] && return 0
  if git -C "$repo" show-ref -q "refs/heads/quaqua/$name"; then
    git -C "$repo" worktree add -q "$wt" "quaqua/$name"
  else
    git -C "$repo" worktree add -q -b "quaqua/$name" "$wt"
  fi
}
run_cleanup() {  # run_cleanup <name> [VAR=val...]
  local name=$1; shift
  : > "$T/events"
  env PATH="$T/bin:$PATH" HOME="$T/home" QUAQUA_RUNS_DIR="$T/runs" HERDR_BIN_PATH="$T/bin/herdr" HERDR_SOCKET_PATH="$T/herdr.sock" EVENTS="$T/events" "$@" \
    "${QUAQUA_BIN:-quaqua}" worker cleanup "$name" "$T/quaqua"
}

# ── 1. cleanup removes the worktree; grid check runs after the pane close ───
worktree "$T/quaqua" zz-clean
run_cleanup zz-clean >/dev/null || fail "cleanup failed"
[[ ! -d "$T/.wt/zz-clean" ]] || fail "worktree not removed"
grep -q '^herdr api snapshot$' "$T/events" || fail "grid check: not called: $(cat "$T/events")"
[[ "$(tail -1 "$T/events")" == "herdr api snapshot" ]] || fail "grid check: not after the pane close: $(cat "$T/events")"
echo "ok: cleanup -> worktree removed, in-process grid check runs after the pane is closed"

# ── 2. GRID_TAB selects the project's 2-4 worker tab ────────────────────────
# the fixture's pending move belongs to group "coucou": --tab coucou-workers
# scopes it in (check fails), the derived quaqua-workers scopes it out.
worktree "$T/quaqua" zz-clean
err=$(run_cleanup zz-clean GRID_TAB=coucou-workers GRID_SNAPSHOT="$T/coucou.json" GRID_CHECK_TIMEOUT_SEC=0 2>&1 >/dev/null) && fail "cleanup should fail on grid check timeout"
[[ "$err" == *"grid check failed"* || "$err" == *"grid check timed out"* ]] || fail "GRID_TAB: pending coucou move not detected: $err"
worktree "$T/quaqua" zz-clean
err=$(run_cleanup zz-clean GRID_SNAPSHOT="$T/coucou.json" GRID_CHECK_TIMEOUT_SEC=0 2>&1 >/dev/null) || fail "cleanup failed with the derived tab: $err"
[[ "$err" != *"grid check failed"* ]] || fail "quaqua-workers must not scope in the coucou move: $err"
echo "ok: GRID_TAB -> the check is scoped to that tab; timeout exits non-zero"

# ── 3. GRID_SKIP=1 disables the grid hook only for pipeline.sh / tests ─────
worktree "$T/quaqua" zz-clean
run_cleanup zz-clean QUAQUA_TEST=1 GRID_SKIP=1 >/dev/null || fail "cleanup failed with GRID_SKIP"
grep -q '^herdr api snapshot' "$T/events" && fail "GRID_SKIP=1: grid checked: $(cat "$T/events")"
worktree "$T/quaqua" zz-clean
run_cleanup zz-clean PIPELINE_TASK=t GRID_SKIP=1 >/dev/null || fail "cleanup failed with PIPELINE_TASK"
grep -q '^herdr api snapshot' "$T/events" && fail "pipeline GRID_SKIP=1: grid checked: $(cat "$T/events")"
echo "ok: GRID_SKIP=1 under tests/pipeline -> grid not checked"
worktree "$T/quaqua" zz-clean
run_cleanup zz-clean GRID_SKIP=1 >/dev/null 2>"$T/err" || fail "cleanup failed with bare GRID_SKIP"
grep -q '^herdr api snapshot' "$T/events" || fail "bare GRID_SKIP=1: grid not checked: $(cat "$T/events")"
grep -q 'GRID_SKIP=1 ignored' "$T/err" || fail "bare GRID_SKIP=1: no warning"
echo "ok: bare GRID_SKIP=1 -> ignored with a warning, grid checked"

# ── 4. a failing grid check blocks cleanup with non-zero exit ───────────────
worktree "$T/quaqua" zz-clean
err=$(run_cleanup zz-clean GRID_FAIL=1 GRID_CHECK_TIMEOUT_SEC=0 2>&1 >/dev/null) && fail "expected non-zero exit when grid check fails"
[[ "$err" == *"grid repair failed"* || "$err" == *"grid check failed"* ]] || fail "grid failure: wrong error: $err"
[[ ! -d "$T/.wt/zz-clean" ]] || fail "grid failure: worktree not removed"
echo "ok: grid failure -> non-zero exit, cleanup fails"
# ── 5. cleanup drops the worker's runs metadata (.owner/.grid) ──────────────
mkdir -p "$T/runs"
worktree "$T/quaqua" zz-clean
echo cos > "$T/runs/zz-clean.owner"; echo quaqua-workers > "$T/runs/zz-clean.grid"
run_cleanup zz-clean QUAQUA_RUNS_DIR="$T/runs" >/dev/null || fail "cleanup failed with runs metadata"
[[ ! -e "$T/runs/zz-clean.owner" && ! -e "$T/runs/zz-clean.grid" ]] || fail "runs metadata left: $(ls "$T/runs")"
echo "ok: cleanup -> runs/<name>.owner and .grid removed"

# ── 6. agent already gone -> worktree removed, branch kept, no pane close ───
worktree "$T/quaqua" zz-gone
mkdir -p "$T/runs"
echo cos > "$T/runs/zz-gone.owner"; echo quaqua-workers > "$T/runs/zz-gone.grid"
run_cleanup zz-gone HERDR_GONE=1 QUAQUA_RUNS_DIR="$T/runs" >/dev/null || fail "cleanup failed when agent is gone"
[[ ! -d "$T/.wt/zz-gone" ]] || fail "gone agent: worktree not removed"
git -C "$T/quaqua" show-ref -q "refs/heads/quaqua/zz-gone" || fail "gone agent: branch deleted"
! grep -q 'pane close' "$T/events" || fail "gone agent: pane close called"
[[ ! -e "$T/runs/zz-gone.owner" && ! -e "$T/runs/zz-gone.grid" ]] || fail "gone agent: runs metadata left"
grep -q '^herdr api snapshot$' "$T/events" || fail "gone agent: grid check not called"
echo "ok: agent already gone -> worktree removed, branch kept, no pane close"

# ── 7. real failure exits non-zero ──────────────────────────────────────────
worktree "$T/quaqua" zz-dirty
echo "dirty" > "$T/.wt/zz-dirty/dirty.txt"
git -C "$T/.wt/zz-dirty" add dirty.txt
run_cleanup zz-dirty >/dev/null 2>&1 && fail "expected non-zero exit on dirty worktree removal failure"
[[ -d "$T/.wt/zz-dirty" ]] || fail "dirty worktree should not be removed"
git -C "$T/quaqua" worktree remove -f "$T/.wt/zz-dirty" 2>/dev/null || true
worktree "$T/quaqua" zz-herdr-err
run_cleanup zz-herdr-err HERDR_ERR=1 >/dev/null 2>&1 && fail "expected non-zero exit on herdr backend failure"
[[ -d "$T/.wt/zz-herdr-err" ]] || fail "worktree removed on unexpected herdr error"
git -C "$T/quaqua" worktree remove -f "$T/.wt/zz-herdr-err" 2>/dev/null || true
echo "ok: real failure (worktree remove or herdr error) -> non-zero exit"

echo "PASS"
