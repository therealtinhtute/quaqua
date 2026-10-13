#!/usr/bin/env bash
# Unit test for plugins/herdr-grid/plan.jq layout planner (phase plugin-shadow, wave 1, T1).
# Tests turn a herdr api snapshot JSON into an ordered list of layout ops.
#
# Requirements verified:
#   R5: Only candidate panes with grid_ready=1 and grid_role=worker are moved.
#       Untagged panes, orchestrator panes, and foreign agents are never touched.
#   R6: Layout formula per owner:
#         1 worker total -> split beside owner (anchor 2/5, worker 3/5, ratio 0.4).
#         otherwise      -> one tab per group:
#                           1 = full
#                           2 = left|right
#                           3 = A|B top + C below
#                           4 = 2x2
#                           >4 in a group is rejected (jq error).
#       Second run on an applied layout produces zero ops (idempotence).
#   R7: Moves of working panes have deferred: true (idle/done/unknown = not working).
#
# Schema contract:
#   {
#     op: "move",
#     pane: "<pane_id>",
#     deferred: <boolean>,
#     target: {
#       kind: "new_tab" | "split",
#       label: "<label>" (new_tab),
#       tab: "<tab_id>" | "@workers" (split),
#       anchor: "<pane_id>" (split),
#       direction: "right" | "down" (split),
#       ratio: <float> (split)
#     }
#   }
#
# Usage: tests/test_herdr_grid_plan.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
plan_jq="$(cd "$here/.." && pwd)/plugins/herdr-grid/plan.jq"
fixdir="$here/fixtures/herdr-grid"

fail() { echo "FAIL: $*" >&2; exit 1; }

[[ -f "$plan_jq" ]] || fail "missing plan.jq at $plan_jq"
[[ -d "$fixdir" ]] || fail "missing fixtures dir at $fixdir"

run_plan() {
  /usr/bin/jq -c -f "$plan_jq" "$1"
}

assert_no_legacy_fields() {
  local json=$1
  local bad
  bad=$(jq -r '[.[].target | keys[] | select(. == "type" or . == "mode" or . == "tab_id" or . == "split" or . == "target_pane" or . == "target_pane_id")] | unique | join(",")' <<<"$json")
  if [[ -n "$bad" ]]; then
    fail "forbidden legacy/redundant fields in target: $bad (in $json)"
  fi
}

# -----------------------------------------------------------------------------
# 1. 0 workers -> 0 ops
# -----------------------------------------------------------------------------
echo "test 1: 0 workers"
out=$(run_plan "$fixdir/0-workers.json")
[[ "$out" == "[]" ]] || fail "0 workers expected [], got: $out"
echo "ok: 0 workers -> []"

# -----------------------------------------------------------------------------
# 2. 1 worker -> 1 op (split right beside anchor, ratio 0.4)
# -----------------------------------------------------------------------------
echo "test 2: 1 worker"
out=$(run_plan "$fixdir/1-worker.json")
assert_no_legacy_fields "$out"
len=$(jq 'length' <<<"$out")
[[ "$len" -eq 1 ]] || fail "1 worker expected 1 op, got length $len: $out"
op=$(jq -r '.[0].op' <<<"$out")
pane=$(jq -r '.[0].pane' <<<"$out")
deferred=$(jq -r '.[0].deferred' <<<"$out")
kind=$(jq -r '.[0].target.kind' <<<"$out")
tab=$(jq -r '.[0].target.tab' <<<"$out")
direction=$(jq -r '.[0].target.direction' <<<"$out")
anchor=$(jq -r '.[0].target.anchor' <<<"$out")
ratio=$(jq -r '.[0].target.ratio' <<<"$out")

[[ "$op" == "move" ]] || fail "expected op=move, got: $op"
[[ "$pane" == "w1:p5" ]] || fail "expected pane=w1:p5, got: $pane"
[[ "$deferred" == "false" ]] || fail "expected deferred=false, got: $deferred"
[[ "$kind" == "split" ]] || fail "expected kind=split, got: $kind"
[[ "$tab" == "w1:t1" ]] || fail "expected tab=w1:t1, got: $tab"
[[ "$direction" == "right" ]] || fail "expected direction=right, got: $direction"
[[ "$anchor" == "w1:p1" ]] || fail "expected anchor=w1:p1, got: $anchor"
[[ "$ratio" == "0.4" ]] || fail "expected ratio=0.4 (2/5|3/5 split), got: $ratio"
echo "ok: 1 worker -> split right beside owner 2/5|3/5"

# -----------------------------------------------------------------------------
# 3. 2 workers -> 2 ops (left | right 50/50)
# -----------------------------------------------------------------------------
echo "test 3: 2 workers"
out=$(run_plan "$fixdir/2-workers.json")
assert_no_legacy_fields "$out"
len=$(jq 'length' <<<"$out")
[[ "$len" -eq 2 ]] || fail "2 workers expected 2 ops, got length $len: $out"
op0_pane=$(jq -r '.[0].pane' <<<"$out")
op0_kind=$(jq -r '.[0].target.kind' <<<"$out")
op0_label=$(jq -r '.[0].target.label' <<<"$out")
op1_pane=$(jq -r '.[1].pane' <<<"$out")
op1_kind=$(jq -r '.[1].target.kind' <<<"$out")
op1_tab=$(jq -r '.[1].target.tab' <<<"$out")
op1_anchor=$(jq -r '.[1].target.anchor' <<<"$out")
op1_dir=$(jq -r '.[1].target.direction' <<<"$out")
op1_ratio=$(jq -r '.[1].target.ratio' <<<"$out")

[[ "$op0_pane" == "w1:p5" && "$op0_kind" == "new_tab" && "$op0_label" == "quaqua-workers" ]] \
  || fail "op 0 mismatch: $out"
[[ "$op1_pane" == "w1:p6" && "$op1_kind" == "split" && "$op1_tab" == "@workers" && "$op1_anchor" == "w1:p5" && "$op1_dir" == "right" && "$op1_ratio" == "0.5" ]] \
  || fail "op 1 mismatch: $out"
echo "ok: 2 workers -> new_tab + split right 0.5"

# -----------------------------------------------------------------------------
# 4. 3 workers -> 3 ops (A|B top + C full width below; build order 0, 2, 1)
# -----------------------------------------------------------------------------
echo "test 4: 3 workers"
out=$(run_plan "$fixdir/3-workers.json")
assert_no_legacy_fields "$out"
len=$(jq 'length' <<<"$out")
[[ "$len" -eq 3 ]] || fail "3 workers expected 3 ops, got length $len: $out"
p0=$(jq -r '.[0].pane' <<<"$out")
k0=$(jq -r '.[0].target.kind' <<<"$out")
p1=$(jq -r '.[1].pane' <<<"$out")
k1=$(jq -r '.[1].target.kind' <<<"$out")
d1=$(jq -r '.[1].target.direction' <<<"$out")
a1=$(jq -r '.[1].target.anchor' <<<"$out")
p2=$(jq -r '.[2].pane' <<<"$out")
k2=$(jq -r '.[2].target.kind' <<<"$out")
d2=$(jq -r '.[2].target.direction' <<<"$out")
a2=$(jq -r '.[2].target.anchor' <<<"$out")

# p5 is new_tab; p7 is split down on p5; p6 is split right on p5
[[ "$p0" == "w1:p5" && "$k0" == "new_tab" ]] || fail "op 0 not p5 new_tab: $out"
[[ "$p1" == "w1:p7" && "$k1" == "split" && "$d1" == "down" && "$a1" == "w1:p5" ]] || fail "op 1 not p7 split down on p5: $out"
[[ "$p2" == "w1:p6" && "$k2" == "split" && "$d2" == "right" && "$a2" == "w1:p5" ]] || fail "op 2 not p6 split right on p5: $out"
echo "ok: 3 workers -> build order [0, 2, 1] (A|B top + C below)"

# -----------------------------------------------------------------------------
# 5. 4 workers -> 4 ops (2x2 grid; build order 0, 2, 1, 3)
# -----------------------------------------------------------------------------
echo "test 5: 4 workers"
out=$(run_plan "$fixdir/4-workers.json")
assert_no_legacy_fields "$out"
len=$(jq 'length' <<<"$out")
[[ "$len" -eq 4 ]] || fail "4 workers expected 4 ops, got length $len: $out"
p0=$(jq -r '.[0].pane' <<<"$out")
p1=$(jq -r '.[1].pane' <<<"$out")
p2=$(jq -r '.[2].pane' <<<"$out")
p3=$(jq -r '.[3].pane' <<<"$out")

[[ "$p0" == "w1:p5" && $(jq -r '.[0].target.kind' <<<"$out") == "new_tab" ]] || fail "op 0 not p5: $out"
[[ "$p1" == "w1:p7" && $(jq -r '.[1].target.kind' <<<"$out") == "split" && $(jq -r '.[1].target.direction' <<<"$out") == "down" && $(jq -r '.[1].target.anchor' <<<"$out") == "w1:p5" ]] || fail "op 1 not p7 down p5: $out"
[[ "$p2" == "w1:p6" && $(jq -r '.[2].target.kind' <<<"$out") == "split" && $(jq -r '.[2].target.direction' <<<"$out") == "right" && $(jq -r '.[2].target.anchor' <<<"$out") == "w1:p5" ]] || fail "op 2 not p6 right p5: $out"
[[ "$p3" == "w1:p8" && $(jq -r '.[3].target.kind' <<<"$out") == "split" && $(jq -r '.[3].target.direction' <<<"$out") == "right" && $(jq -r '.[3].target.anchor' <<<"$out") == "w1:p7" ]] || fail "op 3 not p8 right p7: $out"
echo "ok: 4 workers -> build order [0, 2, 1, 3] (2x2 grid)"

# -----------------------------------------------------------------------------
# 6. 5 workers -> rejected (>4 per group)
# -----------------------------------------------------------------------------
echo "test 6: 5 workers (>4 per group is rejected)"
err=""
if out=$(run_plan "$fixdir/5-workers.json" 2>&1); then
  fail "expected rejection (>4 workers in group), but succeeded with: $out"
else
  err="$out"
fi
[[ "$err" == *"too many workers in group"* ]] || fail "expected 'too many workers in group' error, got: $err"
echo "ok: 5 workers -> rejected with error"

# -----------------------------------------------------------------------------
# 7. two owners in the same group -> planned independently
# -----------------------------------------------------------------------------
echo "test 7: two owners in the same group"
out=$(run_plan "$fixdir/two-owners-same-group.json")
assert_no_legacy_fields "$out"
len=$(jq 'length' <<<"$out")
[[ "$len" -eq 3 ]] || fail "expected 3 ops total (1 for cos + 2 for lead), got: $out"
# cos worker w1:p5 -> split beside w1:p1
cos_op=$(jq -c '.[] | select(.pane == "w1:p5")' <<<"$out")
[[ $(jq -r '.target.kind' <<<"$cos_op") == "split" ]] || fail "cos worker not split: $cos_op"
[[ $(jq -r '.target.anchor' <<<"$cos_op") == "w1:p1" ]] || fail "cos worker not beside w1:p1: $cos_op"
[[ $(jq -r '.target.direction' <<<"$cos_op") == "right" ]] || fail "cos worker direction not right: $cos_op"
[[ $(jq -r '.target.ratio' <<<"$cos_op") == "0.4" ]] || fail "cos worker ratio not 0.4: $cos_op"

# lead workers w2:p6, w2:p7 -> in group tab
lead_p6=$(jq -c '.[] | select(.pane == "w2:p6")' <<<"$out")
lead_p7=$(jq -c '.[] | select(.pane == "w2:p7")' <<<"$out")
[[ $(jq -r '.target.kind' <<<"$lead_p6") == "new_tab" ]] || fail "lead worker 1 not new_tab: $lead_p6"
[[ $(jq -r '.target.kind' <<<"$lead_p7") == "split" ]] || fail "lead worker 2 not split: $lead_p7"
[[ $(jq -r '.target.anchor' <<<"$lead_p7") == "w2:p6" ]] || fail "lead worker 2 not beside w2:p6: $lead_p7"
echo "ok: two owners in the same group planned independently per owner"

# -----------------------------------------------------------------------------
# 8. two sessions / workspaces -> planned independently
# -----------------------------------------------------------------------------
echo "test 8: two sessions / workspaces"
out=$(run_plan "$fixdir/two-sessions.json")
assert_no_legacy_fields "$out"
len=$(jq 'length' <<<"$out")
[[ "$len" -eq 2 ]] || fail "expected 2 ops across sessions, got: $out"
p1=$(jq -c '.[] | select(.pane == "w1:p5")' <<<"$out")
p2=$(jq -c '.[] | select(.pane == "w2:p6")' <<<"$out")
[[ $(jq -r '.target.anchor' <<<"$p1") == "w1:p1" ]] || fail "session 1 worker anchor mismatch: $p1"
[[ $(jq -r '.target.anchor' <<<"$p2") == "w2:p1" ]] || fail "session 2 worker anchor mismatch: $p2"
echo "ok: two sessions / workspaces handled cleanly"

# -----------------------------------------------------------------------------
# 9. untagged pane -> untouched
# -----------------------------------------------------------------------------
echo "test 9: untagged pane"
out=$(run_plan "$fixdir/untagged-pane.json")
assert_no_legacy_fields "$out"
len=$(jq 'length' <<<"$out")
[[ "$len" -eq 1 ]] || fail "expected exactly 1 op (for w1:p5), got $len: $out"
target_pane=$(jq -r '.[0].pane' <<<"$out")
[[ "$target_pane" == "w1:p5" ]] || fail "expected only w1:p5 moved, got: $target_pane"
has_p2=$(jq '[.[] | select(.pane == "w1:p2")] | length' <<<"$out")
has_p3=$(jq '[.[] | select(.pane == "w1:p3")] | length' <<<"$out")
has_p4=$(jq '[.[] | select(.pane == "w1:p4")] | length' <<<"$out")
[[ "$has_p2" -eq 0 && "$has_p3" -eq 0 && "$has_p4" -eq 0 ]] || fail "untagged or non-ready panes included in ops!"
echo "ok: untagged and non-ready panes untouched"

# -----------------------------------------------------------------------------
# 10. working-pane deferral -> deferred: true for working, false otherwise
# -----------------------------------------------------------------------------
echo "test 10: working-pane deferral"
out=$(run_plan "$fixdir/working-deferred.json")
assert_no_legacy_fields "$out"
d_p5=$(jq -r '.[] | select(.pane == "w1:p5") | .deferred' <<<"$out")
d_p6=$(jq -r '.[] | select(.pane == "w1:p6") | .deferred' <<<"$out")
d_p7=$(jq -r '.[] | select(.pane == "w1:p7") | .deferred' <<<"$out")
d_p8=$(jq -r '.[] | select(.pane == "w1:p8") | .deferred' <<<"$out")

[[ "$d_p5" == "true" ]] || fail "working pane w1:p5 expected deferred=true, got: $d_p5"
[[ "$d_p6" == "false" ]] || fail "idle pane w1:p6 expected deferred=false, got: $d_p6"
[[ "$d_p7" == "false" ]] || fail "done pane w1:p7 expected deferred=false, got: $d_p7"
[[ "$d_p8" == "false" ]] || fail "unknown pane w1:p8 expected deferred=false, got: $d_p8"
echo "ok: working-pane deferral flag set correctly (idle/done/unknown = not working)"

# -----------------------------------------------------------------------------
# 11. idempotence: applying ops and re-planning gives zero ops
# -----------------------------------------------------------------------------
echo "test 11: idempotence"

# Helper: simulate applying 1-worker beside owner
applied_1=$(jq '
  .snapshot.panes |= map(if .pane_id == "w1:p5" then .tab_id = "w1:t1" else . end)
  | .snapshot.layouts = [
      {
        "tab_id": "w1:t1",
        "workspace_id": "w1",
        "area": { "x": 0, "y": 0, "width": 200, "height": 50 },
        "panes": [
          { "pane_id": "w1:p1", "rect": { "x": 0, "y": 0, "width": 80, "height": 50 } },
          { "pane_id": "w1:p5", "rect": { "x": 80, "y": 0, "width": 120, "height": 50 } }
        ]
      }
    ]
' "$fixdir/1-worker.json")
replan_1=$(echo "$applied_1" | run_plan -)
[[ "$replan_1" == "[]" ]] || fail "idempotence failed on 1-worker, got: $replan_1"

# Helper: simulate applying 2-workers
applied_2=$(jq '
  .snapshot.tabs += [{ "tab_id": "w1:t_workers", "workspace_id": "w1", "label": "quaqua-workers", "pane_count": 2 }]
  | .snapshot.panes |= map(if (.pane_id == "w1:p5" or .pane_id == "w1:p6") then .tab_id = "w1:t_workers" else . end)
  | .snapshot.layouts += [
      {
        "tab_id": "w1:t_workers",
        "workspace_id": "w1",
        "area": { "x": 0, "y": 0, "width": 200, "height": 50 },
        "panes": [
          { "pane_id": "w1:p5", "rect": { "x": 0, "y": 0, "width": 100, "height": 50 } },
          { "pane_id": "w1:p6", "rect": { "x": 100, "y": 0, "width": 100, "height": 50 } }
        ]
      }
    ]
' "$fixdir/2-workers.json")
replan_2=$(echo "$applied_2" | run_plan -)
[[ "$replan_2" == "[]" ]] || fail "idempotence failed on 2-workers, got: $replan_2"

# Helper: simulate applying 3-workers
applied_3=$(jq '
  .snapshot.tabs += [{ "tab_id": "w1:t_workers", "workspace_id": "w1", "label": "quaqua-workers", "pane_count": 3 }]
  | .snapshot.panes |= map(if (.pane_id == "w1:p5" or .pane_id == "w1:p6" or .pane_id == "w1:p7") then .tab_id = "w1:t_workers" else . end)
  | .snapshot.layouts += [
      {
        "tab_id": "w1:t_workers",
        "workspace_id": "w1",
        "area": { "x": 0, "y": 0, "width": 200, "height": 50 },
        "panes": [
          { "pane_id": "w1:p5", "rect": { "x": 0, "y": 0, "width": 100, "height": 25 } },
          { "pane_id": "w1:p6", "rect": { "x": 100, "y": 0, "width": 100, "height": 25 } },
          { "pane_id": "w1:p7", "rect": { "x": 0, "y": 25, "width": 200, "height": 25 } }
        ]
      }
    ]
' "$fixdir/3-workers.json")
replan_3=$(echo "$applied_3" | run_plan -)
[[ "$replan_3" == "[]" ]] || fail "idempotence failed on 3-workers, got: $replan_3"

# Helper: simulate applying 4-workers
applied_4=$(jq '
  .snapshot.tabs += [{ "tab_id": "w1:t_workers", "workspace_id": "w1", "label": "quaqua-workers", "pane_count": 4 }]
  | .snapshot.panes |= map(if (.pane_id | test("^w1:p[5-8]$")) then .tab_id = "w1:t_workers" else . end)
  | .snapshot.layouts += [
      {
        "tab_id": "w1:t_workers",
        "workspace_id": "w1",
        "area": { "x": 0, "y": 0, "width": 200, "height": 50 },
        "panes": [
          { "pane_id": "w1:p5", "rect": { "x": 0, "y": 0, "width": 100, "height": 25 } },
          { "pane_id": "w1:p6", "rect": { "x": 100, "y": 0, "width": 100, "height": 25 } },
          { "pane_id": "w1:p7", "rect": { "x": 0, "y": 25, "width": 100, "height": 25 } },
          { "pane_id": "w1:p8", "rect": { "x": 100, "y": 25, "width": 100, "height": 25 } }
        ]
      }
    ]
' "$fixdir/4-workers.json")
replan_4=$(echo "$applied_4" | run_plan -)
[[ "$replan_4" == "[]" ]] || fail "idempotence failed on 4-workers, got: $replan_4"

# Helper: simulate applying two-owners-same-group
applied_two_owners=$(jq '
  .snapshot.tabs += [{ "tab_id": "w2:t_workers", "workspace_id": "w2", "label": "quaqua-workers", "pane_count": 2 }]
  | .snapshot.panes |= map(
      if .pane_id == "w1:p5" then .tab_id = "w1:t1"
      elif (.pane_id == "w2:p6" or .pane_id == "w2:p7") then .tab_id = "w2:t_workers"
      else . end
    )
  | .snapshot.layouts = [
      {
        "tab_id": "w1:t1",
        "workspace_id": "w1",
        "area": { "x": 0, "y": 0, "width": 200, "height": 50 },
        "panes": [
          { "pane_id": "w1:p1", "rect": { "x": 0, "y": 0, "width": 80, "height": 50 } },
          { "pane_id": "w1:p5", "rect": { "x": 80, "y": 0, "width": 120, "height": 50 } }
        ]
      },
      {
        "tab_id": "w2:t1",
        "workspace_id": "w2",
        "area": { "x": 0, "y": 0, "width": 200, "height": 50 },
        "panes": [
          { "pane_id": "w2:p1", "rect": { "x": 0, "y": 0, "width": 200, "height": 50 } }
        ]
      },
      {
        "tab_id": "w2:t_workers",
        "workspace_id": "w2",
        "area": { "x": 0, "y": 0, "width": 200, "height": 50 },
        "panes": [
          { "pane_id": "w2:p6", "rect": { "x": 0, "y": 0, "width": 100, "height": 50 } },
          { "pane_id": "w2:p7", "rect": { "x": 100, "y": 0, "width": 100, "height": 50 } }
        ]
      }
    ]
' "$fixdir/two-owners-same-group.json")
replan_two_owners=$(echo "$applied_two_owners" | run_plan -)
[[ "$replan_two_owners" == "[]" ]] || fail "idempotence failed on two owners, got: $replan_two_owners"

echo "ok: idempotence verified across all layout topologies (applying ops -> replan yields [])"

# -----------------------------------------------------------------------------
# 12. converged grid: 3 tagged workers already in a tab as A|B + C below -> []
# -----------------------------------------------------------------------------
echo "test 12: 3 tagged workers already in grid tab -> []"
out=$(run_plan "$fixdir/3-workers-grid.json")
[[ "$out" == "[]" ]] || fail "expected [] for 3-workers-grid, got: $out"

# Also test with tab labelled quaqua-workers
out_named=$(jq '.snapshot.tabs[1].label = "quaqua-workers"' "$fixdir/3-workers-grid.json" | run_plan -)
[[ "$out_named" == "[]" ]] || fail "expected [] for 3-workers-grid with quaqua-workers label, got: $out_named"

echo "ok: 3 tagged workers already in tab (workers and project label) -> []"

echo "ALL TESTS PASSED: test_herdr_grid_plan.sh"
