#!/usr/bin/env bash
# Unit test for plugins/herdr-grid/hook.sh (phase plugin-shadow, wave 2, T3).
# Tests the event-hook reconciler against a stub herdr.
#
# Requirements verified:
#   R7: Early exit on status-changed events that change nothing (idle/done/unknown = not working).
#   R8: Serialized by a mkdir lock with pending counter, dead-pid lock reclaim.
#       Event arriving during unlock is not lost.
#   Shadow mode (default on): logs ops without applying.
#   Non-shadow mode: pipes non-deferred ops to apply.sh.
#   Closed panes detected by snapshot diff (not pane.closed).
#
# Usage: tests/test_herdr_grid_hook.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$here/.." && pwd)"
hook_src="$repo_root/plugins/herdr-grid/hook.sh"
plan_src="$repo_root/plugins/herdr-grid/plan.jq"
manifest_src="$repo_root/plugins/herdr-grid/herdr-plugin.toml"
fixdir="$here/fixtures/herdr-grid"

T=$(mktemp -d "${TMPDIR:-/tmp}/test-grid-hook.XXXXXX")
T=$(cd "$T" && pwd -P)
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# Setup sandbox
mkdir -p "$T/bin" "$T/state" "$T/plugin"
cp "$hook_src" "$T/plugin/hook.sh"
cp "$plan_src" "$T/plugin/plan.jq"
cp "$manifest_src" "$T/plugin/herdr-plugin.toml"
chmod +x "$T/plugin/hook.sh"

CALLS_FILE="$T/calls.txt"
CURRENT_SNAPSHOT="$T/snapshot.json"
echo 0 > "$CALLS_FILE"
cp "$fixdir/1-worker.json" "$CURRENT_SNAPSHOT"

# Stub herdr
cat > "$T/bin/herdr" << 'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = "api" ] && [ "${2:-}" = "snapshot" ]; then
  calls=$(cat "$HERDR_CALLS_FILE" 2>/dev/null || echo 0)
  echo $((calls + 1)) > "$HERDR_CALLS_FILE"
  cat "$HERDR_SNAPSHOT_FILE"
  exit 0
fi
echo "stub herdr received unhandled args: $*" >&2
exit 1
EOF
chmod +x "$T/bin/herdr"

export HERDR_BIN_PATH="$T/bin/herdr"
export HERDR_CALLS_FILE="$CALLS_FILE"
export HERDR_SNAPSHOT_FILE="$CURRENT_SNAPSHOT"
export HERDR_PLUGIN_STATE_DIR="$T/state"

# ── Test 1: Shadow mode logs ops without modifying layout ─────────────────────
echo "test 1: shadow mode logs ops to shadow.log"
echo 0 > "$CALLS_FILE"
cp "$fixdir/1-worker.json" "$CURRENT_SNAPSHOT"
export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p5"}'

out=$(bash "$T/plugin/hook.sh")
[[ -s "$T/state/shadow.log" ]] || fail "test 1: shadow.log was not created or empty"
grep -q "shadow ops:" "$T/state/shadow.log" || fail "test 1: shadow.log missing 'shadow ops:'"
grep -q '"op":"move"' "$T/state/shadow.log" || fail "test 1: shadow.log missing move op"
[[ -f "$T/state/last_snapshot.json" ]] || fail "test 1: last_snapshot.json not cached"
calls=$(cat "$CALLS_FILE")
[[ "$calls" -ge 1 ]] || fail "test 1: stub herdr api snapshot was not called"
echo "ok: shadow mode logged ops and cached snapshot"

# ── Test 2: Non-shadow mode pipes non-deferred ops to apply.sh ────────────────
echo "test 2: non-shadow mode pipes ops to apply.sh"
cat > "$T/plugin/apply.sh" << 'EOF'
#!/usr/bin/env bash
cat > "$APPLY_INPUT_FILE"
EOF
chmod +x "$T/plugin/apply.sh"

APPLY_INPUT="$T/apply_input.json"
export APPLY_INPUT_FILE="$APPLY_INPUT"
export HERDR_GRID_SHADOW="0"

bash "$T/plugin/hook.sh" >/dev/null
[[ -f "$APPLY_INPUT" ]] || fail "test 2: apply.sh was not called"
grep -q '"op":"move"' "$APPLY_INPUT" || fail "test 2: apply.sh input missing move op"
rm -f "$APPLY_INPUT" "$T/plugin/apply.sh"
unset HERDR_GRID_SHADOW
echo "ok: non-shadow mode piped non-deferred ops to apply.sh"

# ── Test 3: Early exit on status-changed events that change nothing ───────────
echo "test 3: early exit on status-changed events"
# State snapshot currently has w1:p5 (worker, idle) and w1:p1 (orchestrator, idle)
echo 0 > "$CALLS_FILE"

# 3a: Non-worker pane w1:p1 status changes -> early exit
export HERDR_PLUGIN_EVENT="pane.agent_status_changed"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p1","agent_status":"working"}'
bash "$T/plugin/hook.sh"
calls=$(cat "$CALLS_FILE")
[[ "$calls" -eq 0 ]] || fail "test 3a: expected 0 herdr calls for orchestrator status change, got $calls"
echo "ok: 3a untagged/orchestrator pane status change early-exits"

# 3b: Worker pane w1:p5 changes from idle to done (both non-working) -> early exit
export HERDR_PLUGIN_EVENT="pane.agent_status_changed"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p5","agent_status":"done"}'
bash "$T/plugin/hook.sh"
calls=$(cat "$CALLS_FILE")
[[ "$calls" -eq 0 ]] || fail "test 3b: expected 0 herdr calls for idle->done transition, got $calls"
echo "ok: 3b non-working to non-working transition early-exits"

# 3c: Worker pane w1:p5 changes from idle to working (working state flips) -> reconciles!
export HERDR_PLUGIN_EVENT="pane.agent_status_changed"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p5","agent_status":"working"}'
bash "$T/plugin/hook.sh" >/dev/null
calls=$(cat "$CALLS_FILE")
[[ "$calls" -gt 0 ]] || fail "test 3c: expected herdr calls for idle->working transition, got 0"
echo "ok: 3c working state flip reconciles"

# 3d: Worker pane w1:p5 changes from working to idle (flip to non-working) -> reconciles!
sed -i.bak 's/"agent_status": "idle"/"agent_status": "working"/g' "$CURRENT_SNAPSHOT"
rm -f "$CURRENT_SNAPSHOT.bak"
export HERDR_PLUGIN_EVENT="pane.agent_status_changed"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p5","agent_status":"working"}'
bash "$T/plugin/hook.sh" >/dev/null

echo 0 > "$CALLS_FILE"
sed -i.bak 's/"agent_status": "working"/"agent_status": "idle"/g' "$CURRENT_SNAPSHOT"
rm -f "$CURRENT_SNAPSHOT.bak"
export HERDR_PLUGIN_EVENT="pane.agent_status_changed"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p5","agent_status":"idle"}'
bash "$T/plugin/hook.sh" >/dev/null
calls=$(cat "$CALLS_FILE")
[[ "$calls" -gt 0 ]] || fail "test 3d: expected herdr calls for working->idle transition, got 0"
echo "ok: 3d working->idle transition reconciles"

# ── Test 4: Closed panes detected by snapshot diff ─────────────────────────────
echo "test 4: closed panes detected by snapshot diff"
# Step A: snapshot has 2 workers
cp "$fixdir/2-workers.json" "$CURRENT_SNAPSHOT"
export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p6"}'
bash "$T/plugin/hook.sh" >/dev/null

# Step B: snapshot now has 1 worker (w1:p6 closed without pane.closed event)
cp "$fixdir/1-worker.json" "$CURRENT_SNAPSHOT"
export HERDR_PLUGIN_EVENT="tab.closed"
export HERDR_PLUGIN_EVENT_JSON='{"tab_id":"w1:t2"}'
bash "$T/plugin/hook.sh" >/dev/null

grep -q "closed pane detected via snapshot diff: w1:p6" "$T/state/shadow.log" || \
  fail "test 4: shadow.log did not record diff-detected closed pane w1:p6"
echo "ok: closed pane detected via snapshot diff"

# ── Test 5: Killed lock holder is reclaimed ───────────────────────────────────
echo "test 5: killed lock holder is reclaimed"
# Create lock dir with dead PID
mkdir -p "$T/state/lock"
# Spawn a short-lived subshell and wait for it to guarantee PID is dead
( : ) &
dead_pid=$!
wait "$dead_pid"
echo "$dead_pid" > "$T/state/lock/pid"

export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p5"}'
bash "$T/plugin/hook.sh" >/dev/null

# Lock should be released and execution succeeded
[[ ! -d "$T/state/lock" ]] || fail "test 5: lock dir still exists after completion"
echo "ok: killed lock holder PID $dead_pid was successfully reclaimed"

# ── Test 6: An event arriving during unlock is not lost ───────────────────────
echo "test 6: an event arriving during unlock is not lost"
echo 0 > "$CALLS_FILE"
# Trigger second event during unlock via HERDR_GRID_TEST_UNLOCK_HOOK
TRIGGER_LOG="$T/event2.log"
BG_PID_FILE="$T/bg.pid"
rm -f "$TRIGGER_LOG" "$BG_PID_FILE"
export HERDR_GRID_TEST_UNLOCK_HOOK="
  (
    export HERDR_PLUGIN_EVENT='pane.created'
    export HERDR_PLUGIN_EVENT_JSON='{\"pane_id\":\"w1:p5\"}'
    unset HERDR_GRID_TEST_UNLOCK_HOOK
    bash '$T/plugin/hook.sh' > '$TRIGGER_LOG' 2>&1
  ) &
  echo \$! > '$BG_PID_FILE'
"
bash "$T/plugin/hook.sh" >/dev/null
unset HERDR_GRID_TEST_UNLOCK_HOOK

# Wait for background event to fully complete
if [ -f "$BG_PID_FILE" ]; then
  bg_pid=$(cat "$BG_PID_FILE")
  while kill -0 "$bg_pid" 2>/dev/null; do
    sleep 0.05
  done
fi

[[ -f "$TRIGGER_LOG" ]] || fail "test 6: second event was not triggered"
[[ ! -d "$T/state/lock" ]] || fail "test 6: lock dir left behind"
calls=$(cat "$CALLS_FILE")
[[ "$calls" -ge 2 ]] || fail "test 6: expected at least 2 snapshot calls, got $calls"
echo "ok: event arriving during unlock was safely handled"

# ── Test 7: Concurrent events do not lose wakeups or leave lock ───────────────
echo "test 7: concurrent events test"
for i in 1 2 3 4 5; do
  (
    export HERDR_PLUGIN_EVENT="pane.created"
    export HERDR_PLUGIN_EVENT_JSON="{\"pane_id\":\"w1:p$i\"}"
    bash "$T/plugin/hook.sh" >/dev/null 2>&1
  ) &
done
wait

[[ ! -d "$T/state/lock" ]] || fail "test 7: lock dir left behind after concurrent events"
echo "ok: concurrent events serialized without lock leaks"

# ── Test 8: Plugin manifest event validation ──────────────────────────────────
echo "test 8: plugin manifest event validation"
grep -q '^id = "herdr-grid"' "$T/plugin/herdr-plugin.toml" || fail "test 8: missing plugin id"
# Verify never layout.updated (causes event recursion loops)
if grep -q 'layout.updated' "$T/plugin/herdr-plugin.toml"; then
  fail "test 8: herdr-plugin.toml must never subscribe to layout.updated"
fi
# Verify all subscribed events are valid herdr plugin events
events_found=$(grep -E '^on = ' "$T/plugin/herdr-plugin.toml" | sed 's/on = "\(.*\)"/\1/' | sort -u)
for ev in $events_found; do
  case "$ev" in
    pane.created|pane.agent_detected|pane.agent_status_changed|pane.moved|tab.closed|workspace.closed) ;;
    *) fail "test 8: unexpected event '$ev' in herdr-plugin.toml" ;;
  esac
done
echo "ok: plugin manifest subscribes only to allowed events and never layout.updated"

# ── Test 9: Stale lock dir with empty PID file is reclaimed ────────────────────
echo "test 9: stale lock dir with empty PID is reclaimed"
mkdir -p "$T/state/lock"
: > "$T/state/lock/pid"
export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p5"}'
bash "$T/plugin/hook.sh" >/dev/null
[[ ! -d "$T/state/lock" ]] || fail "test 9: lock dir still exists"
echo "ok: stale lock dir with empty pid was successfully reclaimed"

echo "PASS"
