#!/usr/bin/env bash
# Verifies that when QUAQUA_TEST=1 is set, a test suite that forgets to set HOME
# or QUAQUA_RUNS_DIR cannot write to the real live workspace DB (~/quaqua/runs/quaqua.db).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
bin="${QUAQUA_BIN:-$root/target/debug/quaqua}"
[[ -x "$bin" ]] || { echo "no quaqua binary at $bin" >&2; exit 1; }

T=$(mktemp -d "${TMPDIR:-/tmp}/test-live-db-guard.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

real_db="$HOME/quaqua/runs/quaqua.db"
before_count=0
if [[ -f "$real_db" ]]; then
  before_count=$(sqlite3 "$real_db" 'select count(*) from farm_events;')
fi

mkdir -p "$T/bin"
cat > "$T/bin/herdr" <<'EOF'
#!/usr/bin/env bash
if [[ "$1 $2" == "agent get" ]]; then
  printf '{"result":{"agent":{"agent_status":"idle","pane_id":"%%p"}}}\n'
  exit 0
fi
if [[ "$1 $2" == "agent prompt" ]]; then
  exit 0
fi
if [[ "$1 $2" == "pane read" ]]; then
  printf 'done\n'
  exit 0
fi
exit 0
EOF
chmod +x "$T/bin/herdr"

# Simulate a test that forgot to set HOME and QUAQUA_RUNS_DIR, running with QUAQUA_TEST=1
export PATH="$T/bin:$PATH"
export HERDR_BIN_PATH="$T/bin/herdr"
export QUAQUA_TEST=1
unset QUAQUA_RUNS_DIR QUAQUA_HOME QUAQUA_ROOT QUAQUA_OWNER HERDR_PANE_ID

# Run lead prompt (which calls shadow gate)
out="$("$bin" lead prompt baro testmsg 2>&1 || true)"

if [[ -f "$real_db" ]]; then
  after_count=$(sqlite3 "$real_db" 'select count(*) from farm_events;')
  [[ "$before_count" -eq "$after_count" ]] || fail "live DB row count changed from $before_count to $after_count!"
fi

echo "ok: suite forgetting HOME with QUAQUA_TEST=1 did not touch live DB"
echo "PASS"
