#!/usr/bin/env bash
# grid-workers.sh — arrange worker panes around the anchor pane (herdr).
#
# The anchor is the caller's pane: the CoS, or a project orchestrator. Workers
# are the agents owned by the caller (runs/<name>.owner == QUAQUA_OWNER,
# default cos; no .owner file = cos) whose registered name matches the worker
# pattern <kind>- (tieude, detu, sude, ni, suhuynh, truonglao, bacdau,
# quansu), excluding *-lead names (a project Lead, <slug>-lead, is never a
# worker), and whose pane is in the anchor's tab or in a group tab: a tab
# labelled with a known group label (--tab, default "workers", or any
# runs/<name>.grid) or its scratch tab. Agents in any other tab, another
# owner's workers, and unnamed or non-worker agents are never touched.
#
# Each worker belongs to the group runs/<name>.grid (spawn-worker.sh writes
# <repo>-workers), else the group of the tab it sits in, else --tab. Groups
# are arranged one after another.
#
# Layout rules:
#   1 worker in total -> same tab as the anchor, split right; anchor 2/5,
#                        worker 3/5.
#   otherwise   -> each group in its own tab labelled <group> (a group of
#                  1 = one full pane):
#                    2 = left|right
#                    3 = A|B top + C full width below
#                    4 = 2x2
#                  the anchor tab keeps only the anchor. An existing tab is
#                  reused only if it is not the anchor tab and holds nothing
#                  but workers.
#   back to 1   -> the worker returns to the anchor tab.
#   >4 workers in a group -> error exit.
# A pane whose agent is working is never moved (skipped with a warning). The
# run stops (exit 1) if a pane starts working between planning and moving, or
# if herdr reports a move that changed nothing — re-run to replan.
#
# Workers are ordered by spawn order (pane id allocation order) and their panes
# are labelled with the worker name (the agent's registered name).
#
# Usage: grid-workers.sh [--tab <label>] [--only <names>] [--dry-run]
#   --tab <label>  label of the 2-4 worker tab (default: workers)
#   --only <names> comma-separated worker names: only the listed agents are
#                  moved or renamed (e.g. a pipeline's own builder+reviewer),
#                  but the owner's whole worker population still decides the
#                  layout — an unlisted co-owner worker keeps the listed ones
#                  out of the anchor tab; unlisted agents are never touched
#   --dry-run      print the planned actions and exit without touching any pane.
#
# Requires herdr and jq; run inside a herdr pane or set HERDR_PANE_ID.
set -euo pipefail

PROG=${0##*/}
. "$(dirname "$0")/../lib/common.sh"

usage() {
  cat <<'EOF'
Usage: grid-workers.sh [--tab <label>] [--only <names>] [--dry-run]

Arrange the caller's worker panes around the anchor pane (the caller's pane):
  1 worker in total  same tab as the anchor, split right (anchor 2/5, worker 3/5)
  otherwise          one tab per group (runs/<name>.grid, else --tab, default
                     "workers"): 1 = full, 2 = left|right, 3 = A|B top +
                     C below, 4 = 2x2
  >4 in a group      error
Workers are the agents owned by QUAQUA_OWNER (default cos; runs/<name>.owner)
in the anchor's tab or a group tab whose registered name matches <kind>- (but
not a <slug>-lead — a project Lead is never a worker); other tabs, other
owners' workers, and unnamed or non-worker agents are never touched.
Panes whose agent is working are never moved (skipped with a warning). If a
pane starts working mid-run, or a move changes nothing, the run stops and asks
for a replan.

  --tab <label>  label of the 2-4 worker tab (default: workers)
  --only <names> comma-separated worker names: only the listed agents are
                 moved or renamed; the owner's whole worker population still
                 decides the layout
  --dry-run      print the planned actions and exit without touching any pane.
EOF
}

# ---------------------------------------------------------------------------
# gather_state — read live herdr state, print one JSON state object.
# ---------------------------------------------------------------------------
# runs_meta — {<name>: {owner, grid}} from runs/<name>.owner and .grid.
runs_meta() {
  local dir=${QUAQUA_RUNS_DIR:-$HOME/quaqua/runs} f b
  for f in "$dir"/*.owner "$dir"/*.grid; do
    [ -f "$f" ] || continue
    b=${f##*/}
    printf '%s\t%s\t%s\n' "${b%.*}" "${b##*.}" "$(head -n 1 "$f")"
  done | jq -Rnc 'reduce (inputs | split("\t")) as $e ({}; .[$e[0]][$e[1]] = $e[2])'
}

gather_state() {  # gather_state <group-label> <default-label> <only-csv>
  local label=$1 dlabel=$2 only_csv=${3:-}
  local anchor_pane=${HERDR_PANE_ID:-}
  if [ -z "$anchor_pane" ]; then
    anchor_pane=$(herdr pane current 2>/dev/null | jq -r '.result.pane.pane_id // empty') || true
  fi
  [ -n "$anchor_pane" ] || die "cannot determine the anchor pane (the caller's pane); run inside a herdr pane or set HERDR_PANE_ID"

  local anchor_json anchor_tab anchor_ws
  anchor_json=$(herdr pane get "$anchor_pane")
  anchor_tab=$(jq -r '.result.pane.tab_id' <<<"$anchor_json")
  anchor_ws=$(jq -r '.result.pane.workspace_id' <<<"$anchor_json")

  local agents panes tabs
  agents=$(herdr agent list)
  panes=$(herdr pane list --workspace "$anchor_ws")
  tabs=$(herdr tab list --workspace "$anchor_ws")

  # Members: the caller's (QUAQUA_OWNER) agents in the anchor's workspace whose
  # pane is in the anchor's tab or in a group tab (a known group label, or its
  # scratch tab) and whose registered name matches the worker pattern <kind>-
  # (the spawn-worker roster). Agents in other tabs, other owners' workers, and
  # unnamed or non-worker agents are never touched. A name ending
  # in -lead is a project Lead (spawn-lead.sh names it <slug>-lead), never a
  # worker — without the exclusion a slug like sude-x would put its Lead in
  # the grid. Pane ids are allocated in creation order; the suffix alphabet
  # is 1-9, A-Z, 0 (base 36, zero last).
  # Group: runs/<name>.grid, else the group tab it sits in, else
  # <default-label>. Membership, population and groups are owner-wide and do
  # not depend on --only: a filtered run must still see the owner's other
  # workers, or a lone listed worker would wrongly take the beside-anchor
  # layout. --only only scopes which members may be moved or renamed.
  local only meta members workers group_all
  only=$(jq -nc --arg csv "$only_csv" '$csv | split(",") | map(select(. != ""))')
  meta=$(runs_meta)
  members=$(jq -c --arg anchor "$anchor_pane" --arg ws "$anchor_ws" --arg atab "$anchor_tab" \
    --arg dlabel "$dlabel" --arg me "${QUAQUA_OWNER:-cos}" --argjson tabs "$tabs" \
    --argjson meta "$meta" '
    def pane_seq:
      (try (capture(":p(?<s>[0-9A-Za-z]+)$").s) catch "") as $s
      | ("123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ0" | split("")) as $alpha
      | reduce ($s | split("")[]) as $c (0; . * 36 + (($alpha | index($c)) + 1));
    def worker_name: test("^(tieude|detu|sude|ni|suhuynh|truonglao|bacdau|quansu)-") and (test("-lead$") | not);
    ([ $meta[] | .grid // empty ] + [$dlabel] | unique) as $known
    | ( [ $tabs.result.tabs[]
          | select(.workspace_id == $ws and .tab_id != $atab)
          | (.label | ltrimstr("grid-scratch-")) as $g
          | select($known | index($g))
          | {key: .tab_id, value: $g} ] | from_entries ) as $group_of_tab
    | [ .result.agents[]
        | select(.pane_id != $anchor and .workspace_id == $ws)
        | select(.tab_id == $atab or $group_of_tab[.tab_id] != null)
        | select((.name // "") | worker_name)
        | ($meta[.name] // {}) as $m
        | select(($m.owner // "cos") == $me)
        | { name: .name, pane: .pane_id, tab: .tab_id,
            status: .agent_status, seq: (.pane_id | pane_seq),
            group: ($m.grid // $group_of_tab[.tab_id] // $dlabel) } ]
    | sort_by(.seq)' <<<"$agents")
  # group_all: every member of this group (the layout is planned for all of
  # them); workers: the subset --only allows this run to touch.
  group_all=$(jq -c --arg g "$label" '[ .[] | select(.group == $g) ]' <<<"$members")
  workers=$(jq -c --argjson only "$only" \
    '[ .[] | select(($only | length) == 0 or (.name as $n | $only | index($n))) ]' \
    <<<"$group_all")

  # The group's workers tab: a tab labelled <label> that is not the anchor tab
  # and whose occupants are all members of this group — an occupant that is a
  # group member but not --only-listed stays legitimate (a second pipeline's
  # worker must not force a duplicate tab), while anything foreign to the
  # group (a plain shell, another group's worker) bars reuse.
  local workers_tab
  workers_tab=$(jq -r --argjson group_all "$group_all" --argjson panes "$panes" \
    --arg atab "$anchor_tab" --arg label "$label" '
    [ .result.tabs[] | select(.label == $label and .tab_id != $atab) ] as $t
    | ( [ $t[]
          | .tab_id as $id
          | [ $panes.result.panes[] | select(.tab_id == $id) ] as $occ
          | select(($occ | length) > 0
                   and all($occ[]; .pane_id as $p | any($group_all[]; .pane == $p))) ]
        | first // null )
    | .tab_id // ""' <<<"$tabs")

  local wt_pane="" wt_layout="null"
  if [ -n "$workers_tab" ]; then
    wt_pane=$(jq -r --arg t "$workers_tab" '[ .result.panes[] | select(.tab_id == $t) | .pane_id ] | first // ""' <<<"$panes")
    if [ -n "$wt_pane" ]; then
      wt_layout=$(herdr pane layout --pane "$wt_pane")
    fi
  fi

  local anchor_layout
  anchor_layout=$(herdr pane layout --pane "$anchor_pane")

  jq -n -c \
    --arg anchor_pane "$anchor_pane" --arg anchor_tab "$anchor_tab" --arg anchor_ws "$anchor_ws" \
    --argjson workers "$workers" --argjson group_all "$group_all" --arg workers_tab "$workers_tab" --arg label "$label" \
    --argjson members "$members" --argjson only "$only" --argjson anchor_layout "$anchor_layout" --argjson wt_layout "$wt_layout" '
    def rects($l): [ $l.result.layout.panes[] | {key: .pane_id, value: .rect} ] | from_entries;
    {
      anchor_pane: $anchor_pane, anchor_tab: $anchor_tab, anchor_ws: $anchor_ws,
      workers: $workers,
      group_all: $group_all,
      only: $only,
      total: ($members | length),
      groups: ($members | map(.group) | unique),
      workers_tab: (if $workers_tab == "" then null else $workers_tab end),
      workers_tab_label: $label,
      anchor_area: $anchor_layout.result.layout.area,
      anchor_rects: rects($anchor_layout),
      wt_area: (if $wt_layout == null then null else $wt_layout.result.layout.area end),
      wt_rects: (if $wt_layout == null then null else rects($wt_layout) end)
    }'
}

# ---------------------------------------------------------------------------
# plan_layout — pure planner: state JSON on stdin, plan JSON array on stdout.
# Actions: evict | place(new-tab|split) | rename | warn.
# ---------------------------------------------------------------------------
PLAN_JQ='
def tol: 2;
def abs: if . < 0 then -. else . end;
def near($a; $b): (($a - $b) | abs) <= tol;
def rect_near($a; $b):
  near($a.x; $b.x) and near($a.y; $b.y) and near($a.width; $b.width) and near($a.height; $b.height);

# Desired cells for n workers in area $a (index 0 = first spawned).
def cells($n; $a):
  ($a.x) as $x | ($a.y) as $y | ($a.width) as $w | ($a.height) as $h
  | ($w / 2 | floor) as $hw | ($h / 2 | floor) as $hh
  | ($w * 2 / 5 | floor) as $cw
  | if $n == 1 then
      [ {x:$x, y:$y, width:$cw, height:$h},
        {x:($x + $cw), y:$y, width:($w - $cw), height:$h} ]
    elif $n == 2 then
      [ {x:$x, y:$y, width:$hw, height:$h},
        {x:($x + $hw), y:$y, width:($w - $hw), height:$h} ]
    elif $n == 3 then
      [ {x:$x, y:$y, width:$hw, height:$hh},
        {x:($x + $hw), y:$y, width:($w - $hw), height:$hh},
        {x:$x, y:($y + $hh), width:$w, height:($h - $hh)} ]
    else
      [ {x:$x, y:$y, width:$hw, height:$hh},
        {x:($x + $hw), y:$y, width:($w - $hw), height:$hh},
        {x:$x, y:($y + $hh), width:$hw, height:($h - $hh)},
        {x:($x + $hw), y:($y + $hh), width:($w - $hw), height:($h - $hh)} ]
    end;

def containers($a):
  ($a.x) as $x | ($a.y) as $y | ($a.width) as $w | ($a.height) as $h
  | ($h / 2 | floor) as $hh
  | { root: {x:$x, y:$y, width:$w, height:$h},
      top: {x:$x, y:$y, width:$w, height:$hh},
      bottom: {x:$x, y:($y + $hh), width:$w, height:($h - $hh)} };

# Rects a worker pane may legitimately occupy: its own cell, or a container
# whose subtree it is the first worker of (the subtree collapsed to it).
def allowed($n; $i; $a):
  (cells($n; $a)) as $c | (containers($a)) as $k
  | if $n == 1 then [$k.root]
    else [ $c[$i] ]
      + (if $i == 0 then (if $n == 2 then [$k.root] else [$k.top, $k.root] end) else [] end)
      + (if $n == 4 and $i == 2 then [$k.bottom] else [] end)
    end;

# Construction order: parents before children.
def build_order($n): if $n == 1 then [0] elif $n == 2 then [0,1] elif $n == 3 then [0,2,1] else [0,2,1,3] end;

def placement($n; $i; $label):
  if $i == 0 then {mode:"new-tab", label:$label}
  elif $n == 2 then {mode:"split", tab:"@workers", dir:"right", anchor_idx:0, ratio:0.5}
  elif $n == 3 then
    (if $i == 2 then {mode:"split", tab:"@workers", dir:"down", anchor_idx:0, ratio:0.5}
     else {mode:"split", tab:"@workers", dir:"right", anchor_idx:0, ratio:0.5} end)
  else
    (if $i == 2 then {mode:"split", tab:"@workers", dir:"down", anchor_idx:0, ratio:0.5}
     elif $i == 1 then {mode:"split", tab:"@workers", dir:"right", anchor_idx:0, ratio:0.5}
     else {mode:"split", tab:"@workers", dir:"right", anchor_idx:2, ratio:0.5} end)
  end;

def rename_actions($ws): [ $ws[] | {action:"rename", pane:.pane, label:.name} ];

. as $s
| ($s.workers) as $ws        # movable: the group members --only selects
| ($s.group_all) as $gws     # every group member — decides the layout shape
| ($s.only) as $only         # empty = all movable
| ($gws | length) as $n
| ($n == 1 and $s.total == 1) as $beside
| (if $beside then $s.anchor_tab else $s.workers_tab end) as $target
| (if $beside then $s.anchor_area else $s.wt_area end) as $area
| (if $beside then $s.anchor_rects else $s.wt_rects end) as $rects0
| (if $rects0 == null then {} else $rects0 end) as $rects
| if $beside then
    ($ws[0]) as $w
    | (($w.tab == $s.anchor_tab)
       and ($rects[$w.pane] != null) and ($rects[$s.anchor_pane] != null)
       and rect_near($rects[$w.pane]; cells(1; $area)[1])
       and rect_near($rects[$s.anchor_pane]; cells(1; $area)[0])) as $in_place
    | if $in_place then
        rename_actions($ws)
      else
        ( [ if ($w.tab == $s.anchor_tab) and ($w.status != "working")
            then {action:"evict", pane:$w.pane} else empty end ]
          + [ if $w.status == "working"
              then {action:"warn", message:"skip \($w.name): agent is working"}
              else {action:"place", pane:$w.pane, mode:"split", tab:"@anchor",
                    dir:"right", anchor:$s.anchor_pane, ratio:0.4} end ]
          + rename_actions($ws) )
      end
  else
    ($rects | keys) as $tab_panes
    | ( [ $ws[] | select(.tab == $target) ] ) as $in_tab
    | ( [ $tab_panes[] | select( . as $p | all($gws[]; .pane != $p) ) ] | length == 0 ) as $all_workers
    | ( [ $in_tab[]
          | select( . as $w
                    | ($rects[$w.pane]) as $r
                    | ($r == null)
                      or ( (allowed($n; ($gws | index($w)); $area) | any(rect_near(.; $r))) | not ) ) ]
        | length == 0 ) as $all_valid
    | (($all_workers and $all_valid) | not) as $mismatch
    | (if $mismatch
       then [ $in_tab[] | select(.status != "working") | {action:"evict", pane:.pane} ]
       else [] end) as $evictions
    # place every group member that is movable under --only and not
    # correctly placed; unlisted members keep their cells but are not touched
    | ( [ build_order($n)[] | $gws[.] ]
        | map(select(($only | length) == 0 or (.name as $nm | $only | index($nm))))
        | if $mismatch then . else map(select(.tab != $target)) end ) as $to_place
    | ( [ $tab_panes[] | select( . as $p | all($evictions[]; .pane != $p) ) ] ) as $present0
    | ( [ $present0[] | {pane:., rect:($rects[.] // {x:0, y:0})} ]
        | sort_by(.rect.y, .rect.x) | map(.pane) ) as $present_sorted
    | ( reduce $to_place[] as $w
          ( {actions:[], present:$present_sorted};
            if $w.status == "working" then
              .actions += [{action:"warn", message:"skip \($w.name): agent is working"}]
            else
              (placement($n; ($gws | index($w)); $s.workers_tab_label)) as $pl
              | (if $pl.mode == "new-tab" then
                   (if (.present | length) == 0 then
                      .actions += [{action:"place", pane:$w.pane, mode:"new-tab", label:$pl.label}]
                    else
                      .actions += [{action:"place", pane:$w.pane, mode:"split", tab:"@workers",
                                    dir:"right", anchor:(.present[0]), ratio:0.5}]
                    end)
                 else
                   ($gws[$pl.anchor_idx].pane) as $anchor
                   | (if (.present | index($anchor)) != null then
                        .actions += [{action:"place", pane:$w.pane, mode:"split", tab:$pl.tab,
                                      dir:$pl.dir, anchor:$anchor, ratio:$pl.ratio}]
                      elif (.present | length) > 0 then
                        .actions += [{action:"place", pane:$w.pane, mode:"split", tab:$pl.tab,
                                      dir:$pl.dir, anchor:(.present[0]), ratio:$pl.ratio}]
                      else
                        .actions += [{action:"place", pane:$w.pane, mode:"new-tab",
                                      label:$s.workers_tab_label}]
                      end)
                 end)
              | .present += [$w.pane]
            end
          )
        ) as $build
    | ( $evictions + $build.actions + rename_actions($ws) )
  end
'

# ---------------------------------------------------------------------------
# render_plan — human-readable one line per action.
# ---------------------------------------------------------------------------
render_plan() {
  jq -r '.[] |
    if .action == "evict" then "evict \(.pane)"
    elif .action == "place" and .mode == "new-tab" then "place \(.pane) new-tab label=\(.label)"
    elif .action == "place" then "place \(.pane) split tab=\(.tab) dir=\(.dir) anchor=\(.anchor) ratio=\(.ratio)"
    elif .action == "rename" then "rename \(.pane) \(.label)"
    elif .action == "warn" then "warn \(.message)"
    else "unknown \(.)" end'
}

# ---------------------------------------------------------------------------
# recheck_idle — the plan was built from an earlier snapshot. Right before each
# move, confirm the pane's agent is still idle: a worker that started working
# since the snapshot must stop the run (a half-built grid is worse than none),
# so ask for a replan instead of moving a busy pane.
# ---------------------------------------------------------------------------
recheck_idle() {  # recheck_idle <pane>
  local pane=$1 status
  status=$(herdr agent list 2>/dev/null | jq -r --arg p "$pane" \
    '[ .result.agents[] | select(.pane_id == $p) | .agent_status ] | first // "unknown"' 2>/dev/null) \
    || status=unknown
  [ "$status" = idle ] \
    || die "pane $pane is '$status', not idle; state changed since planning — re-run to replan"
}

# ---------------------------------------------------------------------------
# validate_move — herdr reports a move that changed nothing as changed=false
# (reason same_tab / zoomed_tab). Treat anything but changed=true with a pane
# and a tab as a stop: the rest of the plan depends on this move.
# ---------------------------------------------------------------------------
validate_move() {  # validate_move <move-response> <action-json>
  local res=$1 action=$2 changed reason pane tab
  changed=$(jq -r '.result.move_result.changed // false' <<<"$res")
  pane=$(jq -r '.result.move_result.pane.pane_id // ""' <<<"$res")
  tab=$(jq -r '.result.move_result.pane.tab_id // ""' <<<"$res")
  if [ "$changed" != true ] || [ -z "$pane" ] || [ -z "$tab" ]; then
    reason=$(jq -r '.result.move_result.reason // "no reason given"' <<<"$res")
    die "pane.move changed nothing (changed=$changed, reason=$reason); re-run to replan: $action"
  fi
}

# ---------------------------------------------------------------------------
# execute_plan — run the plan through herdr. Panes that must be repositioned
# inside their current tab are parked in a scratch tab first, because herdr
# ignores a move whose destination is the pane's current tab.
# ---------------------------------------------------------------------------
execute_plan() {
  local plan=$1 state=$2
  local anchor_tab workers_tab scratch_label
  anchor_tab=$(jq -r '.anchor_tab' <<<"$state")
  workers_tab=$(jq -r '.workers_tab // ""' <<<"$state")
  scratch_label="grid-scratch-$(jq -r '.workers_tab_label' <<<"$state")"
  local scratch_tab="" scratch_root=""
  local -A pane_of=()
  local action kind orig pane res tab dir anchor ratio
  while IFS= read -r action; do
    kind=$(jq -r '.action' <<<"$action")
    case "$kind" in
      evict)
        orig=$(jq -r '.pane' <<<"$action"); pane=${pane_of[$orig]:-$orig}
        recheck_idle "$pane"
        if [ -z "$scratch_tab" ]; then
          res=$(herdr pane move "$pane" --new-tab --label "$scratch_label" --no-focus)
          validate_move "$res" "$action"
          scratch_tab=$(jq -r '.result.move_result.pane.tab_id' <<<"$res")
          scratch_root=$(jq -r '.result.move_result.pane.pane_id' <<<"$res")
        else
          res=$(herdr pane move "$pane" --tab "$scratch_tab" --split right \
                  --target-pane "$scratch_root" --ratio 0.5 --no-focus)
          validate_move "$res" "$action"
        fi
        pane_of[$orig]=$(jq -r '.result.move_result.pane.pane_id' <<<"$res")
        ;;
      place)
        orig=$(jq -r '.pane' <<<"$action"); pane=${pane_of[$orig]:-$orig}
        recheck_idle "$pane"
        if [ "$(jq -r '.mode' <<<"$action")" = new-tab ]; then
          res=$(herdr pane move "$pane" --new-tab --label "$(jq -r '.label' <<<"$action")" --no-focus)
          validate_move "$res" "$action"
          workers_tab=$(jq -r '.result.move_result.pane.tab_id' <<<"$res")
        else
          tab=$(jq -r '.tab' <<<"$action")
          case "$tab" in
            @anchor) tab=$anchor_tab ;;
            @workers) tab=$workers_tab ;;
          esac
          dir=$(jq -r '.dir' <<<"$action")
          anchor=$(jq -r '.anchor' <<<"$action"); anchor=${pane_of[$anchor]:-$anchor}
          ratio=$(jq -r '.ratio' <<<"$action")
          res=$(herdr pane move "$pane" --tab "$tab" --split "$dir" \
                  --target-pane "$anchor" --ratio "$ratio" --no-focus)
          validate_move "$res" "$action"
        fi
        pane_of[$orig]=$(jq -r '.result.move_result.pane.pane_id' <<<"$res")
        ;;
      rename)
        orig=$(jq -r '.pane' <<<"$action"); pane=${pane_of[$orig]:-$orig}
        herdr pane rename "$pane" "$(jq -r '.label' <<<"$action")" >/dev/null
        ;;
      warn)
        printf '%s: %s\n' "$PROG" "$(jq -r '.message' <<<"$action")" >&2
        ;;
      *)
        die "internal error: unknown action '$kind'"
        ;;
    esac
  done < <(jq -c '.[]' <<<"$plan")
}

main() {
  local dry_run=0 label=workers only_csv=''
  while [ $# -gt 0 ]; do
    case "$1" in
      --tab)
        [ $# -ge 2 ] || { usage >&2; exit 2; }
        label=$2; shift ;;
      --only)
        [ $# -ge 2 ] || { usage >&2; exit 2; }
        only_csv=$2; shift ;;
      --dry-run) dry_run=1 ;;
      -h|--help) usage; exit 0 ;;
      *) usage >&2; exit 2 ;;
    esac
    shift
  done
  [ -n "$label" ] || { usage >&2; exit 2; }

  local state n plan g groups=()
  state=$(gather_state "$label" "$label" "$only_csv")
  if [ "$(jq -r '.total' <<<"$state")" -eq 0 ]; then
    printf '%s: no workers to arrange\n' "$PROG" >&2
    exit 0
  fi
  mapfile -t groups < <(jq -r --arg l "$label" '[$l] + (.groups - [$l]) | .[]' <<<"$state")

  # one group at a time; each later group replans from fresh state
  for g in "${groups[@]}"; do
    [ "$g" = "$label" ] || state=$(gather_state "$g" "$label" "$only_csv")
    n=$(jq -r '.workers | length' <<<"$state")
    [ "$n" -gt 0 ] || continue   # under --only a group may hold nothing movable
    if [ "$(jq -r '.group_all | length' <<<"$state")" -gt 4 ]; then
      die "too many workers ($(jq -r '.group_all | length' <<<"$state")) in group $g; the grid supports at most 4"
    fi
    plan=$(jq -c "$PLAN_JQ" <<<"$state")
    if [ "$dry_run" = 1 ]; then
      render_plan <<<"$plan"
    else
      execute_plan "$plan" "$state"
    fi
  done
}

main "$@"
