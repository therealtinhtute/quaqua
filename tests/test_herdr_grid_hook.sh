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
apply_src="$repo_root/plugins/herdr-grid/apply.sh"
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
cp "$apply_src" "$T/plugin/apply.sh"
cp "$plan_src" "$T/plugin/plan.jq"
cp "$manifest_src" "$T/plugin/herdr-plugin.toml"
chmod +x "$T/plugin/hook.sh" "$T/plugin/apply.sh"
CALLS_FILE="$T/calls.txt"
MOVES_LOG="$T/moves.log"
CURRENT_SNAPSHOT="$T/snapshot.json"
echo 0 > "$CALLS_FILE"
: > "$MOVES_LOG"
cp "$fixdir/1-worker.json" "$CURRENT_SNAPSHOT"
# Stub herdr
cat > "$T/bin/herdr" << 'EOF'
#!/usr/bin/env bash
set -euo pipefail

state="${HERDR_SNAPSHOT_FILE:-}"
calls_file="${HERDR_CALLS_FILE:-}"
log="${HERDR_MOVES_LOG:-}"

case "${1:-}" in
  api)
    case "${2:-}" in
      snapshot)
        calls=$(cat "$calls_file" 2>/dev/null || echo 0)
        echo $((calls + 1)) > "$calls_file"
        [[ -n "$state" && -f "$state" ]] || { echo '{"error":"no state"}' >&2; exit 1; }
        cat "$state"
        ;;
      *)
        echo "stub herdr: unhandled api subcommand ${2:-}" >&2; exit 2 ;;
    esac
    ;;
  pane)
    case "${2:-}" in
      move)
        [[ -n "$log" ]] && printf 'move %s\n' "${*:3}" >> "$log"
        pane="${3:-}"; shift 2 || true
        [[ -n "$pane" ]] && shift || true

        tab=""
        label=""
        target_pane=""
        split=""
        ratio=""
        while [[ $# -gt 0 ]]; do
          case "$1" in
            --new-tab) tab="new" ;;
            --label) label="$2"; shift ;;
            --tab) tab="$2"; shift ;;
            --target-pane) target_pane="$2"; shift ;;
            --split) split="$2"; shift ;;
            --ratio) ratio="$2"; shift ;;
            --no-focus) ;;
            *) echo "stub herdr: unknown flag $1" >&2; exit 2 ;;
          esac
          shift
        done

        newtab="$tab"
        if [[ "$tab" == "new" ]]; then
          newtab="w1:t90"
        fi

        newpane="$pane${HERDR_STUB_MOVE_SUFFIX:-}"
        moves_file="$state.moves"
        n=$(( $(cat "$moves_file" 2>/dev/null || echo 0) + 1 ))
        echo "$n" > "$moves_file"

        current_tab=$(/usr/bin/jq -r --arg p "$pane" '(.snapshot // .result.snapshot // .).panes[]? | select(.pane_id == $p) | .tab_id' "$state")
        changed=true
        reason=""
        if [[ "$tab" != "new" && "$tab" == "$current_tab" ]]; then
          changed=false
          reason="same_tab"
        fi
        if [[ -n "${HERDR_STUB_MOVE_FAIL_AT:-}" && "$n" -eq "${HERDR_STUB_MOVE_FAIL_AT}" ]]; then
          changed=false
          reason="${HERDR_STUB_MOVE_REASON:-failed}"
        fi

        if [[ "$changed" == "true" ]]; then
          /usr/bin/jq -c --arg p "$pane" --arg np "$newpane" --arg t "$newtab" --arg l "$label" \
            --arg tp "$target_pane" --arg sp "$split" --arg r "$ratio" '
              (.snapshot // .result.snapshot // .) as $root
              | ($root.panes[] | select(.pane_id == $p) | .workspace_id // "w1") as $w
              | (if .snapshot != null then .snapshot else . end) |= (
                  (if $l != "" and all(.tabs[]; .tab_id != $t)
                   then .tabs += [{tab_id:$t, label:$l, workspace_id:$w, number:90, focused:false, pane_count:1, agent_status:"idle"}]
                   else . end)
                  | (if all(.layouts[]; .tab_id != $t)
                     then .layouts += [{tab_id:$t, workspace_id:$w, zoomed:false, area:{x:0,y:0,width:200,height:50}, focused_pane_id:$np, panes:[{pane_id:$np, rect:{x:0,y:0,width:100,height:50}}], splits:[]}]
                     else
                       .layouts |= map(
                         if .tab_id == $t then
                           if $sp == "right" and $r == "0.4" then
                             .panes = [
                               {pane_id: $tp, rect: {x: 0, y: 0, width: 80, height: 50}},
                               {pane_id: $np, rect: {x: 80, y: 0, width: 120, height: 50}}
                             ]
                           elif $sp == "down" then
                             .panes = [
                               {pane_id: $tp, rect: {x: 0, y: 0, width: 100, height: 25}},
                               {pane_id: $np, rect: {x: 0, y: 25, width: 200, height: 25}}
                             ]
                           elif $sp == "right" and any(.panes[]; .pane_id == "w1:p7") then
                             .panes += [{pane_id: $np, rect: {x: 100, y: 0, width: 100, height: 25}}]
                           elif $sp == "right" then
                             .panes = [
                               {pane_id: $tp, rect: {x: 0, y: 0, width: 100, height: 50}},
                               {pane_id: $np, rect: {x: 100, y: 0, width: 100, height: 50}}
                             ]
                           else
                             .panes += [{pane_id: $np, rect: {x: 100, y: 0, width: 100, height: 50}}]
                           end
                         else . end
                       )
                     end)
                  | ($root.panes[]? | select(.pane_id == $p) | .tab_id) as $prev_t
                  | .agents |= map(if .pane_id == $p then .pane_id = $np | .tab_id = $t else . end)
                  | .panes |= map(if .pane_id == $p then .pane_id = $np | .tab_id = $t else . end)
                  | if ($prev_t != null and $prev_t != $t and ([.panes[] | select(.pane_id != $p and .tab_id == $prev_t)] | length == 0))
                    then .tabs |= map(select(.tab_id != $prev_t)) | .layouts |= map(select(.tab_id != $prev_t))
                    else . end
                )' \
            "$state" > "$state.next" && mv "$state.next" "$state"
        fi

        # Event storm simulation: trigger pane.moved in background if requested
        if [[ "${HERDR_STUB_TRIGGER_EVENTS:-0}" == "1" && "$changed" == "true" ]]; then
          (
            export HERDR_PLUGIN_EVENT="pane.moved"
            export HERDR_PLUGIN_EVENT_JSON="{\"pane_id\":\"$newpane\",\"previous_pane_id\":\"$pane\",\"tab_id\":\"$newtab\"}"
            bash "$HERDR_PLUGIN_HOOK_PATH" >/dev/null 2>&1
          ) &
        fi

        /usr/bin/jq -nc --arg p "$newpane" --arg t "$newtab" --arg prev "$pane" \
          --argjson ch "$changed" --arg r "$reason" \
          '{result:{type:"pane_move",move_result:{changed:$ch,reason:(if $r=="" then null else $r end),pane:{pane_id:$p,tab_id:$t},previous_pane_id:$prev}}}'
        ;;
      *)
        echo "stub herdr: unhandled pane subcommand ${2:-}" >&2; exit 2 ;;
    esac
    ;;
  *)
    echo "stub herdr received unhandled args: $*" >&2
    exit 1
    ;;
esac
EOF
chmod +x "$T/bin/herdr"

export HERDR_BIN_PATH="$T/bin/herdr"
export HERDR_CALLS_FILE="$CALLS_FILE"
export HERDR_MOVES_LOG="$MOVES_LOG"
export HERDR_SNAPSHOT_FILE="$CURRENT_SNAPSHOT"
export HERDR_PLUGIN_STATE_DIR="$T/state"
export HERDR_PLUGIN_HOOK_PATH="$T/plugin/hook.sh"

reset_stub() {
  local fixture=$1
  cp "$fixture" "$CURRENT_SNAPSHOT"
  rm -f "$CURRENT_SNAPSHOT.moves" "$CURRENT_SNAPSHOT.next"
  rm -f "$T/state/last_snapshot.json" "$T/state/pending"
  : > "$MOVES_LOG"
  echo 0 > "$CALLS_FILE"
  unset HERDR_STUB_MOVE_SUFFIX HERDR_STUB_MOVE_FAIL_AT HERDR_STUB_MOVE_REASON HERDR_STUB_TRIGGER_EVENTS || true
}

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
rm -f "$APPLY_INPUT"
cp "$apply_src" "$T/plugin/apply.sh"
chmod +x "$T/plugin/apply.sh"
unset HERDR_GRID_SHADOW APPLY_INPUT_FILE
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

# ── Test 10: Non-shadow live apply converges layout to [] idempotently ────────
echo "test 10: non-shadow live apply converges layout"
reset_stub "$fixdir/2-workers.json"
export HERDR_GRID_SHADOW="0"
export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p6"}'

bash "$T/plugin/hook.sh" >/dev/null

# Verify moves occurred
moves_count=$(wc -l < "$MOVES_LOG" | tr -d ' ')
[[ "$moves_count" -eq 2 ]] || fail "test 10: expected 2 moves executed, got $moves_count"

# Verify plan on updated snapshot is []
remaining_ops=$(/usr/bin/jq -c -f "$plan_src" "$CURRENT_SNAPSHOT")
[[ "$remaining_ops" == "[]" ]] || fail "test 10: expected layout to converge to [], got: $remaining_ops"

# Second run with hook.sh executes 0 additional moves
: > "$MOVES_LOG"
bash "$T/plugin/hook.sh" >/dev/null
second_moves=$(wc -l < "$MOVES_LOG" | tr -d ' ')
[[ "$second_moves" -eq 0 ]] || fail "test 10: expected 0 moves on second run, got $second_moves"

[[ ! -d "$T/state/lock" ]] || fail "test 10: lock dir left behind"
unset HERDR_GRID_SHADOW
echo "ok: non-shadow live apply converges layout to [] idempotently"

# ── Test 11: Partial-apply case (move failure stops execution) ────────────────
echo "test 11: partial-apply case stops on error"
reset_stub "$fixdir/3-workers.json"
export HERDR_GRID_SHADOW="0"
export HERDR_STUB_MOVE_FAIL_AT=2
export HERDR_STUB_MOVE_REASON="split_error"
export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p7"}'

err_out="$T/test11.err"
if bash "$T/plugin/hook.sh" 2>"$err_out"; then
  fail "test 11: expected hook.sh to fail on partial apply error"
fi

# Exactly 2 moves attempted: move 1 succeeded, move 2 failed; move 3 was never attempted
moves_count=$(wc -l < "$MOVES_LOG" | tr -d ' ')
[[ "$moves_count" -eq 2 ]] || fail "test 11: expected exactly 2 moves attempted, got $moves_count"

# Check error output
grep -q "split_error" "$err_out" || fail "test 11: missing split_error in stderr: $(cat "$err_out")"

# Verify first move was applied (w1:p5 in w1:t90)
p5_tab=$(/usr/bin/jq -r '(.snapshot // .result.snapshot // .).panes[] | select(.pane_id == "w1:p5") | .tab_id' "$CURRENT_SNAPSHOT")
[[ "$p5_tab" == "w1:t90" ]] || fail "test 11: expected w1:p5 in w1:t90 from move 1, got $p5_tab"

# Verify move 3 was never attempted (w1:p6 remains in original tab w1:t2)
p6_tab=$(/usr/bin/jq -r '(.snapshot // .result.snapshot // .).panes[] | select(.pane_id == "w1:p6") | .tab_id' "$CURRENT_SNAPSHOT")
[[ "$p6_tab" == "w1:t2" ]] || fail "test 11: expected w1:p6 still in w1:t2, got $p6_tab"
# Lock dir released cleanly
[[ ! -d "$T/state/lock" ]] || fail "test 11: lock dir left behind after partial apply failure"
unset HERDR_GRID_SHADOW HERDR_STUB_MOVE_FAIL_AT HERDR_STUB_MOVE_REASON
echo "ok: partial-apply stops execution, leaves subsequent panes unmoved, and releases lock"

# ── Test 12: Foreign / untagged pane in plan rejected ─────────────────────────
echo "test 12: untagged pane rejected without moves"
reset_stub "$fixdir/untagged-pane.json"
export HERDR_GRID_SHADOW="0"
err_out="$T/test12.err"
if echo '[{"op":"move","pane":"w1:p2","deferred":false,"target":{"kind":"new_tab","label":"workers"}}]' | bash "$T/plugin/apply.sh" 2>"$err_out"; then
  fail "test 12: expected apply.sh to reject untagged pane"
fi
grep -qi "candidate worker" "$err_out" || fail "test 12: missing candidate worker error: $(cat "$err_out")"
moves_count=$(wc -l < "$MOVES_LOG" | tr -d ' ')
[[ "$moves_count" -eq 0 ]] || fail "test 12: expected 0 moves for untagged pane, got $moves_count"
unset HERDR_GRID_SHADOW
echo "ok: foreign / untagged pane rejected without moves"

# ── Test 13: Event storm from own moves absorbed ──────────────────────────────
echo "test 13: event storm from own moves absorbed"
reset_stub "$fixdir/2-workers.json"
export HERDR_GRID_SHADOW="0"
export HERDR_STUB_TRIGGER_EVENTS="1"
export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p6"}'

bash "$T/plugin/hook.sh" >/dev/null
wait

moves_count=$(wc -l < "$MOVES_LOG" | tr -d ' ')
[[ "$moves_count" -eq 2 ]] || fail "test 13: expected exactly 2 moves, got $moves_count"

remaining_ops=$(/usr/bin/jq -c -f "$plan_src" "$CURRENT_SNAPSHOT")
[[ "$remaining_ops" == "[]" ]] || fail "test 13: layout did not converge to []: $remaining_ops"

[[ ! -d "$T/state/lock" ]] || fail "test 13: lock dir left behind"
unset HERDR_GRID_SHADOW HERDR_STUB_TRIGGER_EVENTS
echo "ok: event storm absorbed, no duplicate moves, layout converges"

# ── Test 14: Working panes deferred under HERDR_GRID_SHADOW=0 ─────────────────
echo "test 14: working panes deferred until agent_status_changed"
reset_stub "$fixdir/working-deferred.json"
export HERDR_GRID_SHADOW="0"
export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p5"}'

bash "$T/plugin/hook.sh" >/dev/null

# w1:p5 is working in working-deferred.json -> deferred: true
p5_tab=$(/usr/bin/jq -r '(.snapshot // .result.snapshot // .).panes[] | select(.pane_id == "w1:p5") | .tab_id' "$CURRENT_SNAPSHOT")
[[ "$p5_tab" == "w1:t2" ]] || fail "test 14: working pane w1:p5 should not have moved from w1:t2, got $p5_tab"

# Now flip w1:p5 agent_status from working to idle
sed -i.bak 's/"agent_status": "working"/"agent_status": "idle"/g' "$CURRENT_SNAPSHOT"
rm -f "$CURRENT_SNAPSHOT.bak"
: > "$MOVES_LOG"

export HERDR_PLUGIN_EVENT="pane.agent_status_changed"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p5","agent_status":"idle"}'
bash "$T/plugin/hook.sh" >/dev/null

moves_count=$(wc -l < "$MOVES_LOG" | tr -d ' ')
[[ "$moves_count" -ge 1 ]] || fail "test 14: expected moves after status changed to idle, got $moves_count"
unset HERDR_GRID_SHADOW
echo "ok: working panes deferred until agent_status_changed to idle"

# ── Test 15: Rollback via HERDR_GRID_SHADOW=1 ─────────────────────────────────
echo "test 15: rollback to shadow mode with HERDR_GRID_SHADOW=1"
reset_stub "$fixdir/2-workers.json"
export HERDR_GRID_SHADOW="1"
export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p6"}'

rm -f "$T/state/shadow.log"
bash "$T/plugin/hook.sh" >/dev/null

moves_count=$(wc -l < "$MOVES_LOG" | tr -d ' ')
[[ "$moves_count" -eq 0 ]] || fail "test 15: expected 0 moves in shadow mode, got $moves_count"
grep -q "shadow ops:" "$T/state/shadow.log" || fail "test 15: shadow.log missing 'shadow ops:'"
unset HERDR_GRID_SHADOW
echo "ok: HERDR_GRID_SHADOW=1 successfully rolls back to shadow mode"

# ── Test 16: Freshly spawned 1-worker in same tab moves to ratio 0.4 and grid-workers.sh --check passes ──
echo "test 16: 1-worker freshly spawned in same tab moves into place (ratio 0.4) and check passes"
cat > "$CURRENT_SNAPSHOT" << 'EOF'
{
  "snapshot": {
    "workspaces": [
      { "workspace_id": "w1", "label": "quaqua" }
    ],
    "tabs": [
      { "tab_id": "w1:t1", "workspace_id": "w1", "label": "quaqua", "pane_count": 2 }
    ],
    "panes": [
      {
        "pane_id": "w1:p1",
        "tab_id": "w1:t1",
        "workspace_id": "w1",
        "agent": "claude",
        "agent_status": "idle",
        "tokens": {
          "grid_role": "orchestrator",
          "grid_owner": "cos",
          "grid_ready": "1"
        }
      },
      {
        "pane_id": "w1:p5",
        "tab_id": "w1:t1",
        "workspace_id": "w1",
        "agent": "pi",
        "agent_status": "idle",
        "tokens": {
          "grid_role": "worker",
          "grid_owner": "cos",
          "grid_group": "quaqua",
          "grid_ready": "1"
        }
      }
    ],
    "layouts": [
      {
        "tab_id": "w1:t1",
        "workspace_id": "w1",
        "area": { "x": 0, "y": 0, "width": 200, "height": 50 },
        "panes": [
          { "pane_id": "w1:p1", "rect": { "x": 0, "y": 0, "width": 100, "height": 50 } },
          { "pane_id": "w1:p5", "rect": { "x": 100, "y": 0, "width": 100, "height": 50 } }
        ]
      }
    ],
    "agents": [
      { "name": "cos", "pane_id": "w1:p1", "agent_status": "idle" },
      { "name": "tieude-1", "pane_id": "w1:p5", "agent_status": "idle" }
    ]
  }
}
EOF

export HERDR_GRID_SHADOW="0"
export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p5"}'
: > "$MOVES_LOG"
rm -f "$CURRENT_SNAPSHOT.moves"

bash "$T/plugin/hook.sh" >/dev/null

# 2 moves: parked in scratch tab, then moved to w1:t1 beside w1:p1 ratio 0.4
moves_count=$(wc -l < "$MOVES_LOG" | tr -d ' ')
[[ "$moves_count" -eq 2 ]] || fail "test 16: expected 2 moves executed, got $moves_count"

# Layout must have converged to []
remaining_ops=$(/usr/bin/jq -c -f "$plan_src" "$CURRENT_SNAPSHOT")
[[ "$remaining_ops" == "[]" ]] || fail "test 16: expected layout to converge to [], got: $remaining_ops"

# Anchor rect is 80 (0.4), worker rect is 120 (0.6)
anchor_w=$(/usr/bin/jq -r '(.snapshot // .result.snapshot // .).layouts[] | select(.tab_id == "w1:t1") | .panes[] | select(.pane_id == "w1:p1") | .rect.width' "$CURRENT_SNAPSHOT")
worker_w=$(/usr/bin/jq -r '(.snapshot // .result.snapshot // .).layouts[] | select(.tab_id == "w1:t1") | .panes[] | select(.pane_id != "w1:p1") | .rect.width' "$CURRENT_SNAPSHOT")
[[ "$anchor_w" -eq 80 ]] || fail "test 16: expected anchor width 80 (0.4), got $anchor_w"
[[ "$worker_w" -eq 120 ]] || fail "test 16: expected worker width 120 (0.6), got $worker_w"

# grid-workers.sh --check returns 0
PATH="$T/bin:$PATH" HERDR_CALLS_FILE="$CALLS_FILE" HERDR_MOVES_LOG="$MOVES_LOG" HERDR_SNAPSHOT_FILE="$CURRENT_SNAPSHOT" \
  GRID_CHECK_TIMEOUT_SEC=5 bash "$repo_root/scripts/worker/grid-workers.sh" --check --tab quaqua >/dev/null \
  || fail "test 16: grid-workers.sh --check failed"

unset HERDR_GRID_SHADOW
echo "ok: 1-worker freshly spawned in same tab moves into place (ratio 0.4) and grid-workers.sh --check returns 0"

# ── Test 17: When apply.sh fails, its stderr reaches shadow.log ───────────────
echo "test 17: when apply.sh fails, its stderr reaches shadow.log"
reset_stub "$fixdir/3-workers.json"
export HERDR_GRID_SHADOW="0"
export HERDR_STUB_MOVE_FAIL_AT=2
export HERDR_STUB_MOVE_REASON="stderr_shadow_test_error"
export HERDR_PLUGIN_EVENT="pane.created"
export HERDR_PLUGIN_EVENT_JSON='{"pane_id":"w1:p7"}'

rm -f "$T/state/shadow.log"
err_out="$T/test17.err"
if bash "$T/plugin/hook.sh" 2>"$err_out"; then
  fail "test 17: expected hook.sh to fail on apply failure"
fi

[[ -f "$T/state/shadow.log" ]] || fail "test 17: shadow.log not created"
grep -q "apply error: apply.sh exited with failure" "$T/state/shadow.log" \
  || fail "test 17: shadow.log missing 'apply error: apply.sh exited with failure'"
grep -q "stderr_shadow_test_error" "$T/state/shadow.log" \
  || fail "test 17: shadow.log missing stderr message 'stderr_shadow_test_error'"
grep -q "stderr_shadow_test_error" "$err_out" \
  || fail "test 17: hook.sh stderr missing 'stderr_shadow_test_error'"

unset HERDR_GRID_SHADOW HERDR_STUB_MOVE_FAIL_AT HERDR_STUB_MOVE_REASON
echo "ok: when apply.sh fails, its stderr reaches shadow.log"
echo "PASS"
