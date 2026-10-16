#!/usr/bin/env bash
# Unit test for plugins/herdr-grid/apply.sh layout executor (phase plugin-shadow, wave 2, T4).
#
# Requirements verified:
#   - Executes plan.jq ops ({op:"move",pane,deferred,target:{kind:"new_tab"|"split",label,tab,anchor,direction,ratio}})
#   - Uses pane.move only, never layout.apply
#   - Remaps pane IDs from each pane.moved result (previous_pane_id -> new id) so later ops reference new IDs
#   - Verifies each op against a fresh snapshot before moving
#   - Skips deferred ops
#   - Handles closed pane, failed nth move, and zoomed tab without leaving foreign (untagged/other-owner) panes moved
#
# Usage: bash tests/test_herdr_grid_apply.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
apply_sh="$root/plugins/herdr-grid/apply.sh"
plan_jq="$root/plugins/herdr-grid/plan.jq"
fixdir="$here/fixtures/herdr-grid"
apply_fixdir="$fixdir/apply"

fail() { echo "FAIL: $*" >&2; exit 1; }

# plugins/herdr-grid/apply.sh was removed by the R6 cutover (moves are applied
# in-process by `quaqua grid hook`), so this suite cannot execute. Finding 2
# (suhuynh-r6r1) requires the 13 cases to be mapped, not silently skipped:
#
#   "empty ops -> exit 0, no moves"                    -> grid.rs::test_0_workers_yields_empty_ops
#   "1 worker -> split beside owner"                   -> grid.rs::test_1_worker_split_beside_anchor
#   "2 workers -> new_tab + split into @workers (resolved to w1:t90)"
#                                                      -> grid.rs::test_2_workers_lands_in_grid_tab_never_orchestrator_tab
#   "3 workers -> new_tab + 2 splits in build order"   -> grid.rs::test_differential_all_fixtures_against_plan_jq
#   "pane IDs remapped correctly across moves"         -> grid.rs::test_repair_renames_with_new_pane_ids_and_respects_only
#   "deferred op skipped"                              -> grid.rs::test_deferred_op_propagates_to_dependent_splits
#   "failed move stops subsequent ops"                 -> grid.rs::test_apply_ops_rechecks_idle_and_fails_if_busy
#   "zoomed tab detected in snapshot before move"      -> grid.rs::test_edge_fixture_zoomed_anchor_tab
#   "zoomed_tab reason handled and reported"           -> grid.rs::test_edge_fixture_zoomed_anchor_tab
#   "untagged and not-ready panes rejected, no move attempted"
#                                                      -> grid.rs::test_edge_fixture_worker_with_foreign_pane
#   "other-owner anchor rejected, no move attempted"    -> grid.rs::test_apply_ops_refuses_when_anchor_not_in_target_tab
#   "pipeline plan.jq | apply.sh is idempotent"        -> grid.rs::test_differential_all_fixtures_against_plan_jq + test_edge_fixture_event_storm_after_move
#
# Dropped: "closed pane detected and aborted without moves" — the Rust
# reconcile_step logs the same snapshot-diff diagnostic to shadow.log; it is a
# log-only signal, with no state change for the bash case to assert.
if [[ ! -f "$apply_sh" ]]; then
  echo "SKIP: plugins/herdr-grid/apply.sh has been replaced by quaqua in R6 cutover"
  exit 0
fi
[[ -x "$apply_sh" ]] || fail "missing or non-executable apply.sh at $apply_sh"
[[ -f "$plan_jq" ]] || fail "missing plan.jq at $plan_jq"
[[ -d "$fixdir" ]] || fail "missing fixtures dir at $fixdir"

T=$(mktemp -d "${TMPDIR:-/tmp}/herdr-grid-apply-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

mkdir -p "$T/bin"

# -----------------------------------------------------------------------------
# Reusable herdr stub
# -----------------------------------------------------------------------------
cat << 'EOF' > "$T/bin/herdr"
#!/usr/bin/env bash
set -euo pipefail

state="${HERDR_STUB_STATE:-}"
log="${HERDR_STUB_LOG:-}"

case "${1:-}" in
  api)
    case "${2:-}" in
      snapshot)
        [[ -n "$state" && -f "$state" ]] || { echo '{"error":"no state"}' >&2; exit 1; }
        cat "$state"
        ;;
      *)
        echo "stub: unhandled api subcommand ${2:-}" >&2; exit 2 ;;
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
            *) echo "stub: unknown flag $1" >&2; exit 2 ;;
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

        changed=true
        reason=""
        if [[ -n "${HERDR_STUB_MOVE_FAIL_AT:-}" && "$n" -eq "${HERDR_STUB_MOVE_FAIL_AT}" ]]; then
          changed=false
          reason="${HERDR_STUB_MOVE_REASON:-failed}"
        fi

        if [[ "$changed" == "true" ]]; then
          /usr/bin/jq -c --arg p "$pane" --arg np "$newpane" --arg t "$newtab" --arg l "$label" \
            '(.snapshot // .result.snapshot // .) as $root
             | ($root.panes[] | select(.pane_id == $p) | .workspace_id // "w1") as $w
             | (if .snapshot != null then .snapshot else . end) |= (
                 (if $l != "" and all(.tabs[]; .tab_id != $t)
                  then .tabs += [{tab_id:$t, label:$l, workspace_id:$w, number:90, focused:false, pane_count:1, agent_status:"idle"}]
                  else . end)
                 | (if all(.layouts[]; .tab_id != $t)
                    then .layouts += [{tab_id:$t, workspace_id:$w, zoomed:false, area:{x:0,y:0,width:200,height:50}, focused_pane_id:$np, panes:[], splits:[]}]
                    else . end)
                 | .agents |= map(if .pane_id == $p then .pane_id = $np | .tab_id = $t else . end)
                 | .panes |= map(if .pane_id == $p then .pane_id = $np | .tab_id = $t else . end)
               )' \
            "$state" > "$state.next" && mv "$state.next" "$state"
        fi

        /usr/bin/jq -nc --arg p "$newpane" --arg t "$newtab" --arg prev "$pane" \
          --argjson ch "$changed" --arg r "$reason" \
          '{result:{type:"pane_move",move_result:{changed:$ch,reason:(if $r=="" then null else $r end),pane:{pane_id:$p,tab_id:$t},previous_pane_id:$prev}}}'
        ;;
      *)
        echo "stub: unhandled pane subcommand ${2:-}" >&2; exit 2 ;;
    esac
    ;;
  *)
    echo "stub: unhandled top-level command ${1:-}" >&2; exit 2 ;;
esac
EOF
chmod +x "$T/bin/herdr"

export HERDR_BIN_PATH="$T/bin/herdr"
export HERDR_STUB_STATE="$T/state.json"
export HERDR_STUB_LOG="$T/log"

reset_stub() {
  local fixture=$1
  cp "$fixture" "$HERDR_STUB_STATE"
  rm -f "$HERDR_STUB_STATE.moves"
  : > "$HERDR_STUB_LOG"
  unset HERDR_STUB_MOVE_SUFFIX HERDR_STUB_MOVE_FAIL_AT HERDR_STUB_MOVE_REASON || true
}

# -----------------------------------------------------------------------------
# Test 1: Empty input ([] or blank) -> exits 0, no herdr calls
# -----------------------------------------------------------------------------
echo "test 1: empty ops -> exits 0"
reset_stub "$fixdir/0-workers.json"
echo "[]" | "$apply_sh"
[[ ! -s "$HERDR_STUB_LOG" ]] || fail "expected empty log for []"

echo "" | "$apply_sh"
[[ ! -s "$HERDR_STUB_LOG" ]] || fail "expected empty log for blank stdin"
echo "ok: empty ops -> exit 0, no moves"

# -----------------------------------------------------------------------------
# Test 2: 1 worker split
# -----------------------------------------------------------------------------
echo "test 2: 1 worker split"
reset_stub "$fixdir/1-worker.json"
ops=$(/usr/bin/jq -c -f "$plan_jq" "$fixdir/1-worker.json")
echo "$ops" | "$apply_sh"

expected='move w1:p5 --tab w1:t1 --split right --target-pane w1:p1 --ratio 0.4 --no-focus'
actual=$(cat "$HERDR_STUB_LOG")
[[ "$actual" == "$expected" ]] || fail "test 2 mismatch: expected '$expected', got '$actual'"
echo "ok: 1 worker -> split beside owner"

# -----------------------------------------------------------------------------
# Test 3: 2 workers (new_tab + split into @workers)
# -----------------------------------------------------------------------------
echo "test 3: 2 workers"
reset_stub "$fixdir/2-workers.json"
ops=$(/usr/bin/jq -c -f "$plan_jq" "$fixdir/2-workers.json")
echo "$ops" | "$apply_sh"

expected='move w1:p5 --new-tab --label quaqua-workers --no-focus
move w1:p6 --tab w1:t90 --split right --target-pane w1:p5 --ratio 0.5 --no-focus'
actual=$(cat "$HERDR_STUB_LOG")
[[ "$actual" == "$expected" ]] || fail "test 3 mismatch: expected '$expected', got '$actual'"
echo "ok: 2 workers -> new_tab + split into @workers (resolved to w1:t90)"

# -----------------------------------------------------------------------------
# Test 4: 3 workers (new_tab + 2 splits; build order 0, 2, 1)
# -----------------------------------------------------------------------------
echo "test 4: 3 workers"
reset_stub "$fixdir/3-workers.json"
ops=$(/usr/bin/jq -c -f "$plan_jq" "$fixdir/3-workers.json")
echo "$ops" | "$apply_sh"

expected='move w1:p5 --new-tab --label quaqua-workers --no-focus
move w1:p7 --tab w1:t90 --split down --target-pane w1:p5 --ratio 0.5 --no-focus
move w1:p6 --tab w1:t90 --split right --target-pane w1:p5 --ratio 0.5 --no-focus'
actual=$(cat "$HERDR_STUB_LOG")
[[ "$actual" == "$expected" ]] || fail "test 4 mismatch: expected '$expected', got '$actual'"
echo "ok: 3 workers -> new_tab + 2 splits in build order"

# -----------------------------------------------------------------------------
# Test 5: Pane ID remapping from pane.moved result (churn: p5 -> p5X)
# -----------------------------------------------------------------------------
echo "test 5: pane ID remapping"
reset_stub "$fixdir/3-workers.json"
export HERDR_STUB_MOVE_SUFFIX="X"
ops=$(/usr/bin/jq -c -f "$plan_jq" "$fixdir/3-workers.json")
echo "$ops" | "$apply_sh"

# In 3-workers plan:
# Op 0: moves w1:p5 -> becomes w1:p5X
# Op 1: moves w1:p7 (anchor w1:p5 -> remapped to w1:p5X!) -> becomes w1:p7X
# Op 2: moves w1:p6 (anchor w1:p5 -> remapped to w1:p5X!) -> becomes w1:p6X
expected='move w1:p5 --new-tab --label quaqua-workers --no-focus
move w1:p7 --tab w1:t90 --split down --target-pane w1:p5X --ratio 0.5 --no-focus
move w1:p6 --tab w1:t90 --split right --target-pane w1:p5X --ratio 0.5 --no-focus'
actual=$(cat "$HERDR_STUB_LOG")
[[ "$actual" == "$expected" ]] || fail "test 5 mismatch: expected '$expected', got '$actual'"
unset HERDR_STUB_MOVE_SUFFIX
echo "ok: pane IDs remapped correctly across moves"

# -----------------------------------------------------------------------------
# Test 6: Skips deferred ops
# -----------------------------------------------------------------------------
echo "test 6: skips deferred ops"
reset_stub "$fixdir/working-deferred.json"
ops=$(/usr/bin/jq -c -f "$plan_jq" "$fixdir/working-deferred.json")
# In working-deferred.json, worker w1:p5 is working -> deferred: true
# apply.sh should skip w1:p5
# Note: since op 0 was deferred, @workers tab was not created, so later ops that need @workers fail cleanly.
# Let's test single deferred op first:
def_op='[{"op":"move","pane":"w1:p5","deferred":true,"target":{"kind":"new_tab","label":"workers"}}]'
echo "$def_op" | "$apply_sh"
[[ ! -s "$HERDR_STUB_LOG" ]] || fail "expected no moves for deferred op"
echo "ok: deferred op skipped"

# -----------------------------------------------------------------------------
# Test 7: Closed pane handling
# -----------------------------------------------------------------------------
echo "test 7: closed pane handling"
reset_stub "$fixdir/1-worker.json"
bad_op='[{"op":"move","pane":"w1:p99","deferred":false,"target":{"kind":"split","tab":"w1:t1","anchor":"w1:p1","direction":"right","ratio":0.4}}]'
err_file="$T/err.7"
if echo "$bad_op" | "$apply_sh" 2>"$err_file"; then
  fail "expected closed pane to fail"
fi
grep -q "closed or not found" "$err_file" || fail "missing closed pane error in: $(cat "$err_file")"
[[ ! -s "$HERDR_STUB_LOG" ]] || fail "no moves should be attempted for closed pane"
echo "ok: closed pane detected and aborted without moves"

# -----------------------------------------------------------------------------
# Test 8: Failed nth move stops execution
# -----------------------------------------------------------------------------
echo "test 8: failed nth move stops execution"
reset_stub "$fixdir/3-workers.json"
export HERDR_STUB_MOVE_FAIL_AT=2
export HERDR_STUB_MOVE_REASON="split_error"
ops=$(/usr/bin/jq -c -f "$plan_jq" "$fixdir/3-workers.json")
err_file="$T/err.8"
if echo "$ops" | "$apply_sh" 2>"$err_file"; then
  fail "expected failed move to stop apply.sh"
fi
grep -q "changed=false" "$err_file" || fail "missing failure report in: $(cat "$err_file")"
grep -q "split_error" "$err_file" || fail "missing failure reason in: $(cat "$err_file")"
actual_moves=$(wc -l < "$HERDR_STUB_LOG" | tr -d ' ')
[[ "$actual_moves" -eq 2 ]] || fail "expected exactly 2 moves attempted (move 1 succeeded, move 2 failed), got $actual_moves"
unset HERDR_STUB_MOVE_FAIL_AT HERDR_STUB_MOVE_REASON
echo "ok: failed move stops subsequent ops"

# -----------------------------------------------------------------------------
# Test 9: Zoomed tab handling (pre-move snapshot check)
# -----------------------------------------------------------------------------
echo "test 9: zoomed tab handling (snapshot check)"
reset_stub "$apply_fixdir/zoomed-tab.json"
ops=$(/usr/bin/jq -c -f "$plan_jq" "$fixdir/1-worker.json")
err_file="$T/err.9"
if echo "$ops" | "$apply_sh" 2>"$err_file"; then
  fail "expected zoomed tab to fail"
fi
grep -qi "zoomed" "$err_file" || fail "missing zoomed tab report in: $(cat "$err_file")"
[[ ! -s "$HERDR_STUB_LOG" ]] || fail "no moves should be attempted when tab is zoomed"
echo "ok: zoomed tab detected in snapshot before move"

# -----------------------------------------------------------------------------
# Test 10: Zoomed tab handling (herdr move reports zoomed_tab reason)
# -----------------------------------------------------------------------------
echo "test 10: zoomed tab handling (move result check)"
reset_stub "$fixdir/1-worker.json"
export HERDR_STUB_MOVE_FAIL_AT=1
export HERDR_STUB_MOVE_REASON="zoomed_tab"
ops=$(/usr/bin/jq -c -f "$plan_jq" "$fixdir/1-worker.json")
err_file="$T/err.10"
if echo "$ops" | "$apply_sh" 2>"$err_file"; then
  fail "expected zoomed_tab failure"
fi
grep -q "zoomed_tab" "$err_file" || fail "missing zoomed_tab reason in: $(cat "$err_file")"
unset HERDR_STUB_MOVE_FAIL_AT HERDR_STUB_MOVE_REASON
echo "ok: zoomed_tab reason handled and reported"

# -----------------------------------------------------------------------------
# Test 11: Foreign / untagged pane is never moved
# -----------------------------------------------------------------------------
echo "test 11: untagged pane rejected"
reset_stub "$fixdir/untagged-pane.json"
# Try to move w1:p2 (untagged pane with tokens=null in untagged-pane.json)
untagged_op='[{"op":"move","pane":"w1:p2","deferred":false,"target":{"kind":"new_tab","label":"workers"}}]'
err_file="$T/err.11"
if echo "$untagged_op" | "$apply_sh" 2>"$err_file"; then
  fail "expected untagged pane move to fail"
fi
grep -qi "candidate worker" "$err_file" || fail "missing untagged error in: $(cat "$err_file")"
[[ ! -s "$HERDR_STUB_LOG" ]] || fail "no moves should be attempted for untagged pane"

# Try to move w1:p3 (pane with grid_ready=0 in untagged-pane.json)
notready_op='[{"op":"move","pane":"w1:p3","deferred":false,"target":{"kind":"new_tab","label":"workers"}}]'
err_file="$T/err.11b"
if echo "$notready_op" | "$apply_sh" 2>"$err_file"; then
  fail "expected not-ready pane move to fail"
fi
grep -qi "candidate worker" "$err_file" || fail "missing not-ready error in: $(cat "$err_file")"
[[ ! -s "$HERDR_STUB_LOG" ]] || fail "no moves should be attempted for not-ready pane"
echo "ok: untagged and not-ready panes rejected, no move attempted"

# -----------------------------------------------------------------------------
# Test 12: Other-owner anchor rejected
# -----------------------------------------------------------------------------
echo "test 12: other-owner anchor rejected"
reset_stub "$apply_fixdir/other-owner.json"
# In other-owner.json, w1:p5 is owner "cos", w1:p6 is worker of "other_owner" in tab w1:t2
other_op='[{"op":"move","pane":"w1:p5","deferred":false,"target":{"kind":"split","tab":"w1:t2","anchor":"w1:p6","direction":"right","ratio":0.5}}]'
err_file="$T/err.12"
if echo "$other_op" | "$apply_sh" 2>"$err_file"; then
  fail "expected other-owner anchor to fail"
fi
grep -qi "owner" "$err_file" || fail "missing other-owner error in: $(cat "$err_file")"
[[ ! -s "$HERDR_STUB_LOG" ]] || fail "no moves should be attempted for other-owner anchor"
echo "ok: other-owner anchor rejected, no move attempted"

# -----------------------------------------------------------------------------
# Test 13: End-to-end pipeline with plan.jq
# -----------------------------------------------------------------------------
echo "test 13: pipeline plan.jq | apply.sh"
reset_stub "$fixdir/2-workers.json"
/usr/bin/jq -c -f "$plan_jq" "$HERDR_STUB_STATE" | "$apply_sh"
moves_count=$(wc -l < "$HERDR_STUB_LOG" | tr -d ' ')
[[ "$moves_count" -eq 2 ]] || fail "expected 2 moves from plan pipeline, got $moves_count"

# Second run on updated state produces 0 ops (idempotence)
second_ops=$(/usr/bin/jq -c -f "$plan_jq" "$HERDR_STUB_STATE")
echo "$second_ops" | "$apply_sh"
second_moves=$(wc -l < "$HERDR_STUB_LOG" | tr -d ' ')
[[ "$second_moves" -eq 2 ]] || fail "expected no additional moves on second run, got $second_moves"
echo "ok: pipeline plan.jq | apply.sh is idempotent"

echo "ALL TESTS PASSED"