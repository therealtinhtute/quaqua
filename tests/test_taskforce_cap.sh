#!/usr/bin/env bash
# Sandbox test for taskforce weight 2 in cap:
# 1. Active dual-lens taskforce reserves 2 slots in worker cap (weights["dual-lens"] = 2).
# 2. Combined live workers and reservation equals 2.
# 3. Cap enforcement: 1 dual-lens (2 slots) + 2 standalone workers reaches cap (5th refused).
# 4. Starting refusal: owner with 3 workers cannot start dual-lens (needs 2 slots, cap is 4).
# 5. Finished dual-lens releases its slot reservation.
# Usage: tests/test_taskforce_cap.sh   (exit 0 = pass)
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

T=$(mktemp -d "${TMPDIR:-/tmp}/test-taskforce-cap.XXXXXX")
T=$(cd "$T" && pwd -P)
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

mkdir -p "$T/bin" "$T/repo" "$T/home/quaqua/runs" "$T/home/quaqua/rules/_all"
cp "$repo/team.toml" "$T/team.toml"
export QUAQUA_ROOT="$T"
export QUAQUA_TEAM_TOML="$T/team.toml"
export QUAQUA_OWNER=cos
export HOME="$T/home"
runs_dir="$T/home/quaqua/runs"
mkdir -p "$T/home/quaqua/rules/_all"
printf '# rule\n- test rule\n' > "$T/home/quaqua/rules/_all/test.md"
git -C "$T/repo" init -q
git -C "$T/repo" -c user.email=t@t -c user.name=t commit -qm init --allow-empty

cat > "$T/bin/git" << 'STUB'
#!/usr/bin/env bash
case " $* " in
  *"status --porcelain"*) exit 0 ;;
  *"rev-parse"*) echo "c0ffee" ;;
  *) exit 0 ;;
esac
STUB
chmod +x "$T/bin/git"

cat > "$T/bin/herdr" << 'STUB'
#!/usr/bin/env bash
case "${1:-} ${2:-}" in
  "agent list")
    cat "$AGENT_JSON" ;;
  "agent get")
    printf '{"error":{"code":"agent_not_found"}}\n' >&2; exit 1 ;;
  "pane split")
    printf '{"result":{"pane":{"pane_id":"%%1"}}}\n' ;;
  *) exit 0 ;;
esac
STUB
chmod +x "$T/bin/herdr"

export PATH="$T/bin:$PATH"
export AGENT_JSON="$T/agents.json"
echo '{"result":{"agents":[]}}' > "$AGENT_JSON"

# ── 1. Active dual-lens with 0 workers -> reserves 2 slots ────────────────────────
cat > "$runs_dir/taskforce-tf1.state" << STATE
task=tf1
kind=dual-lens
owner=cos
finished=0
taskforce_pid=$$
STATE

cap_out=$("$bin" cap cos)
[[ "$cap_out" -eq 2 ]] || fail "dual-lens reservation: want 2 slots, got $cap_out"
echo "ok: active dual-lens with 0 workers reserves 2 slots"

# ── 2. Active dual-lens with 1 live worker -> 1 live + 1 reserved = 2 slots ───────
cat > "$AGENT_JSON" << 'JSON'
{"result":{"agents":[
  {"name":"truonglao-tf1dl1","agent_status":"working"}
]}}
JSON
echo "cos" > "$runs_dir/truonglao-tf1dl1.owner"
echo "tf1" > "$runs_dir/truonglao-tf1dl1.taskforce"

cap_out=$("$bin" cap cos)
[[ "$cap_out" -eq 2 ]] || fail "dual-lens with 1 worker: want 2 slots, got $cap_out"
echo "ok: dual-lens with 1 live worker still accounts for 2 slots (1 live + 1 reserved)"

# ── 3. Active dual-lens with 2 live workers -> 2 live = 2 slots ───────────────────
cat > "$AGENT_JSON" << 'JSON'
{"result":{"agents":[
  {"name":"truonglao-tf1dl1","agent_status":"working"},
  {"name":"suhuynh-tf1dl2","agent_status":"working"}
]}}
JSON
echo "cos" > "$runs_dir/suhuynh-tf1dl2.owner"
echo "tf1" > "$runs_dir/suhuynh-tf1dl2.taskforce"

cap_out=$("$bin" cap cos)
[[ "$cap_out" -eq 2 ]] || fail "dual-lens with 2 workers: want 2 slots, got $cap_out"
echo "ok: dual-lens with 2 live workers accounts for 2 slots"

# ── 4. Cap reached: dual-lens (2 slots) + 2 standalone workers = 4 slots ──────────
cat > "$AGENT_JSON" << 'JSON'
{"result":{"agents":[
  {"name":"truonglao-tf1dl1","agent_status":"working"},
  {"name":"suhuynh-tf1dl2","agent_status":"working"},
  {"name":"sude-w1","agent_status":"working"},
  {"name":"sude-w2","agent_status":"working"}
]}}
JSON
echo "cos" > "$runs_dir/sude-w1.owner"
echo "cos" > "$runs_dir/sude-w2.owner"

cap_out=$("$bin" cap cos)
[[ "$cap_out" -eq 4 ]] || fail "cap at limit: want 4 slots, got $cap_out"

# Attempting 5th worker spawn is refused
rc=0
"$bin" worker spawn sude-w3 sude "$T/repo" >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 1 ]] || fail "5th worker should be refused: exit $rc, want 1"
grep -q "worker cap exceeded" "$T/err" || fail "missing worker cap exceeded error: $(cat "$T/err")"
echo "ok: dual-lens (2 slots) + 2 standalone = 4 slots; 5th worker refused"

# ── 5. Starting refusal: owner with 3 workers cannot start dual-lens ───────────────
cat > "$AGENT_JSON" << 'JSON'
{"result":{"agents":[
  {"name":"sude-w1","agent_status":"working"},
  {"name":"sude-w2","agent_status":"working"},
  {"name":"sude-w3","agent_status":"working"}
]}}
JSON
echo "cos" > "$runs_dir/sude-w3.owner"
rm -f "$runs_dir"/taskforce-*.state

cat > "$T/brief.md" << 'EOF'
Goal: test taskforce cap
Hard constraints: none
Current design: direct
EOF

rc=0
"$bin" taskforce dual-lens testtf "$T/repo" --brief "$T/brief.md" >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 2 ]] || fail "taskforce start over cap: exit $rc, want 2"
grep -q "worker cap exceeded" "$T/err" || fail "missing cap error on taskforce start: $(cat "$T/err")"
echo "ok: owner with 3 workers cannot start dual-lens (needs 2, cap is 4)"

# ── 6. Finished dual-lens releases reservation ────────────────────────────────────
cat > "$AGENT_JSON" << 'JSON'
{"result":{"agents":[]}}
JSON
rm -f "$runs_dir"/*.owner "$runs_dir"/*.taskforce

cat > "$runs_dir/taskforce-tf1.state" << STATE
task=tf1
kind=dual-lens
owner=cos
finished=1
taskforce_pid=$$
STATE

cap_out=$("$bin" cap cos)
[[ "$cap_out" -eq 0 ]] || fail "finished dual-lens: want 0 slots, got $cap_out"
echo "ok: finished dual-lens releases slot reservation (0 slots used)"

echo "PASS"
