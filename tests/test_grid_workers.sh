#!/usr/bin/env bash
# Sandbox test for grid-workers.sh layout planning. No live panes are touched:
# a stub herdr serves fixture state and records mutations.
# Usage: tests/test_grid_workers.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-grid-workers.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# owner/group metadata (runs/<name>.owner|.grid): empty unless a case adds it
export QUAQUA_RUNS_DIR="$T/runs" QUAQUA_OWNER=cos
mkdir -p "$QUAQUA_RUNS_DIR"

# ── stub herdr: reads $HERDR_STUB_STATE, logs mutations to $HERDR_STUB_LOG ───
mkdir -p "$T/bin"
cat > "$T/bin/herdr" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
state=${HERDR_STUB_STATE:?}
log=${HERDR_STUB_LOG:-/dev/null}
cmd=${1:-}; shift || true
case "$cmd" in
  pane)
    sub=${1:-}; shift || true
    case "$sub" in
      get)
        jq -c --arg id "${1:-}" '{result:{pane:(.panes[]|select(.pane_id==$id))}}' "$state" ;;
      list)
        jq -c '{result:{panes:.panes}}' "$state" ;;
      layout)
        id=""
        while [ $# -gt 0 ]; do case "$1" in --pane) id=$2; shift 2 ;; *) id=$1; shift ;; esac; done
        jq -c --arg id "$id" '(.panes[]|select(.pane_id==$id)|.tab_id) as $t | {result:{layout:.layouts[$t]}}' "$state" ;;
      current)
        jq -c '{result:{pane:(.panes[]|select(.focused))}}' "$state" ;;
      move)
        printf 'move %s\n' "$*" >> "$log"
        pane=$1; shift
        label=""; tab=""
        while [ $# -gt 0 ]; do
          case "$1" in
            --new-tab) tab=new ;;
            --label) label=$2; shift ;;
            --tab) tab=$2; shift ;;
          esac
          shift
        done
        if [ "$tab" = new ]; then
          case "$label" in
            workers) newtab="w1:t90" ;;
            grid-scratch-workers) newtab="w1:t91" ;;
            coucou-workers) newtab="w1:t92" ;;
            mono-workers) newtab="w1:t94" ;;
            bp-workers) newtab="w1:t95" ;;
            grid-scratch-coucou-workers) newtab="w1:t93" ;;
            *) newtab="w1:t99" ;;
          esac
        else
          newtab=$tab
        fi
        # optional id churn: every move renames the pane (p5 -> p5X -> p5XX)
        newpane=$pane${HERDR_STUB_MOVE_SUFFIX:-}
        # optional no-op: the Nth move reports changed=false with a reason
        n=$(( $(cat "$state.moves" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$state.moves"
        changed=true; reason=""
        if [ -n "${HERDR_STUB_MOVE_FAIL_AT:-}" ] && [ "$n" = "$HERDR_STUB_MOVE_FAIL_AT" ]; then
          changed=false; reason=${HERDR_STUB_MOVE_REASON:-zoomed_tab}
        fi
        if [ "$changed" = true ]; then
          # keep the state consistent: the pane keeps its agent under the new id;
          # a new tab is listed with its label
          jq -c --arg p "$pane" --arg np "$newpane" --arg t "$newtab" --arg l "$label" \
            '(.panes[] | select(.pane_id == $p) | .workspace_id) as $w
             | (if $l != "" and all(.tabs[]; .tab_id != $t)
                then .tabs += [{tab_id:$t, label:$l, workspace_id:$w}] else . end)
             | .agents |= map(if .pane_id == $p then .pane_id = $np | .tab_id = $t else . end)
             | .panes |= map(if .pane_id == $p then .pane_id = $np | .tab_id = $t else . end)' \
            "$state" > "$state.next" && mv "$state.next" "$state"
        fi
        jq -nc --arg p "$newpane" --arg t "$newtab" --arg prev "$pane" \
          --argjson ch "$changed" --arg r "$reason" \
          '{result:{move_result:{changed:$ch,reason:$r,pane:{pane_id:$p,tab_id:$t},previous_pane_id:$prev}}}' ;;
      rename)
        printf 'rename %s\n' "$*" >> "$log"
        jq -nc --arg p "$1" '{result:{pane:{pane_id:$p}}}' ;;
      *)
        echo "stub: unhandled pane $sub" >&2; exit 2 ;;
    esac ;;
  agent)
    sub=${1:-}; shift || true
    case "$sub" in
      list)
        jq -c '{result:{agents:.agents}}' "$state"
        # optional status flip: after the Nth agent list, every agent turns
        # working (a worker that starts working between plan and execute)
        if [ -n "${HERDR_STUB_FLIP_STATUS:-}" ]; then
          n=$(( $(cat "$state.lists" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$state.lists"
          if [ "$n" -ge "${HERDR_STUB_FLIP_AFTER:-1}" ]; then
            jq -c --arg s "$HERDR_STUB_FLIP_STATUS" '.agents |= map(.agent_status=$s)' "$state" > "$state.next" \
              && mv "$state.next" "$state"
          fi
        fi ;;
      *) echo "stub: unhandled agent $sub" >&2; exit 2 ;;
    esac ;;
  tab)
    sub=${1:-}; shift || true
    case "$sub" in
      list) jq -c '{result:{tabs:.tabs}}' "$state" ;;
      *) echo "stub: unhandled tab $sub" >&2; exit 2 ;;
    esac ;;
  *)
    echo "stub: unhandled command $cmd" >&2; exit 2 ;;
esac
STUB
chmod +x "$T/bin/herdr"

# ── fixture builder ──────────────────────────────────────────────────────────
# Spec lines:
#   anchor <pane>
#   worker <name> <agent> <pane> <tab> <status> [workspace]   (name "-" = unnamed)
#   tab <tab> <label>
#   layout <tab> <W>x<H> <pane>=<x>,<y>,<w>,<h> ...
mkstate() {
  local anchor="" ws="" workers='[]' tabs='[]' layouts='{}' line
  while IFS= read -r line; do
    case "$line" in
      ""|\#*) continue ;;
      anchor\ *) anchor=${line#anchor }; ws=${anchor%%:*} ;;
      worker\ *)
        local _ name agent pane tab status w
        read -r _ name agent pane tab status w <<<"$line"
        [ -n "${w:-}" ] || w=$ws
        # name and agent are independent, as in real herdr metadata
        if [ "$name" = "-" ]; then
          workers=$(jq -c --arg a "$agent" --arg p "$pane" --arg t "$tab" --arg s "$status" --arg w "$w" \
            '. + [{agent:$a,pane:$p,tab:$t,status:$s,workspace:$w}]' <<<"$workers")
        else
          workers=$(jq -c --arg n "$name" --arg a "$agent" --arg p "$pane" --arg t "$tab" --arg s "$status" --arg w "$w" \
            '. + [{name:$n,agent:$a,pane:$p,tab:$t,status:$s,workspace:$w}]' <<<"$workers")
        fi ;;
      tab\ *)
        local _ tab label
        read -r _ tab label <<<"$line"
        tabs=$(jq -c --arg t "$tab" --arg l "$label" --arg w "$ws" \
          '. + [{tab_id:$t,label:$l,workspace_id:$w}]' <<<"$tabs") ;;
      layout\ *)
        local _ tab area panes spec pid coords x y pw ph pj='[]'
        read -r _ tab area panes <<<"$line"
        local lw=${area%x*} lh=${area#*x}
        for spec in $panes; do
          pid=${spec%%=*}; coords=${spec#*=}
          IFS=, read -r x y pw ph <<<"$coords"
          pj=$(jq -c --arg p "$pid" --argjson x "$x" --argjson y "$y" --argjson w "$pw" --argjson h "$ph" \
            '. + [{pane_id:$p,rect:{x:$x,y:$y,width:$w,height:$h}}]' <<<"$pj")
        done
        layouts=$(jq -c --arg t "$tab" --argjson w "$lw" --argjson h "$lh" --argjson panes "$pj" \
          '. + {($t): {area:{x:0,y:0,width:$w,height:$h}, panes:$panes}}' <<<"$layouts") ;;
      *) echo "mkstate: bad line: $line" >&2; return 1 ;;
    esac
  done
  jq -nc --arg ws "$ws" --argjson workers "$workers" --argjson tabs "$tabs" --argjson layouts "$layouts" '
    {
      panes: [ $layouts | to_entries[] | .key as $tab | .value.panes[]
               | {pane_id:.pane_id, tab_id:$tab, workspace_id:$ws, agent:null,
                  agent_status:"unknown", focused:false} ],
      agents: [ $workers[] | {name:.name, pane_id:.pane, agent:.agent, agent_status:.status,
                              tab_id:.tab, workspace_id:.workspace} ],
      tabs: $tabs,
      layouts: $layouts
    }'
}

plan() {  # plan [label] [only-csv]; stdin: spec -> dry-run plan lines
  local label=${1:-} only=${2:-} spec anchor args=()
  spec=$(cat)
  anchor=$(sed -n 's/^anchor //p' <<<"$spec" | head -1)
  mkstate <<<"$spec" > "$T/state.json"
  [ -n "$label" ] && args+=(--tab "$label")
  [ -n "$only" ] && args+=(--only "$only")
  HERDR_STUB_STATE="$T/state.json" HERDR_PANE_ID="$anchor" PATH="$T/bin:$PATH" \
    "$src/worker/grid-workers.sh" "${args[@]}" --dry-run
}

run() {  # run [label] [only-csv]; stdin: spec -> mutation log (script executed against the stub)
  local label=${1:-} only=${2:-} spec anchor args=()
  spec=$(cat)
  anchor=$(sed -n 's/^anchor //p' <<<"$spec" | head -1)
  mkstate <<<"$spec" > "$T/state.json"
  : > "$T/log"; rm -f "$T/state.json.moves" "$T/state.json.lists"
  [ -n "$label" ] && args+=(--tab "$label")
  [ -n "$only" ] && args+=(--only "$only")
  HERDR_STUB_STATE="$T/state.json" HERDR_STUB_LOG="$T/log" HERDR_PANE_ID="$anchor" PATH="$T/bin:$PATH" \
    "$src/worker/grid-workers.sh" "${args[@]}"
  cat "$T/log"
}

run_fail() {  # run_fail [label]; stdin: spec -> mutation log; asserts the script exits non-zero
  local label=${1:-} spec anchor args=()
  spec=$(cat)
  anchor=$(sed -n 's/^anchor //p' <<<"$spec" | head -1)
  mkstate <<<"$spec" > "$T/state.json"
  : > "$T/log"; rm -f "$T/state.json.moves" "$T/state.json.lists"
  [ -n "$label" ] && args=(--tab "$label")
  rc=0
  HERDR_STUB_STATE="$T/state.json" HERDR_STUB_LOG="$T/log" HERDR_PANE_ID="$anchor" PATH="$T/bin:$PATH" \
    "$src/worker/grid-workers.sh" "${args[@]}" >"$T/out" 2>"$T/err" || rc=$?
  [ "$rc" -ne 0 ] || fail "expected a non-zero exit"
  cat "$T/log"
}

expect() {  # expect <name> <expected> <actual>
  if [ "$2" = "$3" ]; then
    echo "ok: $1"
  else
    printf 'FAIL: %s\nexpected:\n%s\nactual:\n%s\n' "$1" "$2" "$3" >&2
    exit 1
  fi
}

# ── 1. one worker, fresh spawn (50/50) -> rebuild to CoS 2/5 | worker 3/5 ────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,100,50
EOF
)
expect "1 worker: rebuild to 2/5|3/5" \
'evict w1:p5
place w1:p5 split tab=@anchor dir=right anchor=w1:p1 ratio=0.4
rename w1:p5 tieude-alpha' "$actual"

# ── 2. one worker already in place -> no moves (idempotent) ──────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,80,50 w1:p5=80,0,120,50
EOF
)
expect "1 worker: already in place" 'rename w1:p5 tieude-alpha' "$actual"

# ── 3. two workers in the CoS tab -> workers tab, left|right ─────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "2 workers: left|right" \
'place w1:p5 new-tab label=workers
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 4. three workers -> A|B top + C full width below ─────────────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
worker sude-gamma codex w1:p7 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,50,50 w1:p5=50,0,50,50 w1:p6=100,0,50,50 w1:p7=150,0,50,50
EOF
)
expect "3 workers: A|B top + C below" \
'place w1:p5 new-tab label=workers
place w1:p7 split tab=@workers dir=down anchor=w1:p5 ratio=0.5
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta
rename w1:p7 sude-gamma' "$actual"

# ── 5. four workers -> 2x2 ───────────────────────────────────────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
worker sude-gamma codex w1:p7 w1:t1 idle
worker ni-delta pi w1:p8 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,50,50 w1:p5=50,0,50,50 w1:p6=100,0,50,50 w1:p7=150,0,50,50 w1:p8=0,0,25,25
EOF
)
expect "4 workers: 2x2" \
'place w1:p5 new-tab label=workers
place w1:p7 split tab=@workers dir=down anchor=w1:p5 ratio=0.5
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
place w1:p8 split tab=@workers dir=right anchor=w1:p7 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta
rename w1:p7 sude-gamma
rename w1:p8 ni-delta' "$actual"

# ── 6. three workers already in the grid -> no moves (idempotent) ────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
worker detu-beta pi w1:p6 w1:t3 idle
worker sude-gamma codex w1:p7 w1:t3 idle
tab w1:t1 1
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,100,25 w1:p6=100,0,100,25 w1:p7=0,25,200,25
EOF
)
expect "3 workers: already in grid" \
'rename w1:p5 tieude-alpha
rename w1:p6 detu-beta
rename w1:p7 sude-gamma' "$actual"

# ── 7. 3 -> 4: the grid collapses to sude-gamma, ni-delta splits it (1 move) ─
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
worker detu-beta pi w1:p6 w1:t3 idle
worker sude-gamma codex w1:p7 w1:t3 idle
worker ni-delta pi w1:p8 w1:t1 idle
tab w1:t1 1
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,200,50 w1:p8=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,100,25 w1:p6=100,0,100,25 w1:p7=0,25,200,25
EOF
)
expect "3->4: place ni-delta right of sude-gamma" \
'place w1:p8 split tab=@workers dir=right anchor=w1:p7 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta
rename w1:p7 sude-gamma
rename w1:p8 ni-delta' "$actual"

# ── 8. 3 -> 2: tree shape changes, rebuild ───────────────────────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
worker sude-gamma codex w1:p7 w1:t3 idle
tab w1:t1 1
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,100,25 w1:p7=0,25,200,25
EOF
)
expect "3->2: rebuild" \
'evict w1:p5
evict w1:p7
place w1:p5 new-tab label=workers
place w1:p7 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p7 sude-gamma' "$actual"

# ── 9. 2 -> 3: tree shape changes, rebuild ───────────────────────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
worker detu-beta pi w1:p6 w1:t3 idle
worker sude-gamma codex w1:p7 w1:t1 idle
tab w1:t1 1
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,200,50 w1:p7=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,100,50 w1:p6=100,0,100,50
EOF
)
expect "2->3: rebuild" \
'evict w1:p5
evict w1:p6
place w1:p5 new-tab label=workers
place w1:p7 split tab=@workers dir=down anchor=w1:p5 ratio=0.5
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta
rename w1:p7 sude-gamma' "$actual"

# ── 10. back to one worker -> returns to the CoS tab ─────────────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
tab w1:t1 1
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,200,50
EOF
)
expect "back to 1 worker: return to CoS tab" \
'place w1:p5 split tab=@anchor dir=right anchor=w1:p1 ratio=0.4
rename w1:p5 tieude-alpha' "$actual"

# ── 11. working worker is never moved (warned, rest still arranged) ──────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 working
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "working worker skipped" \
'warn skip tieude-alpha: agent is working
place w1:p6 new-tab label=workers
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 12. lone working worker misplaced -> warn only ───────────────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 working
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,100,50
EOF
)
expect "working worker: no move" \
'warn skip tieude-alpha: agent is working
rename w1:p5 tieude-alpha' "$actual"

# ── 13. agents outside the CoS workspace are not workers ─────────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker truonglao-other codex w2:p9 w2:t1 idle w2
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,80,50 w1:p5=80,0,120,50
EOF
)
expect "cross-workspace agent ignored" 'rename w1:p5 tieude-alpha' "$actual"

# ── 14. more than four workers -> error exit ─────────────────────────────────
spec=$(cat <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
worker sude-gamma codex w1:p7 w1:t1 idle
worker ni-delta pi w1:p8 w1:t1 idle
worker suhuynh-eps pi w1:p9 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,200,50 w1:p5=0,0,40,50 w1:p6=40,0,40,50 w1:p7=80,0,40,50 w1:p8=120,0,40,50 w1:p9=160,0,40,50
EOF
)
mkstate <<<"$spec" > "$T/state.json"
if err=$(HERDR_STUB_STATE="$T/state.json" HERDR_PANE_ID=w1:p1 PATH="$T/bin:$PATH" \
         "$src/worker/grid-workers.sh" --dry-run 2>&1 >/dev/null); then
  fail "5 workers: expected non-zero exit"
fi
[[ "$err" == *"too many workers (5)"* ]] || fail "5 workers: missing error message (got: $err)"
echo "ok: 5 workers -> error exit"

# ── 15. no workers -> no-op ──────────────────────────────────────────────────
spec=$(cat <<'EOF'
anchor w1:p1
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,200,50
EOF
)
mkstate <<<"$spec" > "$T/state.json"
HERDR_STUB_STATE="$T/state.json" HERDR_PANE_ID=w1:p1 PATH="$T/bin:$PATH" \
  "$src/worker/grid-workers.sh" >/dev/null 2>&1 || fail "no workers: expected exit 0"
echo "ok: no workers -> no-op"

# ── 16. execution: two workers -> exact herdr commands ───────────────────────
actual=$(run <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "exec: 2 workers" \
'move w1:p5 --new-tab --label workers --no-focus
move w1:p6 --tab w1:t90 --split right --target-pane w1:p5 --ratio 0.5 --no-focus
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 17. execution: rebuild parks panes in a scratch tab first ────────────────
actual=$(run <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
worker sude-gamma codex w1:p7 w1:t3 idle
tab w1:t1 1
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,100,25 w1:p7=0,25,200,25
EOF
)
expect "exec: rebuild via scratch tab" \
'move w1:p5 --new-tab --label grid-scratch-workers --no-focus
move w1:p7 --tab w1:t91 --split right --target-pane w1:p5 --ratio 0.5 --no-focus
move w1:p5 --new-tab --label workers --no-focus
move w1:p7 --tab w1:t90 --split right --target-pane w1:p5 --ratio 0.5 --no-focus
rename w1:p5 tieude-alpha
rename w1:p7 sude-gamma' "$actual"

# ── 18. execution: one worker returns to the CoS tab at 2/5|3/5 ──────────────
actual=$(run <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
tab w1:t1 1
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,200,50
EOF
)
expect "exec: back to CoS tab" \
'move w1:p5 --tab w1:t1 --split right --target-pane w1:p1 --ratio 0.4 --no-focus
rename w1:p5 tieude-alpha' "$actual"

# ── 19. labels come from the agent's registered name, not its harness ────────
# two workers on the same harness (pi) must keep distinct labels
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "names: two pi workers keep their own names" \
'place w1:p5 new-tab label=workers
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 20. only registered worker names join the grid ───────────────────────────
# an unnamed session, a named non-worker (e.g. tldraw) and a project Lead
# (sude-lead: the <slug>-lead convention collides with the ^<kind>- worker
# pattern when the slug starts with a roster kind) in the anchor tab are
# never moved or renamed
actual=$(plan <<'EOF'
anchor w1:p1
worker - claude w1:p5 w1:t1 idle
worker tldraw claude w1:p7 w1:t1 idle
worker sude-lead claude w1:p8 w1:t1 idle
worker tieude-alpha pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 240x50 w1:p1=0,0,80,50 w1:p5=80,0,40,50 w1:p7=120,0,40,50 w1:p8=160,0,40,50 w1:p6=200,0,40,50
EOF
)
expect "names: unnamed, non-worker and <slug>-lead agents are not workers" \
'evict w1:p6
place w1:p6 split tab=@anchor dir=right anchor=w1:p1 ratio=0.4
rename w1:p6 tieude-alpha' "$actual"

# ── 21. a pane that starts working after planning stops the run ──────────────
actual=$(HERDR_STUB_FLIP_STATUS=working run_fail <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "recheck: working before the first move -> nothing moved" '' "$actual"
grep -q replan "$T/err" || fail "recheck: no replan hint on stderr"
echo "ok: recheck: replan hint on stderr"

# ── 22. a status flip after one move stops the rest of the plan ──────────────
actual=$(HERDR_STUB_FLIP_STATUS=working HERDR_STUB_FLIP_AFTER=2 run_fail <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "recheck: flip after the first move -> one move, no renames" \
'move w1:p5 --new-tab --label workers --no-focus' "$actual"

# ── 23. a move that changed nothing stops the run ────────────────────────────
actual=$(HERDR_STUB_MOVE_FAIL_AT=1 HERDR_STUB_MOVE_REASON=zoomed_tab run_fail <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "move no-op: new-tab changed=false -> stop, no renames" \
'move w1:p5 --new-tab --label workers --no-focus' "$actual"
grep -q zoomed_tab "$T/err" || fail "move no-op: reason not reported"
echo "ok: move no-op: reason reported"

# ── 24. a no-op after a successful move stops the rest of the plan ───────────
actual=$(HERDR_STUB_MOVE_FAIL_AT=2 HERDR_STUB_MOVE_REASON=same_tab run_fail <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "move no-op: split changed=false -> stop after the first move" \
'move w1:p5 --new-tab --label workers --no-focus
move w1:p6 --tab w1:t90 --split right --target-pane w1:p5 --ratio 0.5 --no-focus' "$actual"

# ── 25. the CoS tab is never reused as the workers tab ───────────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 workers
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "workers tab: CoS tab labelled workers is not reused" \
'place w1:p5 new-tab label=workers
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 26. a tab holding a non-worker is not reused ─────────────────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t2 idle
worker detu-beta pi w1:p6 w1:t3 idle
tab w1:t1 1
tab w1:t2 workers
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t2 200x50 w1:p5=0,0,100,50 w1:p9=100,0,100,50
layout w1:t3 200x50 w1:p6=0,0,100,50
EOF
)
expect "workers tab: tab with a plain shell is skipped" \
'evict w1:p6
place w1:p5 new-tab label=workers
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 27. pane ids that change on every move stay mapped to the original pane ──
actual=$(HERDR_STUB_MOVE_SUFFIX=X run <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
worker sude-gamma codex w1:p7 w1:t3 idle
tab w1:t1 1
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,100,25 w1:p7=0,25,200,25
EOF
)
expect "id churn: anchors and renames follow the returned ids" \
'move w1:p5 --new-tab --label grid-scratch-workers --no-focus
move w1:p7 --tab w1:t91 --split right --target-pane w1:p5X --ratio 0.5 --no-focus
move w1:p5X --new-tab --label workers --no-focus
move w1:p7X --tab w1:t90 --split right --target-pane w1:p5XX --ratio 0.5 --no-focus
rename w1:p5XX tieude-alpha
rename w1:p7XX sude-gamma' "$actual"

# ── 28. --tab <label>: the 2-4 worker tab uses the project label ─────────────
actual=$(plan coucou-workers <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "--tab: new tab uses the project label" \
'place w1:p5 new-tab label=coucou-workers
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 29. workers of another group in the same workspace are never touched ─────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker detu-beta pi w1:p6 w1:t1 idle
worker truonglao-other1 pi w1:p7 w1:t4 idle
worker bacdau-other2 pi w1:p8 w1:t4 idle
tab w1:t1 1
tab w1:t4 coucou-workers
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
layout w1:t4 200x50 w1:p7=0,0,100,50 w1:p8=100,0,100,50
EOF
)
expect "group: another project's workers are ignored" \
'place w1:p5 new-tab label=workers
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 30. the CoS workers tab is not part of a project group ───────────────────
actual=$(plan coucou-workers <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker sude-cos1 pi w1:p7 w1:t3 idle
tab w1:t1 1
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,80,50 w1:p5=80,0,120,50
layout w1:t3 200x50 w1:p7=0,0,200,50
EOF
)
expect "group: the CoS workers tab is ignored for a project label" 'rename w1:p5 tieude-alpha' "$actual"

# ── 31. an agent in an unrelated tab is not part of the group ────────────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker sude-stray pi w1:p6 w1:t2 idle
tab w1:t1 1
tab w1:t2 2
layout w1:t1 200x50 w1:p1=0,0,80,50 w1:p5=80,0,120,50
layout w1:t2 200x50 w1:p6=0,0,200,50
EOF
)
expect "group: agent in an unrelated tab is ignored" 'rename w1:p5 tieude-alpha' "$actual"

# ── 32. --tab: an existing project grid is reused (idempotent) ───────────────
actual=$(plan coucou-workers <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
worker detu-beta pi w1:p6 w1:t3 idle
tab w1:t1 1
tab w1:t3 coucou-workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,100,50 w1:p6=100,0,100,50
EOF
)
expect "--tab: existing project grid reused" \
'rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 33. execution: the scratch tab is per group ──────────────────────────────
actual=$(run coucou-workers <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
worker sude-gamma codex w1:p7 w1:t3 idle
tab w1:t1 1
tab w1:t3 coucou-workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,100,25 w1:p7=0,25,200,25
EOF
)
expect "exec: rebuild via the group's scratch tab" \
'move w1:p5 --new-tab --label grid-scratch-coucou-workers --no-focus
move w1:p7 --tab w1:t93 --split right --target-pane w1:p5 --ratio 0.5 --no-focus
move w1:p5 --new-tab --label coucou-workers --no-focus
move w1:p7 --tab w1:t92 --split right --target-pane w1:p5 --ratio 0.5 --no-focus
rename w1:p5 tieude-alpha
rename w1:p7 sude-gamma' "$actual"

# ── 34. a worker parked in the group's scratch tab rejoins the group ─────────
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t9 idle
worker detu-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
tab w1:t9 grid-scratch-workers
layout w1:t1 200x50 w1:p1=0,0,200,50 w1:p6=0,0,200,50
layout w1:t9 200x50 w1:p5=0,0,200,50
EOF
)
expect "scratch: parked worker rejoins the group" \
'place w1:p5 new-tab label=workers
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 35. --tab without a value (or empty) -> usage error ──────────────────────
mkstate <<'EOF' > "$T/state.json"
anchor w1:p1
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,200,50
EOF
if err=$(HERDR_STUB_STATE="$T/state.json" HERDR_PANE_ID=w1:p1 PATH="$T/bin:$PATH" \
         "$src/worker/grid-workers.sh" --tab 2>&1 >/dev/null); then
  fail "--tab without a value: expected non-zero exit"
fi
[[ "$err" == *"Usage:"* ]] || fail "--tab without a value: no usage on stderr (got: $err)"
if err=$(HERDR_STUB_STATE="$T/state.json" HERDR_PANE_ID=w1:p1 PATH="$T/bin:$PATH" \
         "$src/worker/grid-workers.sh" --tab "" 2>&1 >/dev/null); then
  fail "--tab '': expected non-zero exit"
fi
[[ "$err" == *"Usage:"* ]] || fail "--tab '': no usage on stderr (got: $err)"
echo "ok: --tab without a value -> usage error"

# ── 36. a workers tab holding a non-worker is not reused ─────────────────────
# the unnamed session stays where it is; the workers get a fresh tab
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t3 idle
worker detu-beta pi w1:p6 w1:t3 idle
worker - claude w1:p9 w1:t3 idle
tab w1:t1 1
tab w1:t3 workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t3 200x50 w1:p5=0,0,100,25 w1:p6=100,0,100,25 w1:p9=0,25,200,25
EOF
)
expect "names: workers tab with an unnamed session is not reused" \
'place w1:p5 new-tab label=workers
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 tieude-alpha
rename w1:p6 detu-beta' "$actual"

# ── 37. --only: unlisted workers still count toward the owner's population ──
# sude-cos1 is never moved or renamed, but its existence keeps sude-isl out
# of the anchor tab: population is owner-wide, so a lone listed worker of two
# goes to the project tab (a filtered run must not fake the 1-worker layout)
actual=$(plan coucou-workers sude-isl <<'EOF'
anchor w1:p1
worker sude-isl omp w1:p5 w1:t1 idle
worker sude-cos1 omp w1:p7 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,80,50 w1:p5=80,0,60,50 w1:p7=140,0,60,50
EOF
)
expect "--only: unlisted worker untouched, listed worker -> project tab" \
'place w1:p5 new-tab label=coucou-workers
rename w1:p5 sude-isl' "$actual"

# ── 38. --only: two listed workers -> the project tab, the third stays ───────
actual=$(plan coucou-workers sude-isl,quansu-islr1 <<'EOF'
anchor w1:p1
worker sude-isl omp w1:p5 w1:t1 idle
worker quansu-islr1 codex w1:p6 w1:t1 idle
worker sude-cos1 omp w1:p7 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,50,50 w1:p5=50,0,50,50 w1:p6=100,0,50,50 w1:p7=150,0,50,50
EOF
)
expect "--only: two listed workers -> project tab, third untouched" \
'place w1:p5 new-tab label=coucou-workers
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 sude-isl
rename w1:p6 quansu-islr1' "$actual"

# ── 39. --only names nobody -> no-op ────────────────────────────────────────
mkstate <<'EOF' > "$T/state.json"
anchor w1:p1
worker sude-cos1 omp w1:p5 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,100,50
EOF
HERDR_STUB_STATE="$T/state.json" HERDR_PANE_ID=w1:p1 PATH="$T/bin:$PATH" \
  "$src/worker/grid-workers.sh" --only sude-ghost >/dev/null 2>&1 \
  || fail "--only ghost: expected exit 0"
echo "ok: --only matching no agents -> no-op"

# ── 40. --only: a group tab holding an unlisted group member is reused ──────
# sude-cos1 belongs to the same coucou-workers group but is not in --only:
# it is a legitimate occupant — a second pipeline must not force a duplicate
# project tab — and it is never moved or renamed itself
actual=$(plan coucou-workers sude-isl,quansu-islr1 <<'EOF'
anchor w1:p1
worker sude-isl omp w1:p5 w1:t1 idle
worker quansu-islr1 codex w1:p6 w1:t1 idle
worker sude-cos1 omp w1:p7 w1:t4 idle
tab w1:t1 1
tab w1:t4 coucou-workers
layout w1:t1 200x50 w1:p1=0,0,50,50 w1:p5=50,0,50,50 w1:p6=100,0,50,50
layout w1:t4 200x50 w1:p7=0,0,200,50
EOF
)
expect "--only: same-group unlisted occupant keeps the tab reusable" \
'place w1:p5 split tab=@workers dir=right anchor=w1:p7 ratio=0.5
place w1:p6 split tab=@workers dir=right anchor=w1:p5 ratio=0.5
rename w1:p5 sude-isl
rename w1:p6 quansu-islr1' "$actual"

# ── 41. --only without a value -> usage error ───────────────────────────────
mkstate <<'EOF' > "$T/state.json"
anchor w1:p1
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,200,50
EOF
if err=$(HERDR_STUB_STATE="$T/state.json" HERDR_PANE_ID=w1:p1 PATH="$T/bin:$PATH" \
         "$src/worker/grid-workers.sh" --only 2>&1 >/dev/null); then
  fail "--only without a value: expected non-zero exit"
fi
[[ "$err" == *"Usage:"* ]] || fail "--only without a value: no usage on stderr (got: $err)"
echo "ok: --only without a value -> usage error"

# owner/group cases: each gets a fresh runs dir
runs_case() { export QUAQUA_RUNS_DIR="$T/runs-$1"; mkdir -p "$QUAQUA_RUNS_DIR"; }

# ── 42. another owner's worker in the anchor tab is never touched ────────────
runs_case 42
echo baroiplayer-lead > "$QUAQUA_RUNS_DIR/sude-bpga.owner"
actual=$(plan <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker sude-bpga pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,80,50 w1:p5=80,0,60,50 w1:p6=140,0,60,50
EOF
)
expect "owner: a foreign-owner worker is ignored, mine stays beside the anchor" \
'evict w1:p5
place w1:p5 split tab=@anchor dir=right anchor=w1:p1 ratio=0.4
rename w1:p5 tieude-alpha' "$actual"

# ── 43. one owner, two projects -> one tab per project, even a lone worker ───
runs_case 43
echo mono-workers > "$QUAQUA_RUNS_DIR/tieude-alpha.grid"
echo bp-workers > "$QUAQUA_RUNS_DIR/sude-beta.grid"
actual=$(run <<'EOF'
anchor w1:p1
worker tieude-alpha pi w1:p5 w1:t1 idle
worker sude-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,100,50 w1:p5=100,0,50,50 w1:p6=150,0,50,50
EOF
)
expect "groups: each project gets its own tab" \
'move w1:p6 --new-tab --label bp-workers --no-focus
rename w1:p6 sude-beta
move w1:p5 --new-tab --label mono-workers --no-focus
rename w1:p5 tieude-alpha' "$actual"

# ── 44. the owner's only worker, stranded in its project tab, comes back ─────
runs_case 44
echo mono-workers > "$QUAQUA_RUNS_DIR/suhuynh-x.grid"
actual=$(plan bp-workers <<'EOF'
anchor w1:p1
worker suhuynh-x claude w1:p5 w1:t11 idle
tab w1:t1 1
tab w1:t11 mono-workers
layout w1:t1 200x50 w1:p1=0,0,200,50
layout w1:t11 200x50 w1:p5=0,0,200,50
EOF
)
expect "groups: a lone stranded worker returns beside the anchor" \
'place w1:p5 split tab=@anchor dir=right anchor=w1:p1 ratio=0.4
rename w1:p5 suhuynh-x' "$actual"

# ── 45. --only: a worker in another project group keeps the listed worker ────
# out of the anchor tab — the pipeline scenario: the owner runs a second
# project's worker beside this one; population and groups are owner-wide
runs_case 45
echo mono-workers > "$QUAQUA_RUNS_DIR/sude-alpha.grid"
echo other-workers > "$QUAQUA_RUNS_DIR/sude-beta.grid"
actual=$(plan mono-workers sude-alpha <<'EOF'
anchor w1:p1
worker sude-alpha pi w1:p5 w1:t1 idle
worker sude-beta pi w1:p6 w1:t1 idle
tab w1:t1 1
layout w1:t1 200x50 w1:p1=0,0,80,50 w1:p5=80,0,60,50 w1:p6=140,0,60,50
EOF
)
expect "--only: other-group co-owner -> project tab, other group untouched" \
'place w1:p5 new-tab label=mono-workers
rename w1:p5 sude-alpha' "$actual"

# ── 46. --only: a tab holding a different group's worker is not reused ───────
# sude-beta sits in the mono-workers tab but its .grid says other-workers —
# foreign to this group, so the tab must not be adopted and it is untouched
runs_case 46
echo other-workers > "$QUAQUA_RUNS_DIR/sude-beta.grid"
actual=$(plan mono-workers sude-alpha <<'EOF'
anchor w1:p1
worker sude-alpha pi w1:p5 w1:t1 idle
worker sude-beta pi w1:p6 w1:t4 idle
tab w1:t1 1
tab w1:t4 mono-workers
layout w1:t1 200x50 w1:p1=0,0,80,50 w1:p5=80,0,120,50
layout w1:t4 200x50 w1:p6=0,0,200,50
EOF
)
expect "--only: tab with a foreign-group occupant is not reused" \
'place w1:p5 new-tab label=mono-workers
rename w1:p5 sude-alpha' "$actual"

echo "PASS"
