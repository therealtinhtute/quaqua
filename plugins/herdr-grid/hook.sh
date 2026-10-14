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
FAILED_OPS_FILE="$STATE_DIR/failed_ops.json"
FAILED_SNAP_FILE="$STATE_DIR/failed_snap.json"

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

  # If a worker pane is present but not yet marked grid_ready=1 (in-flight spawn),
  # poll briefly for grid_ready=1 so we don't miss the spawn layout.
  local pending_ready
  pending_ready=$(echo "$snapshot" | "$JQ" -r '
    [ (.result.snapshot // .snapshot // .).panes[]?
      | select((.tokens?.grid_role // "") == "worker" and (.tokens?.grid_ready == null or (.tokens?.grid_ready // "") == ""))
    ] | length
  ' 2>/dev/null || echo 0)

  if [ "$pending_ready" -gt 0 ]; then
    local poll_attempts=0
    while [ "$poll_attempts" -lt 15 ]; do
      sleep 0.2 2>/dev/null || sleep 1
      poll_attempts=$((poll_attempts + 1))
      local cur_snap
      cur_snap=$("$HERDR_BIN" api snapshot 2>/dev/null || echo "")
      if [ -n "$cur_snap" ] && [ "$cur_snap" != "{}" ]; then
        local still_pending
        still_pending=$(echo "$cur_snap" | "$JQ" -r '
          [ (.result.snapshot // .snapshot // .).panes[]?
            | select((.tokens?.grid_role // "") == "worker" and (.tokens?.grid_ready == null or (.tokens?.grid_ready // "") == ""))
          ] | length
        ' 2>/dev/null || echo 0)
        snapshot="$cur_snap"
        if [ "$still_pending" -eq 0 ]; then
          break
        fi
      fi
    done
  fi

  # Save snapshot to cache atomically
  echo "$snapshot" > "$LAST_SNAPSHOT.tmp"
  mv -f "$LAST_SNAPSHOT.tmp" "$LAST_SNAPSHOT"
  # Run layout plan
  local ops
  ops=$(echo "$snapshot" | "$JQ" -c -f "$PLAN_JQ" 2>/dev/null || echo "[]")

  if [ "$ops" = "[]" ] || [ -z "$ops" ]; then
    rm -f "$FAILED_OPS_FILE" "$FAILED_SNAP_FILE" 2>/dev/null || true
  fi

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
    non_deferred=$(echo "$ops" | "$JQ" -c '
      reduce .[] as $op (
        {deferred_panes: {}, workers_tab_deferred: false, non_deferred: []};
        if $op.deferred == true then
          .deferred_panes[$op.pane] = true
          | (if $op.target.kind == "new_tab" then .workers_tab_deferred = true else . end)
        elif (.deferred_panes[$op.target.anchor // ""] == true) or ($op.target.tab == "@workers" and .workers_tab_deferred) then
          .deferred_panes[$op.pane] = true
        else
          .non_deferred += [$op]
        end
      ) | .non_deferred
    ' 2>/dev/null || echo "[]")

    # Failure backoff: avoid infinite retry loops when an op fails on an unchanged layout
    local snap_sig
    snap_sig=$(echo "$snapshot" | "$JQ" -c '
      (.result.snapshot // .snapshot // .) as $root
      | ( [ ($root.tabs // [])[] | {key: .tab_id, value: .label} ] | from_entries ) as $tab_labels
      | [ ($root.panes // [])[]
          | select((.tokens?.grid_role // "") == "worker")
          | {
              pane_id,
              tab: (if ($tab_labels[.tab_id] // "" | startswith("grid-scratch-")) then "@scratch" else .tab_id end),
              status: (if .agent_status == "working" then "working" else "idle" end),
              tokens: {
                role: .tokens?.grid_role,
                owner: .tokens?.grid_owner,
                group: .tokens?.grid_group,
                ready: .tokens?.grid_ready
              }
            }
        ] | sort_by(.pane_id)
    ' 2>/dev/null || echo "")

    if [ -f "$FAILED_OPS_FILE" ] && [ -f "$FAILED_SNAP_FILE" ]; then
      local last_snap_sig
      last_snap_sig=$(cat "$FAILED_SNAP_FILE" 2>/dev/null || true)
      if [ -n "$snap_sig" ] && [ "$last_snap_sig" = "$snap_sig" ]; then
        # Snapshot state has not changed since last failure.
        # Filter out previously failed ops to prevent error storm / thrashing.
        local prev_count
        prev_count=$(echo "$non_deferred" | "$JQ" 'length' 2>/dev/null || echo 0)
        non_deferred=$(echo "$non_deferred" | "$JQ" -c --slurpfile failed "$FAILED_OPS_FILE" '
          ($failed[0] // []) as $f |
          map(select(. as $op | ($f | any(.pane == $op.pane and .target == $op.target)) | not))
        ' 2>/dev/null || echo "[]")
        local new_count
        new_count=$(echo "$non_deferred" | "$JQ" 'length' 2>/dev/null || echo 0)
        if [ "$prev_count" -gt "$new_count" ]; then
          local ts
          ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date +"%Y-%m-%d %H:%M:%S")
          echo "[$ts] backoff: skipping previously failed ops" >> "$SHADOW_LOG" 2>/dev/null || true
        fi
      else
        rm -f "$FAILED_OPS_FILE" "$FAILED_SNAP_FILE" 2>/dev/null || true
      fi
    fi

    if [ -x "$PLUGIN_DIR/apply.sh" ] && [ "$non_deferred" != "[]" ] && [ -n "$non_deferred" ]; then
      local ts
      ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date +"%Y-%m-%d %H:%M:%S")
      echo "[$ts] apply ops: $non_deferred" >> "$SHADOW_LOG" 2>/dev/null || true
      local apply_err="" apply_rc=0
      apply_err=$(echo "$non_deferred" | "$PLUGIN_DIR/apply.sh" 2>&1) || apply_rc=$?
      if [ $apply_rc -ne 0 ]; then
        echo "[$ts] apply error: apply.sh exited with failure" >> "$SHADOW_LOG" 2>/dev/null || true
        if [ -n "$apply_err" ]; then
          echo "$apply_err" >> "$SHADOW_LOG" 2>/dev/null || true
          echo "$apply_err" >&2
        fi
        # Record failed ops and snapshot state to back off until state changes
        echo "$non_deferred" > "$FAILED_OPS_FILE.tmp"
        mv -f "$FAILED_OPS_FILE.tmp" "$FAILED_OPS_FILE"
        local post_snap_sig="$snap_sig"
        local err_snapshot
        err_snapshot=$("$HERDR_BIN" api snapshot 2>/dev/null || echo "{}")
        if [ -n "$err_snapshot" ] && [ "$err_snapshot" != "{}" ]; then
          echo "$err_snapshot" > "$LAST_SNAPSHOT.tmp"
          mv -f "$LAST_SNAPSHOT.tmp" "$LAST_SNAPSHOT"
          post_snap_sig=$(echo "$err_snapshot" | "$JQ" -c '
            (.result.snapshot // .snapshot // .) as $root
            | ( [ ($root.tabs // [])[] | {key: .tab_id, value: .label} ] | from_entries ) as $tab_labels
            | [ ($root.panes // [])[]
                | select((.tokens?.grid_role // "") == "worker")
                | {
                    pane_id,
                    tab: (if ($tab_labels[.tab_id] // "" | startswith("grid-scratch-")) then "@scratch" else .tab_id end),
                    status: (if .agent_status == "working" then "working" else "idle" end),
                    tokens: {
                      role: .tokens?.grid_role,
                      owner: .tokens?.grid_owner,
                      group: .tokens?.grid_group,
                      ready: .tokens?.grid_ready
                    }
                  }
              ] | sort_by(.pane_id)
          ' 2>/dev/null || echo "$snap_sig")
        fi
        echo "$post_snap_sig" > "$FAILED_SNAP_FILE.tmp"
        mv -f "$FAILED_SNAP_FILE.tmp" "$FAILED_SNAP_FILE"
        return 1
      fi
      # Apply succeeded: clear failed ops record
      rm -f "$FAILED_OPS_FILE" "$FAILED_SNAP_FILE" 2>/dev/null || true
      # Refresh snapshot cache after apply so LAST_SNAPSHOT reflects applied moves
      local post_snapshot
      post_snapshot=$("$HERDR_BIN" api snapshot 2>/dev/null || echo "{}")
      if [ -n "$post_snapshot" ] && [ "$post_snapshot" != "{}" ]; then
        echo "$post_snapshot" > "$LAST_SNAPSHOT.tmp"
        mv -f "$LAST_SNAPSHOT.tmp" "$LAST_SNAPSHOT"
      fi
    fi
  fi
}

# Main loop handling pending counter
while true; do
  rm -f "$PENDING_FILE" 2>/dev/null || true
  if ! reconcile; then
    exit 1
  fi

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
