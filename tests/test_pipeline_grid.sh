#!/usr/bin/env bash
# Sandbox test for pipeline grid tab separation (fixing bash bug 2026-10-15):
# 1. Pipeline builder + reviewer (2 workers) land in GRID_TAB (workers tab), never the Lead tab.
# 2. The Lead pane remains alone in the Lead tab.
# 3. quaqua grid plan for a team with 2 workers assigns them to the workers tab, not the anchor tab.
# Usage: tests/test_pipeline_grid.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/.." && pwd)"
bin="${QUAQUA_BIN:-$repo/target/debug/quaqua}"
if [[ ! -x "$bin" ]]; then
  if [[ -f "$repo/crates/quaqua/Cargo.toml" ]]; then
    cargo build --locked --manifest-path "$repo/crates/quaqua/Cargo.toml"
    bin="$repo/target/debug/quaqua"
  elif command -v quaqua >/dev/null 2>&1; then
    bin="$(command -v quaqua)"
  fi
fi
[[ -x "$bin" ]] || { echo "error: binary not found at $bin" >&2; exit 1; }

export T=$(mktemp -d "${TMPDIR:-/tmp}/test-pipeline-grid.XXXXXX")
export T=$(cd "$T" && pwd -P)
fail() { echo "FAIL: $*" >&2; exit 1; }

mkdir -p "$T/bin" "$T/repo" "$T/home/quaqua/runs" "$T/home/quaqua/rules/_all"
cp "$repo/team.toml" "$T/team.toml"
export QUAQUA_ROOT="$T"
export QUAQUA_TEAM_TOML="$T/team.toml"
export HOME="$T/home"
export QUAQUA_OWNER="mylead-lead"
export GRID_TAB="myrepo-workers"

mkdir -p "$T/home/quaqua/rules/_all"
printf '# rule\n- test rule\n' > "$T/home/quaqua/rules/_all/test.md"
git -C "$T/repo" init -q
git -C "$T/repo" -c user.email=t@t -c user.name=t commit -qm init --allow-empty

cat > "$T/brief.md" << 'EOF'
Goal: test pipeline grid tab placement
Hard constraints: none
Current design: direct
EOF

export CALLS="$T/calls"
: > "$CALLS"
cat > "$T/bin/git" << 'STUB'
#!/usr/bin/env bash
[[ -n ${CALLS:-} ]] && echo "git $*" >> "$CALLS"
case " $* " in
  *"status --porcelain"*) exit 0 ;;
  *"diff --numstat"*) printf '200\t10\tsrc/a.c\n' ;;
  *"diff --name-only"*) printf 'src/a.c\n' ;;
  *"rev-parse HEAD"*)
    cf="$T/git_call_count"
    c=$(cat "$cf" 2>/dev/null || echo 0)
    c=$(( c + 1 ))
    echo "$c" > "$cf"
    if (( c <= 1 )); then
      echo "ba5eba5000000000000000000000000000000000"
    else
      echo "c0ffee1deadbeef0000000000000000000000000"
    fi
    ;;
  *) exit 0 ;;
esac
STUB
chmod +x "$T/bin/git"

cat > "$T/bin/herdr" << 'STUB'
#!/usr/bin/env bash
[[ -n ${CALLS:-} ]] && echo "herdr $*" >> "$CALLS"
case "${1:-} ${2:-}" in
  "agent get")
    printf '{"result":{"agent":{"agent_status":"working","pane_id":"%%1"}}}\n' ;;
  "pane split")
    printf '{"result":{"pane":{"pane_id":"%%1"}}}\n' ;;
  "agent prompt")
    w=$3
    case "$4" in
      *"phase: PLAN"*)
        printf 'plan\n\n## Lessons\n- none\n' > "$HOME/quaqua/runs/$w.md" ;;
      *"phase: BUILD"*)
        printf 'build\n\n## Lessons\n- none\n' > "$HOME/quaqua/runs/$w.md" ;;
      *"phase: REVIEW"*)
        printf 'Verdict: clean\n' > "$HOME/quaqua/runs/$w.md" ;;
    esac
    exit 0 ;;
  *) exit 0 ;;
esac
STUB
chmod +x "$T/bin/herdr"

cat > "$T/bin/grid-workers.sh" << 'STUB'
#!/usr/bin/env bash
[[ -n ${CALLS:-} ]] && echo "grid $*" >> "$CALLS"
exit 0
STUB
chmod +x "$T/bin/grid-workers.sh"

cat > "$T/bin/spawn-worker.sh" << 'STUB'
#!/usr/bin/env bash
[[ -n ${CALLS:-} ]] && echo "spawn $*" >> "$CALLS"
exit 0
STUB
chmod +x "$T/bin/spawn-worker.sh"

cat > "$T/bin/prompt-worker.sh" << 'STUB'
#!/usr/bin/env bash
[[ -n ${CALLS:-} ]] && echo "prompt $*" >> "$CALLS"
if [[ "$1" == "--resume" ]]; then
  w=$2
else
  w=$1
fi
printf 'report\n\n## Lessons\n- none\nVerdict: clean\n' > "$HOME/quaqua/runs/$w.md"
exit 0
STUB
chmod +x "$T/bin/prompt-worker.sh"

cat > "$T/bin/cleanup-worker.sh" << 'STUB'
#!/usr/bin/env bash
[[ -n ${CALLS:-} ]] && echo "cleanup $*" >> "$CALLS"
exit 0
STUB
chmod +x "$T/bin/cleanup-worker.sh"

export PATH="$T/bin:$PATH"
export QUAQUA_SCRIPT_DIR="$T/bin"

# ── 1. Pipeline execution: builder + reviewer gridded into GRID_TAB ──────────────
rc=0
"$bin" pipeline pgrid "$T/repo" --brief "$T/brief.md" >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 5 ]] || fail "run1 (plan-ready): exit $rc, want 5: $(cat "$T/err")"

# Resume to finish build + review
rc=0
"$bin" pipeline --resume pgrid "$T/repo" >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 0 ]] || fail "resume: exit $rc, want 0: $(cat "$T/err")"

# Check grid calls recorded
glines=$(grep '^grid ' "$CALLS")
echo "$glines" | grep -q 'grid --check --tab myrepo-workers --only sude-pgrid' \
  || fail "builder grid missing: $glines"
echo "$glines" | grep -q 'grid --check --tab myrepo-workers --only sude-pgrid,quansu-pgridr1' \
  || fail "reviewer+builder grid missing: $glines"

# ── 2. Pure grid planner verification: 2 workers placed in workers tab, not anchor tab ─
# Snapshot representing:
# Tab t1: Lead pane (%lead) tagged role=orchestrator, owner=mylead-lead
# Tab t2: label="myrepo-workers" (workers tab)
# Panes %w1 (sude-pgrid) and %w2 (quansu-pgridr1) tagged role=worker, owner=mylead-lead, group=myrepo
cat > "$T/snapshot.json" << 'JSON'
{
  "tabs": [
    {"tab_id": "t1", "label": "lead-tab", "workspace_id": "ws1"},
    {"tab_id": "t2", "label": "myrepo-workers", "workspace_id": "ws1"}
  ],
  "panes": [
    {
      "pane_id": "%lead",
      "tab_id": "t1",
      "workspace_id": "ws1",
      "tokens": {
        "grid_role": "orchestrator",
        "grid_owner": "mylead-lead"
      }
    },
    {
      "pane_id": "%w1",
      "tab_id": "t1",
      "workspace_id": "ws1",
      "agent_status": "working",
      "tokens": {
        "grid_role": "worker",
        "grid_owner": "mylead-lead",
        "grid_group": "myrepo",
        "grid_ready": "1"
      }
    },
    {
      "pane_id": "%w2",
      "tab_id": "t1",
      "workspace_id": "ws1",
      "agent_status": "working",
      "tokens": {
        "grid_role": "worker",
        "grid_owner": "mylead-lead",
        "grid_group": "myrepo",
        "grid_ready": "1"
      }
    }
  ],
  "layouts": [
    {
      "tab_id": "t1",
      "panes": [{"pane_id": "%lead"}, {"pane_id": "%w1"}, {"pane_id": "%w2"}],
      "area": {"x": 0, "y": 0, "width": 200, "height": 50}
    },
    {
      "tab_id": "t2",
      "panes": [],
      "area": {"x": 0, "y": 0, "width": 200, "height": 50}
    }
  ],
  "agents": [
    {"name": "mylead-lead", "pane_id": "%lead"}
  ]
}
JSON

plan_out=$("$bin" grid plan --file "$T/snapshot.json")
echo "Plan output:"
echo "$plan_out"

# Every move operation for the workers must target the workers tab (new_tab/split into workers), NEVER tab "t1" (the Lead tab)
echo "$plan_out" | grep -q 'myrepo' || fail "workers not planned into workers group: $plan_out"
! echo "$plan_out" | grep -q '"tab":"t1"' || fail "worker planned into Lead tab t1: $plan_out"
echo "ok: 2 workers planned to land in GRID_TAB (myrepo), never the Lead tab (t1)"

echo "PASS"
