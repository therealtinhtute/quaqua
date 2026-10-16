# plan.jq — turn a herdr api snapshot JSON into an ordered list of layout ops.
#
# Requirements:
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
# Ops use pane.move only:
#   {
#     "op": "move",
#     "pane": "<pane_id>",
#     "deferred": <boolean>,
#     "target": {
#       "kind": "new_tab" | "split",
#       "label": "<tab_label>" (when kind == "new_tab"),
#       "tab": "<tab_id>" | "@workers" (when kind == "split"),
#       "anchor": "<pane_id>" (when kind == "split"),
#       "direction": "right" | "down" (when kind == "split"),
#       "ratio": <float> (when kind == "split")
#     }
#   }

def tol: 2;
def abs: if . < 0 then -. else . end;
def near($a; $b): (($a - $b) | abs) <= tol;
def rect_near($a; $b):
  if ($a == null or $b == null) then false
  else
    near($a.x; $b.x) and near($a.y; $b.y) and near($a.width; $b.width) and near($a.height; $b.height)
  end;

# Parse pane sequence from id (e.g. w1:p5 -> 5, w1:pA -> 10)
def pane_seq:
  (try (capture(":p(?<s>[0-9A-Za-z]+)$").s) catch "") as $s
  | if $s == "" then 0
    else
      ("123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ0" | split("")) as $alpha
      | reduce ($s | split("")[]) as $c (0; . * 36 + (($alpha | index($c) // 0) + 1))
    end;

# Check if a pane is a candidate worker (R5)
def is_candidate_worker:
  (.tokens? != null) and
  ((.tokens.grid_ready // "") == "1" or .tokens.grid_ready == 1) and
  ((.tokens.grid_role // "") == "worker") and
  ((.tokens.grid_owner // "") != "") and
  ((.tokens.grid_group // "") != "");

# idle, done, unknown = not working (R7)
def is_working:
  . == "working";

# Check if a tab label matches a group (exact, with/without -workers suffix, or default "workers")
def is_group_tab($group; $label):
  if ($label == null or $label == "") then false
  else
    ($label == $group)
    or ($label == "\($group)-workers")
    or ($label == ($group | rtrimstr("-workers")))
    or ($label == "workers")
  end;

# Desired cells for n workers in area $a (index 0 = first spawned)
def cells($n; $a):
  ($a.x // 0) as $x | ($a.y // 0) as $y | ($a.width // 200) as $w | ($a.height // 50) as $h
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
    elif $n == 4 then
      [ {x:$x, y:$y, width:$hw, height:$hh},
        {x:($x + $hw), y:$y, width:($w - $hw), height:$hh},
        {x:$x, y:($y + $hh), width:$hw, height:($h - $hh)},
        {x:($x + $hw), y:($y + $hh), width:($w - $hw), height:($h - $hh)} ]
    else
      []
    end;

# Construction order: parents before children
def build_order($n):
  if $n == 1 then [0]
  elif $n == 2 then [0, 1]
  elif $n == 3 then [0, 2, 1]
  elif $n == 4 then [0, 2, 1, 3]
  else [] end;

# Split target specification helper
def split_target($n; $i; $gws; $tab_ref):
  if $n == 2 then
    {
      kind: "split",
      tab: $tab_ref,
      anchor: $gws[0].pane_id,
      direction: "right",
      ratio: 0.5
    }
  elif $n == 3 then
    if $i == 2 then
      {
        kind: "split",
        tab: $tab_ref,
        anchor: $gws[0].pane_id,
        direction: "down",
        ratio: 0.5
      }
    else # $i == 1
      {
        kind: "split",
        tab: $tab_ref,
        anchor: $gws[0].pane_id,
        direction: "right",
        ratio: 0.5
      }
    end
  elif $n == 4 then
    if $i == 2 then
      {
        kind: "split",
        tab: $tab_ref,
        anchor: $gws[0].pane_id,
        direction: "down",
        ratio: 0.5
      }
    elif $i == 1 then
      {
        kind: "split",
        tab: $tab_ref,
        anchor: $gws[0].pane_id,
        direction: "right",
        ratio: 0.5
      }
    else # $i == 3
      {
        kind: "split",
        tab: $tab_ref,
        anchor: $gws[2].pane_id,
        direction: "right",
        ratio: 0.5
      }
    end
  else
    {}
  end;

# Find anchor pane for an owner
def find_owner_pane($owner; $panes; $agents; $ws):
  # 1. Pane tagged with matching grid_owner and non-worker role
  ([ $panes[] | select(.tokens? and .tokens.grid_owner == $owner and (.tokens.grid_role != "worker")) | .pane_id ] | first)
  //
  # 2. Registered agent whose name == $owner
  ([ $agents[] | select(.name == $owner) | .pane_id ] | first)
  //
  # 3. Pane whose label == $owner
  ([ $panes[] | select(.label == $owner) | .pane_id ] | first)
  //
  # 4. Pane whose agent == $owner
  ([ $panes[] | select(.agent == $owner) | .pane_id ] | first)
  //
  # 5. For cos only: pane tagged orchestrator/lead/cos or first non-worker pane
  (if $owner == "cos" then
     ([ $panes[] | select(.workspace_id == $ws and .tokens? and (.tokens.grid_role == "orchestrator" or .tokens.grid_role == "cos" or .tokens.grid_role == "lead")) | .pane_id ] | first)
     //
     ([ $panes[] | select(.workspace_id == $ws and (is_candidate_worker | not)) | .pane_id ] | first)
   else null end)
  //
  null;

# A split whose anchor is deferred cannot run, and a split into @workers
# cannot run while the group's new tab is deferred. apply_ops and the hook
# propagate deferral by this same rule; the planner must too, or the check
# counts an unrunnable op as pending and times out while its anchor works.
def propagate_deferrals:
  ( reduce .[] as $op
      ( {ops: [], deferred: [], tab_deferred: false};
        if $op.deferred then
          { ops: (.ops + [$op]),
            deferred: (.deferred + [$op.pane]),
            tab_deferred: (if $op.target.kind == "new_tab" then true else .tab_deferred end) }
        elif ($op.target.kind == "split"
              and (((.deferred | index($op.target.anchor)) != null)
                   or ($op.target.tab == "@workers" and .tab_deferred))) then
          { ops: (.ops + [$op + {deferred: true}]),
            deferred: (.deferred + [$op.pane]),
            tab_deferred: .tab_deferred }
        else
          { ops: (.ops + [$op]), deferred: .deferred, tab_deferred: .tab_deferred }
        end )
    | .ops );

# ---------------------------------------------------------------------------
# Main pipeline
# ---------------------------------------------------------------------------
(.result.snapshot // .snapshot // .) as $root
| ($root.panes // []) as $panes
| ($root.tabs // []) as $tabs
| ($root.layouts // []) as $layouts
| ($root.agents // []) as $agents
| ($root.workspaces // []) as $workspaces

# Map of tab_id -> layout
| ( [ $layouts[] | {key: .tab_id, value: .} ] | from_entries ) as $layout_by_tab

# Extract all candidate workers in allocation order
| [ $panes[] | select(is_candidate_worker) ] as $candidate_workers
| ( if ($candidate_workers | length) == 0 then
    []
  else
    # Distinct owners in first-seen order
    ( [ $candidate_workers[].tokens.grid_owner ] | unique ) as $owners
    | reduce $owners[] as $owner
        ( [];
          ( [ $candidate_workers[] | select(.tokens.grid_owner == $owner) ]
            | sort_by((.pane_id | pane_seq), .pane_id) ) as $owner_workers
          | ($owner_workers | length) as $total
          | if $total == 0 then
              .
            else
              ($owner_workers[0].workspace_id // "") as $owner_ws
              | find_owner_pane($owner; $panes; $agents; $owner_ws) as $anchor_pane_id
              | ([ $panes[] | select(.pane_id == $anchor_pane_id) ] | first // null) as $anchor_pane
              | ($anchor_pane.tab_id // null) as $anchor_tab

              | if ($total == 1 and $anchor_pane != null) then
                  # ---------------------------------------------------------
                  # 1 worker total for this owner -> split beside owner
                  # ---------------------------------------------------------
                  $owner_workers[0] as $w
                  # "Beside the anchor" = shares its tab. The repair path
                  # (reconcile.rs `beside`) accepts the same, and a herdr pane
                  # split lands 50/50, so a ratio-based test here would make
                  # --check time out on an already-arranged pane.
                  | ( ($anchor_tab != null) and ($w.tab_id == $anchor_tab) ) as $in_place

                  | if $in_place then
                      .
                    else
                      . + [
                        {
                          op: "move",
                          pane: $w.pane_id,
                          deferred: ($w.agent_status | is_working),
                          target: {
                            kind: "split",
                            tab: $anchor_tab,
                            anchor: $anchor_pane.pane_id,
                            direction: "right",
                            ratio: 0.4
                          }
                        }
                      ]
                    end
                else
                  # ---------------------------------------------------------
                  # >1 workers total for this owner -> one tab per group
                  # ---------------------------------------------------------
                  ( [ $owner_workers[].tokens.grid_group ] | unique ) as $groups
                  | reduce $groups[] as $group
                      ( .;
                        ( [ $owner_workers[] | select(.tokens.grid_group == $group) ]
                          | sort_by((.pane_id | pane_seq), .pane_id) ) as $gws
                        | ($gws | length) as $n

                        | if $n > 4 then
                            error("too many workers in group \($group): \($n) (max 4)")
                          else
                            # Search for an existing valid workers tab for this group
                            ( [ $tabs[]
                                | select((.workspace_id == $owner_ws or $owner_ws == "")
                                         and ($anchor_tab == null or .tab_id != $anchor_tab)
                                         and is_group_tab($group; .label))
                                | .tab_id as $tid
                                | [ $panes[] | select(.tab_id == $tid) ] as $occ
                                | select(($occ | length > 0)
                                         and ($occ | all(. as $p | any($gws[]; .pane_id == $p.pane_id))))
                              ]
                              | sort_by(if (.label == $group or .label == "\($group)-workers" or .label == ($group | rtrimstr("-workers"))) then 0 else 1 end)
                              | first // null ) as $valid_tab

                            | if $valid_tab != null then
                                $valid_tab.tab_id as $wt_id
                                | ($layout_by_tab[$wt_id] // null) as $wt_layout
                                | ($wt_layout.area // {x:0, y:0, width:200, height:50}) as $wt_area
                                | ( ($gws | all(.tab_id == $wt_id))
                                    and ([ $panes[] | select(.tab_id == $wt_id) ] | length == $n) ) as $all_present

                                | ( if $all_present then
                                      if $n == 1 then
                                        ([ $wt_layout.panes[] | select(.pane_id == $gws[0].pane_id) | .rect ] | first // null) as $r0
                                        | rect_near($r0; $wt_area)
                                      else
                                        cells($n; $wt_area) as $req_cells
                                        | [ range(0; $n) ] | all(
                                            . as $i
                                            | ([ $wt_layout.panes[] | select(.pane_id == $gws[$i].pane_id) | .rect ] | first // null) as $ri
                                            | rect_near($ri; $req_cells[$i])
                                          )
                                      end
                                    else false end ) as $in_place

                                | if $in_place then
                                    .
                                  else
                                    # Existing tab exists but needs placement/completion
                                    ( [ $gws[] | select(.tab_id == $wt_id) ] | length ) as $in_tab_count
                                    | if ($in_tab_count > 0 and $gws[0].tab_id == $wt_id) then
                                        # Worker 0 is already in the tab; place remaining workers
                                        . + [
                                          build_order($n)[] as $i
                                          | select($i > 0 and $gws[$i].tab_id != $wt_id)
                                          | {
                                              op: "move",
                                              pane: $gws[$i].pane_id,
                                              deferred: ($gws[$i].agent_status | is_working),
                                              target: split_target($n; $i; $gws; $wt_id)
                                            }
                                        ]
                                      else
                                        # Need full rebuild in workers tab
                                        . + [
                                          build_order($n)[] as $i
                                          | if $i == 0 then
                                              {
                                                op: "move",
                                                pane: $gws[0].pane_id,
                                                deferred: ($gws[0].agent_status | is_working),
                                                target: {
                                                  kind: "new_tab",
                                                  label: $group
                                                }
                                              }
                                            else
                                              {
                                                op: "move",
                                                pane: $gws[$i].pane_id,
                                                deferred: ($gws[$i].agent_status | is_working),
                                                target: split_target($n; $i; $gws; "@workers")
                                              }
                                            end
                                        ]
                                      end
                                  end
                              else
                                # No valid workers tab exists -> create new tab and split
                                . + [
                                  build_order($n)[] as $i
                                  | if $i == 0 then
                                      {
                                        op: "move",
                                        pane: $gws[0].pane_id,
                                        deferred: ($gws[0].agent_status | is_working),
                                        target: {
                                          kind: "new_tab",
                                          label: $group
                                        }
                                      }
                                    else
                                      {
                                        op: "move",
                                        pane: $gws[$i].pane_id,
                                        deferred: ($gws[$i].agent_status | is_working),
                                        target: split_target($n; $i; $gws; "@workers")
                                      }
                                    end
                                ]
                              end
                          end
                      )
                end
            end
        )
  end
) | propagate_deferrals
