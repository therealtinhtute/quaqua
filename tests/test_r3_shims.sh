#!/usr/bin/env bash
# Runner for R3 targeted tests against the compiled quaqua binary using temporary shims.
# Does NOT modify any file in scripts/, rules/, AGENTS.md, models.md, or memory/.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/.." && pwd)"

# Ensure binary is up to date
cargo build --locked --manifest-path "$repo/crates/quaqua/Cargo.toml"
bin="$repo/target/debug/quaqua"
[[ -x "$bin" ]] || { echo "error: binary not found at $bin" >&2; exit 1; }

T_SHIM=$(mktemp -d "${TMPDIR:-/tmp}/quaqua-r3-shims.XXXXXX")
T_SHIM=$(cd "$T_SHIM" && pwd -P)
trap 'rm -rf "$T_SHIM"' EXIT

mkdir -p "$T_SHIM/scripts/worker" "$T_SHIM/scripts/lead" "$T_SHIM/scripts/lib" "$T_SHIM/tests"
cp -R "$repo/scripts/lib/"* "$T_SHIM/scripts/lib/"
cp -R "$repo/scripts/lead/"* "$T_SHIM/scripts/lead/"
cp "$repo/scripts/worker/grid-workers.sh" "$T_SHIM/scripts/worker/grid-workers.sh"
cp "$repo/team.toml" "$T_SHIM/team.toml"
cp "$repo/models.md" "$T_SHIM/models.md"
cp -R "$repo/tests/"* "$T_SHIM/tests/"
cp -R "$repo/rules" "$T_SHIM/rules"
cp -R "$repo/plugins" "$T_SHIM/plugins"
# Create temporary shims in $T_SHIM/scripts/worker/
cat > "$T_SHIM/scripts/worker/spawn-worker.sh" << EOF
#!/usr/bin/env bash
# Usage: spawn-worker.sh [--base <ref>] [--review] <name> <kind: tieude|detu|sude|ni|suhuynh|truonglao|bacdau|quansu> <repo-path>
#   name = <kind>-<task>, ascii [a-z0-9-], <=20 chars, not ending in -lead
#          (*-lead is the project Lead convention — spawn-lead.sh <slug>-lead)
#   --base <ref>  start branch quaqua/<name> at <ref> instead of <repo> HEAD
#   --review      bacdau|quansu|ni|suhuynh|truonglao: repo read-only, \$HOME/quaqua/runs writable
export QUAQUA_SCRIPT_DIR="\$(cd "\$(dirname "\$0")" && pwd)"
exec "$bin" worker spawn "\$@"
EOF

cat > "$T_SHIM/scripts/worker/prompt-worker.sh" << EOF
#!/usr/bin/env bash
# Usage: prompt-worker.sh <name> <task>        submit task brief, wait for runs file
#        prompt-worker.sh --resume <name> [--extend <sec>]
#                                             keep waiting; --extend adds <sec> to
#                                             this name's allowance (persisted in
#                                             runs/.<name>.budget; the cap still wins)
#        prompt-worker.sh --check-brief "<task>"   validate a brief + print its budget
export QUAQUA_SCRIPT_DIR="\$(cd "\$(dirname "\$0")" && pwd)"
exec "$bin" worker prompt "\$@"
EOF

cat > "$T_SHIM/scripts/worker/cleanup-worker.sh" << EOF
#!/usr/bin/env bash
# Usage: cleanup-worker.sh <name> <repo-path>
# Stops the agent pane and removes its worktree. Branch quaqua/<name> is kept.
export QUAQUA_SCRIPT_DIR="\$(cd "\$(dirname "\$0")" && pwd)"
exec "$bin" worker cleanup "\$@"
EOF

cat > "$T_SHIM/scripts/worker/worker-cap.sh" << EOF
#!/usr/bin/env bash
# Usage: worker-cap.sh <owner> [<exclude-worker>]
exec "$bin" cap "\$@"
EOF

chmod +x "$T_SHIM/scripts/worker"/*.sh

run_suite() {
  local suite="$1"
  echo "=== Running $suite ==="
  bash "$T_SHIM/tests/$suite"
}

if [[ $# -gt 0 ]]; then
  for arg in "$@"; do
    run_suite "$arg"
  done
else
  run_suite "test_spawn_worker.sh"
  run_suite "test_cleanup_worker.sh"
  run_suite "test_prompt_worker.sh"
  run_suite "test_prompt_worker_watchdog.sh"
  run_suite "test_worker_cap.sh"
fi

echo "ALL TARGET SUITES PASSED"
