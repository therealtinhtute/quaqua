#!/usr/bin/env bash
# apply.sh — execute plan.jq layout ops via herdr pane move
#
# Reads JSON ops on stdin (array or stream of objects):
#   {
#     "op": "move",
#     "pane": "<pane_id>",
#     "deferred": <boolean>,
#     "target": {
#       "kind": "new_tab" | "split",
#       "label": "<label>",
#       "tab": "<tab_id>" | "@workers",
#       "anchor": "<pane_id>",
#       "direction": "right" | "down",
#       "ratio": <float>
#     }
#   }
#
# Requirements:
#   - bash 3.2 safe (no associative arrays, no bash 4+ features)
#   - /usr/bin/jq only
#   - herdr only via $HERDR_BIN_PATH
#   - Never reads quaqua files
#   - Uses pane.move only, never layout.apply
#   - Remaps pane IDs from each pane.moved result (previous_pane_id -> new id)
#   - Verifies each op against a fresh snapshot before moving
#   - Skips deferred ops
#   - Handles closed pane, failed nth move, zoomed tab
#   - Never leaves foreign (untagged or other-owner) panes moved
set -euo pipefail

if [ -x "/usr/bin/jq" ]; then
  JQ="/usr/bin/jq"
elif command -v jq >/dev/null 2>&1; then
  JQ="$(command -v jq)"
else
  echo "apply.sh: error: jq not found" >&2
  exit 1
fi

HERDR="${HERDR_BIN_PATH:-herdr}"

# Read all input from stdin
input=$(cat)
if [ -z "$input" ] || [ "$input" = "[]" ] || [ "$input" = "null" ]; then
  exit 0
fi

# Normalize ops to an array of objects
ops=$(printf '%s\n' "$input" | "$JQ" -s 'if length == 1 and (.[0] | type == "array") then .[0] else . end')
count=$(printf '%s\n' "$ops" | "$JQ" 'length')
if [ -z "$count" ] || [ "$count" -eq 0 ] 2>/dev/null; then
  exit 0
fi

# In-memory mappings (JSON strings for bash 3.2 compatibility)
# pane_map: { "<old_id>": "<new_id>" }
pane_map='{}'
# group_tabs: { "<group_name>": "<tab_id>", "@latest": "<tab_id>" }
group_tabs='{}'
deferred_panes='{}'

for ((i = 0; i < count; i++)); do
  op=$(printf '%s\n' "$ops" | "$JQ" -c ".[$i]")

  # Check op name
  op_name=$(printf '%s\n' "$op" | "$JQ" -r '.op // empty')
  if [ "$op_name" != "move" ]; then
    printf 'apply.sh: error: unhandled op "%s"\n' "$op_name" >&2
    exit 1
  fi

  # Skip deferred ops
  deferred=$(printf '%s\n' "$op" | "$JQ" -r '.deferred // false')
  if [ "$deferred" = "true" ]; then
    deferred_panes=$(printf '%s\n' "$deferred_panes" | "$JQ" -c --arg p "$(printf '%s\n' "$op" | "$JQ" -r '.pane // empty')" '.[$p] = true')
    continue
  fi

  # Validate target kind
  kind=$(printf '%s\n' "$op" | "$JQ" -r '.target.kind // empty')
  if [ "$kind" != "new_tab" ] && [ "$kind" != "split" ]; then
    printf 'apply.sh: error: unhandled target kind "%s"\n' "$kind" >&2
    exit 1
  fi

  # Remap pane id
  orig_pane=$(printf '%s\n' "$op" | "$JQ" -r '.pane // empty')
  if [ -z "$orig_pane" ]; then
    printf 'apply.sh: error: missing pane in op\n' >&2
    exit 1
  fi
  pane=$(printf '%s\n' "$pane_map" | "$JQ" -r --arg p "$orig_pane" '.[$p] // $p')

  # Remap anchor if split
  orig_anchor=""
  anchor=""
  if [ "$kind" = "split" ]; then
    orig_anchor=$(printf '%s\n' "$op" | "$JQ" -r '.target.anchor // empty')
    if [ -z "$orig_anchor" ]; then
      printf 'apply.sh: error: missing anchor in split op\n' >&2
      exit 1
    fi
    anchor=$(printf '%s\n' "$pane_map" | "$JQ" -r --arg a "$orig_anchor" '.[$a] // $a')
    anchor_deferred=$(printf '%s\n' "$deferred_panes" | "$JQ" -r --arg a "$orig_anchor" '.[$a] // false')
    if [ "$anchor_deferred" = "true" ]; then
      deferred_panes=$(printf '%s\n' "$deferred_panes" | "$JQ" -c --arg p "$orig_pane" '.[$p] = true')
      continue
    fi
  fi

  # ---------------------------------------------------------------------------
  # Verify against fresh snapshot
  # ---------------------------------------------------------------------------
  if ! snapshot=$("$HERDR" api snapshot 2>&1) || [ -z "$snapshot" ]; then
    printf 'apply.sh: error: failed to fetch herdr api snapshot\n' >&2
    exit 1
  fi

  # Verify pane exists
  pane_obj=$(printf '%s\n' "$snapshot" | "$JQ" -c --arg p "$pane" '
    (.result.snapshot // .snapshot // .) as $root
    | ($root.panes // []) as $panes
    | [ $panes[] | select(.pane_id == $p) ] | first // empty
  ')
  if [ -z "$pane_obj" ]; then
    printf 'apply.sh: error: pane %s closed or not found in snapshot\n' "$pane" >&2
    exit 1
  fi

  # Verify candidate worker (tokens, ready, role=worker, owner, group)
  is_candidate=$(printf '%s\n' "$pane_obj" | "$JQ" -r '
    (.tokens? != null) and
    ((.tokens.grid_ready // "") == "1" or .tokens.grid_ready == 1) and
    ((.tokens.grid_role // "") == "worker") and
    ((.tokens.grid_owner // "") != "") and
    ((.tokens.grid_group // "") != "")
  ')
  if [ "$is_candidate" != "true" ]; then
    printf 'apply.sh: error: pane %s is not a candidate worker (tokens invalid or untagged)\n' "$pane" >&2
    exit 1
  fi

  worker_owner=$(printf '%s\n' "$pane_obj" | "$JQ" -r '.tokens.grid_owner')
  worker_group=$(printf '%s\n' "$pane_obj" | "$JQ" -r '.tokens.grid_group')
  current_tab=$(printf '%s\n' "$pane_obj" | "$JQ" -r '.tab_id // empty')

  # Check if current tab is zoomed
  current_tab_zoomed=$(printf '%s\n' "$snapshot" | "$JQ" -r --arg t "$current_tab" '
    (.result.snapshot // .snapshot // .) as $root
    | ($root.layouts // []) as $layouts
    | [ $layouts[] | select(.tab_id == $t) | .zoomed // false ] | first // false
  ')
  if [ "$current_tab_zoomed" = "true" ]; then
    printf 'apply.sh: error: pane %s is in zoomed tab %s (zoomed_tab)\n' "$pane" "$current_tab" >&2
    exit 1
  fi

  # Verification specific to split
  target_tab=""
  if [ "$kind" = "split" ]; then
    tab_spec=$(printf '%s\n' "$op" | "$JQ" -r '.target.tab // empty')
    if [ "$tab_spec" = "@workers" ]; then
      target_tab=$(printf '%s\n' "$group_tabs" | "$JQ" -r --arg g "$worker_group" '.[$g] // .["@latest"] // empty')
      if [ -z "$target_tab" ]; then
        target_tab=$(printf '%s\n' "$snapshot" | "$JQ" -r --arg g "$worker_group" '
          (.result.snapshot // .snapshot // .) as $root
          | [ ($root.tabs // [])[] | select((.label == $g) or (.label == "\($g)-workers") or (.label == "workers")) | .tab_id ] | first // empty
        ')
      fi
      if [ -z "$target_tab" ]; then
        anchor_deferred=$(printf '%s\n' "$deferred_panes" | "$JQ" -r --arg a "$orig_anchor" '.[$a] // false')
        if [ "$anchor_deferred" = "true" ] || [ "$(printf '%s\n' "$deferred_panes" | "$JQ" 'length')" -gt 0 ]; then
          deferred_panes=$(printf '%s\n' "$deferred_panes" | "$JQ" -c --arg p "$orig_pane" '.[$p] = true')
          continue
        fi
        printf 'apply.sh: error: cannot resolve @workers tab for group %s\n' "$worker_group" >&2
        exit 1
      fi
    elif [ -n "$tab_spec" ]; then
      target_tab="$tab_spec"
    else
      printf 'apply.sh: error: missing tab in split op\n' >&2
      exit 1
    fi

    # Check if target tab is zoomed
    target_tab_zoomed=$(printf '%s\n' "$snapshot" | "$JQ" -r --arg t "$target_tab" '
      (.result.snapshot // .snapshot // .) as $root
      | ($root.layouts // []) as $layouts
      | [ $layouts[] | select(.tab_id == $t) | .zoomed // false ] | first // false
    ')
    if [ "$target_tab_zoomed" = "true" ]; then
      printf 'apply.sh: error: target tab %s is zoomed (zoomed_tab)\n' "$target_tab" >&2
      exit 1
    fi

    # Verify anchor pane exists in snapshot
    anchor_obj=$(printf '%s\n' "$snapshot" | "$JQ" -c --arg a "$anchor" '
      (.result.snapshot // .snapshot // .) as $root
      | ($root.panes // []) as $panes
      | [ $panes[] | select(.pane_id == $a) ] | first // empty
    ')
    if [ -z "$anchor_obj" ]; then
      printf 'apply.sh: error: anchor pane %s closed or not found in snapshot\n' "$anchor" >&2
      exit 1
    fi

    # Verify anchor is in target tab
    anchor_tab=$(printf '%s\n' "$anchor_obj" | "$JQ" -r '.tab_id // empty')
    if [ "$anchor_tab" != "$target_tab" ]; then
      printf 'apply.sh: error: anchor %s is in tab %s, expected target tab %s\n' "$anchor" "$anchor_tab" "$target_tab" >&2
      exit 1
    fi

    # Verify anchor ownership (if worker, must have same owner)
    anchor_role=$(printf '%s\n' "$anchor_obj" | "$JQ" -r '.tokens.grid_role // empty')
    anchor_owner=$(printf '%s\n' "$anchor_obj" | "$JQ" -r '.tokens.grid_owner // empty')
    if [ "$anchor_role" = "worker" ] && [ -n "$anchor_owner" ] && [ "$anchor_owner" != "$worker_owner" ]; then
      printf 'apply.sh: error: anchor %s is worker of different owner (%s != %s)\n' "$anchor" "$anchor_owner" "$worker_owner" >&2
      exit 1
    fi
  fi

  # ---------------------------------------------------------------------------
  # Execute herdr pane move
  # ---------------------------------------------------------------------------
  move_output=""
  move_rc=0
  parked_pane=""
  if [ "$kind" = "new_tab" ]; then
    label=$(printf '%s\n' "$op" | "$JQ" -r '.target.label // empty')
    if [ -z "$label" ]; then
      label="$worker_group"
    fi
    move_output=$("$HERDR" pane move "$pane" --new-tab --label "$label" --no-focus 2>&1) || move_rc=$?
  elif [ "$kind" = "split" ]; then
    direction=$(printf '%s\n' "$op" | "$JQ" -r '.target.direction // "right"')
    ratio=$(printf '%s\n' "$op" | "$JQ" -r '.target.ratio // 0.5')
    if [ "$current_tab" = "$target_tab" ]; then
      # herdr ignores a move whose destination is the pane's current tab (reason: same_tab).
      # Park the pane in a temporary scratch tab first, then move it to the split target.
      scratch_label="grid-scratch-$worker_group"
      scratch_output=$("$HERDR" pane move "$pane" --new-tab --label "$scratch_label" --no-focus 2>&1) || move_rc=$?
      if [ $move_rc -ne 0 ]; then
        printf 'apply.sh: error: failed to park pane %s in scratch tab (exit %d): %s\n' "$pane" "$move_rc" "$scratch_output" >&2
        exit 1
      fi
      scratch_changed=$(printf '%s\n' "$scratch_output" | "$JQ" -r '(.result.move_result // .move_result // .).changed // false' 2>/dev/null || echo "false")
      if [ "$scratch_changed" != "true" ]; then
        scratch_reason=$(printf '%s\n' "$scratch_output" | "$JQ" -r '(.result.move_result // .move_result // .).reason // "unknown"' 2>/dev/null || echo "unknown")
        printf 'apply.sh: error: scratch move changed nothing for %s (changed=%s, reason=%s)\n' "$pane" "$scratch_changed" "$scratch_reason" >&2
        exit 1
      fi
      parked_pane=$(printf '%s\n' "$scratch_output" | "$JQ" -r '(.result.move_result // .move_result // .).pane.pane_id // empty' 2>/dev/null || true)
      if [ -z "$parked_pane" ]; then
        printf 'apply.sh: error: missing pane_id in scratch move result for %s\n' "$pane" >&2
        exit 1
      fi
      move_output=$("$HERDR" pane move "$parked_pane" --tab "$target_tab" --split "$direction" --target-pane "$anchor" --ratio "$ratio" --no-focus 2>&1) || move_rc=$?
    else
      move_output=$("$HERDR" pane move "$pane" --tab "$target_tab" --split "$direction" --target-pane "$anchor" --ratio "$ratio" --no-focus 2>&1) || move_rc=$?
    fi
  fi

  if [ $move_rc -ne 0 ]; then
    printf 'apply.sh: error: herdr pane move failed (exit %d): %s\n' "$move_rc" "$move_output" >&2
    exit 1
  fi

  # ---------------------------------------------------------------------------
  # Validate move result
  # ---------------------------------------------------------------------------
  changed=$(printf '%s\n' "$move_output" | "$JQ" -r '(.result.move_result // .move_result // .).changed // false' 2>/dev/null || echo "false")
  reason=$(printf '%s\n' "$move_output" | "$JQ" -r '(.result.move_result // .move_result // .).reason // "unknown"' 2>/dev/null || echo "unknown")
  if [ "$changed" != "true" ]; then
    printf 'apply.sh: error: pane.move changed nothing for %s (changed=%s, reason=%s)\n' "$pane" "$changed" "$reason" >&2
    exit 1
  fi

  new_pane_id=$(printf '%s\n' "$move_output" | "$JQ" -r '(.result.move_result // .move_result // .).pane.pane_id // empty' 2>/dev/null || true)
  new_tab_id=$(printf '%s\n' "$move_output" | "$JQ" -r '(.result.move_result // .move_result // .).pane.tab_id // empty' 2>/dev/null || true)
  prev_pane_id=$(printf '%s\n' "$move_output" | "$JQ" -r '(.result.move_result // .move_result // .).previous_pane_id // empty' 2>/dev/null || true)

  if [ -z "$new_pane_id" ]; then
    printf 'apply.sh: error: missing pane_id in move result for %s\n' "$pane" >&2
    exit 1
  fi

  # ---------------------------------------------------------------------------
  # Update ID remappings and group tab state
  # ---------------------------------------------------------------------------
  prev_id="${prev_pane_id:-$pane}"
  pane_map=$(printf '%s\n' "$pane_map" | "$JQ" -c \
    --arg prev "$prev_id" --arg orig "$orig_pane" --arg cur "$pane" --arg next "$new_pane_id" --arg parked "${parked_pane:-}" '
      with_entries(if .value == $prev or .value == $orig or .value == $cur or (.value != "" and .value == $parked) then .value = $next else . end)
      | .[$prev] = $next
      | .[$orig] = $next
      | .[$cur] = $next
      | (if $parked != "" then .[$parked] = $next else . end)
  ')

  if [ "$kind" = "new_tab" ] && [ -n "$new_tab_id" ]; then
    group_tabs=$(printf '%s\n' "$group_tabs" | "$JQ" -c --arg g "$worker_group" --arg t "$new_tab_id" '
      .[$g] = $t | .["@latest"] = $t
    ')
  fi
done

exit 0
