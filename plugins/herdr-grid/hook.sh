#!/usr/bin/env bash
# hook.sh — event-hook reconciler for herdr-grid plugin.
#
# Requirements:
#   R7: Moves of working panes are deferred until agent_status_changed.
#       idle, done, unknown = not working.
#   R8: Event-hook driven, serialized by a mkdir lock with pending counter,
#       dead-pid lock reclaim, bash-3.2 safe, no flock.
#   Shadow mode (default on): runs plan.jq on `herdr api snapshot` and logs ops.
#   When shadow mode is off: pipes non-deferred ops to apply.sh.
#   Closed panes are detected by snapshot diff against last snapshot.
#   Early exits on status-changed events that change nothing.
set -euo pipefail

JQ="/usr/bin/jq"
if [ ! -x "$JQ" ]; then
  JQ="$(command -v jq || echo "jq")"
fi

HERDR_BIN="${HERDR_BIN_PATH:-herdr}"
PLUGIN_DIR="$(cd "$(dirname "$0")" && pwd)"
PLAN_JQ="$PLUGIN_DIR/plan.jq"

# State directory for lock, pending counter, and snapshot cache
STATE_DIR="${HERDR_PLUGIN_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/herdr/plugins/herdr-grid}"
mkdir -p "$STATE_DIR" 2>/dev/null || true

LOCK_DIR="$STATE_DIR/lock"
PID_FILE="$LOCK_DIR/pid"
PENDING_FILE="$STATE_DIR/pending"
LAST_SNAPSHOT="$STATE_DIR/last_snapshot.json"
SHADOW_LOG="$STATE_DIR/shadow.log"

EVENT_NAME="${HERDR_PLUGIN_EVENT:-}"
EVENT_JSON="${HERDR_PLUGIN_EVENT_JSON:-}"

# Check for early exit on pane.agent_status_changed events that change nothing.
# idle, done, unknown = not working.
# If previous state was non-working and new state is non-working (or same),
# or the pane is not a ready worker pane, early exit without acquiring lock.
if [ "$EVENT_NAME" = "pane.agent_status_changed" ] && [ -n "$EVENT_JSON" ] && [ -f "$LAST_SNAPSHOT" ]; then
  event_pane_id=$(echo "$EVENT_JSON" | "$JQ" -r '.pane_id // empty' 2>/dev/null || true)
  new_status=$(echo "$EVENT_JSON" | "$JQ" -r '.agent_status // empty' 2>/dev/null || true)

  if [ -n "$event_pane_id" ] && [ -n "$new_status" ]; then
    pane_summary=$("$JQ" -n -r --arg pid "$event_pane_id" --slurpfile snap "$LAST_SNAPSHOT" '
      ($snap[0].result.snapshot // $snap[0].snapshot // $snap[0]) as $root |
      ($root.panes // [] | [ .[] | select(.pane_id? == $pid) ] | first) as $p |
      if $p == null then "missing"
      else
        "\($p.tokens?.grid_role // ""):\($p.tokens?.grid_ready // ""):\($p.agent_status // "unknown")"
      end
    ' 2>/dev/null || echo "missing")

    if [ "$pane_summary" = "missing" ]; then
      # Foreign/untracked pane
      exit 0
    fi

    p_role=$(echo "$pane_summary" | cut -d: -f1)
    p_ready=$(echo "$pane_summary" | cut -d: -f2)
    p_old_status=$(echo "$pane_summary" | cut -d: -f3)

    if [ "$p_role" != "worker" ]; then
      # Untagged or non-worker pane
      exit 0
    fi

    # Check if working status changed
    old_working="0"
    if [ "$p_old_status" = "working" ]; then
      old_working="1"
    fi

    new_working="0"
    if [ "$new_status" = "working" ]; then
      new_working="1"
    fi

    if [ "$old_working" = "$new_working" ]; then
      # Status changed between non-working states (idle/done/unknown) or unchanged
      exit 0
    fi
  fi
fi

# Bump pending counter
echo "$$" >> "$PENDING_FILE"

# Lock acquisition with dead-pid reclaim
acquire_lock() {
  local attempts=0
  local max_attempts=100
  while [ "$attempts" -lt "$max_attempts" ]; do
    if mkdir "$LOCK_DIR" 2>/dev/null; then
      echo "$$" > "$PID_FILE"
      return 0
    fi

    local holder_pid=""
    if [ -f "$PID_FILE" ]; then
      holder_pid=$(cat "$PID_FILE" 2>/dev/null || true)
    fi
    case "$holder_pid" in
      ''|*[!0-9]*) holder_pid="" ;;
    esac

    if [ -n "$holder_pid" ]; then
      if ! kill -0 "$holder_pid" 2>/dev/null; then
        # Dead pid detected: reclaim lock
        rm -f "$PID_FILE" 2>/dev/null || true
        rmdir "$LOCK_DIR" 2>/dev/null || true
        continue
      fi
    elif [ "$attempts" -gt 5 ]; then
      # Empty or invalid PID file holding lock dir for >250ms
      rm -f "$PID_FILE" 2>/dev/null || true
      rmdir "$LOCK_DIR" 2>/dev/null || true
      continue
    fi

    sleep 0.05
    attempts=$((attempts + 1))
  done
  return 1
}

if ! acquire_lock; then
  echo "[herdr-grid] could not acquire lock after timeout" >&2
  exit 1
fi

cleanup_lock() {
  rm -f "$PID_FILE" 2>/dev/null || true
  rmdir "$LOCK_DIR" 2>/dev/null || true
}
trap cleanup_lock INT TERM HUP EXIT

reconcile() {
  local snapshot
  snapshot=$("$HERDR_BIN" api snapshot 2>/dev/null || echo "{}")
  if [ -z "$snapshot" ] || [ "$snapshot" = "{}" ]; then
    return 0
  fi

  # Snapshot diff to detect closed panes (missed by pane.closed on tab/workspace close)
  if [ -f "$LAST_SNAPSHOT" ]; then
    local closed_panes
    closed_panes=$(echo "$snapshot" | "$JQ" -r --slurpfile last "$LAST_SNAPSHOT" '
      [ ($last[0].result.snapshot // $last[0].snapshot // $last[0]).panes[]? | select((.tokens?.grid_role // "") == "worker" and (.tokens?.grid_ready // "") == "1") | .pane_id ] as $old |
      [ (.result.snapshot // .snapshot // .).panes[]? | select((.tokens?.grid_role // "") == "worker" and (.tokens?.grid_ready // "") == "1") | .pane_id ] as $cur |
      ($old - $cur)[]
    ' 2>/dev/null || true)

    if [ -n "$closed_panes" ]; then
      local ts
      ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date +"%Y-%m-%d %H:%M:%S")
      for cp in $closed_panes; do
        echo "[$ts] closed pane detected via snapshot diff: $cp" >> "$SHADOW_LOG" 2>/dev/null || true
      done
    fi
  fi

  # Save snapshot to cache atomically
  echo "$snapshot" > "$LAST_SNAPSHOT.tmp"
  mv -f "$LAST_SNAPSHOT.tmp" "$LAST_SNAPSHOT"

  # Run layout plan
  local ops
  ops=$(echo "$snapshot" | "$JQ" -c -f "$PLAN_JQ" 2>/dev/null || echo "[]")

  # Determine shadow mode (default: 1)
  local shadow_mode=1
  if [ "${HERDR_GRID_SHADOW:-}" = "0" ] || [ "${HERDR_GRID_SHADOW:-}" = "false" ]; then
    shadow_mode=0
  elif [ -f "$STATE_DIR/shadow" ] && grep -qiE "false|0" "$STATE_DIR/shadow" 2>/dev/null; then
    shadow_mode=0
  fi

  if [ "$shadow_mode" -eq 1 ]; then
    # Shadow mode: log ops only
    local ts
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date +"%Y-%m-%d %H:%M:%S")
    echo "[$ts] shadow ops: $ops" >> "$SHADOW_LOG" 2>/dev/null || true
    echo "$ops"
  else
    # Non-shadow mode: pipe non-deferred ops to apply.sh
    local non_deferred
    non_deferred=$(echo "$ops" | "$JQ" -c '[.[] | select(.deferred != true)]' 2>/dev/null || echo "[]")
    if [ -x "$PLUGIN_DIR/apply.sh" ] && [ "$non_deferred" != "[]" ]; then
      echo "$non_deferred" | "$PLUGIN_DIR/apply.sh"
    fi
  fi
}

# Main loop handling pending counter
while true; do
  rm -f "$PENDING_FILE" 2>/dev/null || true
  reconcile

  # If more events arrived during reconcile, loop again
  if [ -f "$PENDING_FILE" ]; then
    continue
  fi

  break
done

# Optional test injection hook before unlocking
if [ -n "${HERDR_GRID_TEST_UNLOCK_HOOK:-}" ]; then
  eval "$HERDR_GRID_TEST_UNLOCK_HOOK"
fi

# Release lock cleanly
trap - INT TERM HUP EXIT
cleanup_lock

# Verify no event arrived during unlock window
if [ -f "$PENDING_FILE" ]; then
  if mkdir "$LOCK_DIR" 2>/dev/null; then
    echo "$$" > "$PID_FILE"
    trap cleanup_lock INT TERM HUP EXIT
    while [ -f "$PENDING_FILE" ]; do
      rm -f "$PENDING_FILE" 2>/dev/null || true
      reconcile
    done
    trap - INT TERM HUP EXIT
    cleanup_lock
  fi
fi
