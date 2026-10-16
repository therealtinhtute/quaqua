#!/usr/bin/env bash
# Runner for R5 targeted tests against the compiled quaqua binary using
# temporary shim roots. Reproduces tests/test_herdr_gate.sh and
# tests/test_init_workspace.sh with the two bash entrypoints replaced by one-line
# shims over `quaqua gate` / `quaqua init`. Does NOT modify scripts/, rules/,
# AGENTS.md, models.md, memory/ or .claude/settings.json.
# Usage: tests/test_r5_shims.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/.." && pwd -P)"

cargo build --locked --manifest-path "$repo/crates/quaqua/Cargo.toml"
bin="$repo/target/debug/quaqua"
[[ -x "$bin" ]] || { echo "error: binary not found at $bin" >&2; exit 1; }

T=$(mktemp -d "${TMPDIR:-/tmp}/quaqua-r5-shims.XXXXXX")
T=$(cd "$T" && pwd -P)
trap 'rm -rf "$T"' EXIT

# ── gate shim root: the test only addresses scripts/hooks/herdr-gate.sh ───────
G="$T/gate"
mkdir -p "$G/scripts/hooks" "$G/tests"
cat > "$G/scripts/hooks/herdr-gate.sh" <<EOF
#!/usr/bin/env bash
exec "$bin" gate "\$@"
EOF
chmod +x "$G/scripts/hooks/herdr-gate.sh"
cp "$repo/tests/test_herdr_gate.sh" "$G/tests/test_herdr_gate.sh"

# ── init shim root: a true fresh clone = tracked skeleton minus the personal-
# state paths (memory/ projects/ raw/ runs/ reports/ USER.md). git-inited so the
# test's `git ls-files` sandbox and check-ignore guard work; the hook shim stands
# in for scripts/workspace/init-workspace.sh.
I="$T/init"
mkdir -p "$I"
git -C "$repo" ls-files -z | while IFS= read -r -d '' f; do
  case $f in memory/*|projects/*|raw/*|runs/*|reports/*|USER.md) continue;; esac
  mkdir -p "$I/$(dirname "$f")"
  cp "$repo/$f" "$I/$f"
done
mkdir -p "$I/tests" "$I/scripts/hooks" "$I/scripts/workspace"
cp "$repo/tests/test_init_workspace.sh" "$I/tests/test_init_workspace.sh"
cp "$repo/scripts/hooks/pre-commit" "$I/scripts/hooks/pre-commit"
chmod +x "$I/scripts/hooks/pre-commit"
cat > "$I/scripts/workspace/init-workspace.sh" <<EOF
#!/usr/bin/env bash
export QUAQUA_SCRIPT_DIR="\$(cd "\$(dirname "\$0")" && pwd)"
exec "$bin" init "\$@"
EOF
chmod +x "$I/scripts/workspace/init-workspace.sh"
git -C "$I" init -q && git -C "$I" add -A

echo "=== quaqua gate vs tests/test_herdr_gate.sh (shim root $G) ==="
bash "$G/tests/test_herdr_gate.sh"

echo "=== quaqua init vs tests/test_init_workspace.sh (shim root $I) ==="
bash "$I/tests/test_init_workspace.sh"

echo "ALL R5 TARGET SUITES PASSED"
