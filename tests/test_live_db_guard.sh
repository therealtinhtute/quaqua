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

file_stat() {
  if [[ -e "$1" ]]; then
    stat -c "%Y %s" "$1" 2>/dev/null || stat -f "%m %z" "$1" 2>/dev/null || true
  fi
}

real_db="$HOME/quaqua/runs/quaqua.db"
before_stat="$(file_stat "$real_db")"
before_wal_stat="$(file_stat "$real_db-wal")"
before_shm_stat="$(file_stat "$real_db-shm")"
before_jrn_stat="$(file_stat "$real_db-journal")"

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

after_stat="$(file_stat "$real_db")"
[[ "$before_stat" == "$after_stat" ]] || fail "live DB was modified! stat changed from '$before_stat' to '$after_stat'"
[[ "$(file_stat "$real_db-wal")" == "$before_wal_stat" ]] || fail "live DB wal file modified!"
[[ "$(file_stat "$real_db-shm")" == "$before_shm_stat" ]] || fail "live DB shm file modified!"
[[ "$(file_stat "$real_db-journal")" == "$before_jrn_stat" ]] || fail "live DB journal file modified!"
[[ "$out" == *"would_deny:"* ]] || fail "expected would_deny in lead prompt output, got: $out"

echo "ok: suite forgetting HOME with QUAQUA_TEST=1 did not touch live DB"
echo "PASS"
