#!/usr/bin/env bash
# Sandbox test for worker cap enforcement:
# 1. Cap refusal: owner with 8 live task workers refuses the 9th with a clear error.
# 2. Owner isolation: Lead's workers do not count against cos (and vice-versa).
# 3. Pipeline counts as 2:
#    - an active pipeline + 6 standalone workers reaches cap (7th standalone refused).
#    - an owner with 7 standalone workers cannot start a pipeline (exceeds cap of 8).
# 4. Project Leads (<slug>-lead) do not count toward the worker cap.
# 5. Rename leaves no dangling refs (legacy role).
# Usage: tests/test_worker_cap.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
root="$(cd "$here/.." && pwd)"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-worker-cap.XXXXXX")
T=$(cd "$T" && pwd -P)
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

mkdir -p "$T/bin" "$T/repo" "$T/home/quaqua/runs" "$T/home/quaqua/rules/_all"
printf '# rule\n- test rule\n' > "$T/home/quaqua/rules/_all/test.md"

git -C "$T/repo" init -q
git -C "$T/repo" -c user.email=t@t -c user.name=t commit -qm init --allow-empty

# Stub git and herdr
cat > "$T/bin/git" <<'STUB'
#!/usr/bin/env bash
case " $* " in
  *"worktree add"*)
    wt=""
    prev=""
    for a in "$@"; do
      if [[ "$prev" == "-b" ]]; then :
      elif [[ "$a" == /* ]]; then wt="$a"; fi
      prev="$a"
    done
    mkdir -p "$wt"
    exit 0 ;;
  *"worktree list"*)
    for d in "$T"/.wt/*; do
      [[ -d "$d" ]] && echo "worktree $d"
    done
    exit 0 ;;
esac
exit 0
STUB
chmod +x "$T/bin/git"

cat > "$T/bin/herdr" <<STUB
#!/usr/bin/env bash
case "\${1:-} \${2:-}" in
  "agent list")
    cat "\${HERDR_AGENTS_FILE:-$T/agents.json}" 2>/dev/null || echo '{"result":{"agents":[]}}'
    exit 0 ;;
  "pane split"*)
    echo '{"result":{"pane":{"pane_id":"%1"}}}'
    exit 0 ;;
  "agent start"*)
    exit 0 ;;
esac
exit 0
STUB
chmod +x "$T/bin/herdr"
export HERDR_BIN_PATH="$T/bin/herdr"
export HERDR_SOCKET_PATH="$T/herdr.sock"
export PATH="$T/bin:$PATH"

spawn() {
  local name=$1 kind=$2; shift 2
  rc=0
  HOME="$T/home" PATH="$T/bin:$PATH" HERDR_BIN_PATH="$T/bin/herdr" HERDR_SOCKET_PATH="$T/herdr.sock" QUAQUA_TEST=1 GRID_SKIP=1 \
    env -u QUAQUA_OWNER "$@" "${QUAQUA_BIN:-quaqua}" worker spawn "$name" "$kind" "$T/repo" >"$T/out" 2>"$T/err" || rc=$?
}

# ── 1. Cap refusal: 8 live workers for cos -> 9th refused ───────────────────
cat > "$T/agents.json" <<'JSON'
{"result":{"agents":[
  {"name":"sude-w1","agent_status":"idle","pane_id":"p1"},
  {"name":"sude-w2","agent_status":"idle","pane_id":"p2"},
  {"name":"sude-w3","agent_status":"idle","pane_id":"p3"},
  {"name":"sude-w4","agent_status":"idle","pane_id":"p4"},
  {"name":"sude-w5","agent_status":"idle","pane_id":"p5"},
  {"name":"sude-w6","agent_status":"idle","pane_id":"p6"},
  {"name":"sude-w7","agent_status":"idle","pane_id":"p7"},
  {"name":"sude-w8","agent_status":"idle","pane_id":"p8"}
]}}
JSON
for i in 1 2 3 4 5 6 7 8; do
  echo "cos" > "$T/home/quaqua/runs/sude-w$i.owner"
done

spawn sude-w9 sude
[[ $rc -eq 1 ]] || fail "cap refusal: exit $rc, want 1"
grep -q 'worker cap exceeded' "$T/err" || fail "cap refusal: missing error message: $(cat "$T/err")"
grep -q '8 live worker slots for owner .cos.' "$T/err" || fail "cap refusal: owner not cited: $(cat "$T/err")"
echo "ok: cap refusal -> 9th worker refused when cos has 8 live workers"

# ── 2. Owner isolation: cos has 8, but coucou-lead can spawn ────────────────
spawn sude-leadw1 sude QUAQUA_OWNER=coucou-lead
[[ $rc -eq 0 ]] || fail "owner isolation (lead): exit $rc, want 0: $(cat "$T/err")"
[[ -f "$T/home/quaqua/runs/sude-leadw1.owner" ]] || fail "owner file missing"
[[ "$(cat "$T/home/quaqua/runs/sude-leadw1.owner")" == "coucou-lead" ]] || fail "wrong owner recorded"
echo "ok: owner isolation -> Lead can spawn despite cos being at cap"

# And lead at cap does not block cos
cat > "$T/agents.json" <<'JSON'
{"result":{"agents":[
  {"name":"sude-lw1","agent_status":"idle","pane_id":"p1"},
  {"name":"sude-lw2","agent_status":"idle","pane_id":"p2"},
  {"name":"sude-lw3","agent_status":"idle","pane_id":"p3"},
  {"name":"sude-lw4","agent_status":"idle","pane_id":"p4"},
  {"name":"sude-lw5","agent_status":"idle","pane_id":"p5"},
  {"name":"sude-lw6","agent_status":"idle","pane_id":"p6"},
  {"name":"sude-lw7","agent_status":"idle","pane_id":"p7"},
  {"name":"sude-lw8","agent_status":"idle","pane_id":"p8"}
]}}
JSON
for i in 1 2 3 4 5 6 7 8; do
  echo "coucou-lead" > "$T/home/quaqua/runs/sude-lw$i.owner"
done
spawn sude-cosok sude QUAQUA_OWNER=cos
[[ $rc -eq 0 ]] || fail "owner isolation (cos): exit $rc, want 0: $(cat "$T/err")"
echo "ok: owner isolation -> cos can spawn despite Lead being at cap"

# ── 3. Leads do not count toward the cap ─────────────────────────────────────
cat > "$T/agents.json" <<'JSON'
{"result":{"agents":[
  {"name":"coucou-lead","agent_status":"idle","pane_id":"p0"},
  {"name":"mono-lead","agent_status":"idle","pane_id":"p00"},
  {"name":"sude-w1","agent_status":"idle","pane_id":"p1"},
  {"name":"sude-w2","agent_status":"idle","pane_id":"p2"},
  {"name":"sude-w3","agent_status":"idle","pane_id":"p3"},
  {"name":"sude-w4","agent_status":"idle","pane_id":"p4"},
  {"name":"sude-w5","agent_status":"idle","pane_id":"p5"},
  {"name":"sude-w6","agent_status":"idle","pane_id":"p6"},
  {"name":"sude-w7","agent_status":"idle","pane_id":"p7"}
]}}
JSON
for i in 1 2 3 4 5 6 7; do
  echo "cos" > "$T/home/quaqua/runs/sude-w$i.owner"
done
# 7 task workers + 2 leads: cos has 7 slots used, can still spawn 8th worker
spawn sude-w8 sude QUAQUA_OWNER=cos
[[ $rc -eq 0 ]] || fail "leads exemption: exit $rc, want 0: $(cat "$T/err")"
echo "ok: leads do not count toward the worker cap"

# ── 4. Pipeline counts as 2: 1 pipeline + 6 workers reaches cap ──────────────
cat > "$T/agents.json" <<'JSON'
{"result":{"agents":[
  {"name":"sude-mytest","agent_status":"idle","pane_id":"p1"},
  {"name":"sude-s1","agent_status":"idle","pane_id":"p2"},
  {"name":"sude-s2","agent_status":"idle","pane_id":"p3"},
  {"name":"sude-s3","agent_status":"idle","pane_id":"p4"},
  {"name":"sude-s4","agent_status":"idle","pane_id":"p5"},
  {"name":"sude-s5","agent_status":"idle","pane_id":"p6"},
  {"name":"sude-s6","agent_status":"idle","pane_id":"p7"}
]}}
JSON
echo "cos" > "$T/home/quaqua/runs/sude-mytest.owner"
echo "mytest" > "$T/home/quaqua/runs/sude-mytest.pipeline"
for i in 1 2 3 4 5 6; do
  echo "cos" > "$T/home/quaqua/runs/sude-s$i.owner"
done
cat > "$T/home/quaqua/runs/pipeline-mytest.state" <<'STATE'
task=mytest
builder=sude
lead=cos
finished=0
pipeline_pid=999999
STATE
# 1 pipeline (reserves 2) + 6 standalone = 8 slots. Attempting 7th standalone fails.
spawn sude-s7 sude QUAQUA_OWNER=cos
[[ $rc -eq 1 ]] || fail "pipeline reserves 2: exit $rc, want 1"
grep -q 'worker cap exceeded' "$T/err" || fail "pipeline reserves 2: missing error message: $(cat "$T/err")"
echo "ok: pipeline counts as 2 -> 1 active pipeline + 6 standalone workers reaches cap (7th standalone refused)"
# ── 5. Pipeline starting refusal: 7 standalone workers -> pipeline refuses ───
cat > "$T/agents.json" <<'JSON'
{"result":{"agents":[
  {"name":"sude-w1","agent_status":"idle","pane_id":"p1"},
  {"name":"sude-w2","agent_status":"idle","pane_id":"p2"},
  {"name":"sude-w3","agent_status":"idle","pane_id":"p3"},
  {"name":"sude-w4","agent_status":"idle","pane_id":"p4"},
  {"name":"sude-w5","agent_status":"idle","pane_id":"p5"},
  {"name":"sude-w6","agent_status":"idle","pane_id":"p6"},
  {"name":"sude-w7","agent_status":"idle","pane_id":"p7"}
]}}
JSON
for i in 1 2 3 4 5 6 7; do
  echo "cos" > "$T/home/quaqua/runs/sude-w$i.owner"
done
rm -f "$T/home/quaqua/runs"/pipeline-*.state

brief="$T/brief.md"
cat > "$brief" <<'EOF'
Goal: test pipeline cap
Hard constraints: none
Current design: test
EOF
prc=0
HOME="$T/home" PATH="$T/bin:$PATH" HERDR_BIN_PATH="$T/bin/herdr" HERDR_SOCKET_PATH="$T/herdr.sock" GRID_SKIP=1 QUAQUA_OWNER=cos \
  "${QUAQUA_BIN:-quaqua}" pipeline pipetest "$T/repo" --brief "$brief" >"$T/out" 2>"$T/err" || prc=$?
[[ $prc -eq 2 ]] || fail "pipeline cap refusal: exit $prc, want 2: $(cat "$T/err")"
grep -q 'worker cap exceeded' "$T/err" || fail "pipeline cap refusal: missing error message: $(cat "$T/err")"
echo "ok: pipeline counts as 2 -> owner with 7 workers cannot start pipeline (needs 2, cap is 8)"
# ── 6. Dangling legacy references check ──────────────────────────────────────
# No file in the repo (except AGENTS.md which CoS owns and git history) should mention the old name.
# memory/ is tracked in this repo (upstream gitignores it): dated logs record the
# old name as history — excluding them is intentional, not a gap in the check.
legacy_pat="super""visor"
dangling=$(rg "$legacy_pat" "$root" --glob '!templates/workspace/AGENTS.md' --glob '!tests/test_*.sh' --glob '!memory/' || true)
[[ -z "$dangling" ]] || fail "dangling legacy references found:\n$dangling"
echo "ok: rename leaves no dangling refs ($legacy_pat)"

echo "PASS"
