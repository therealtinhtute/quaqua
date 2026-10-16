#!/usr/bin/env bash
# Finding 1 (suhuynh-r6r1): the PreToolUse hook must fail CLOSED when the
# quaqua binary is missing. Claude Code blocks only on exit 2; a shim that
# dies with 127 (bare `quaqua gate` with no binary on PATH) lets every
# pane/workspace spawn+move through. This suite runs the real repo shim with
# no quaqua anywhere and asserts exit 2, then checks that $QUAQUA_BIN is
# honoured, and that .claude/settings.json routes through the shim (not a bare
# PATH lookup). No real herdr, no quaqua on PATH, no repo mutation.
# Usage: tests/test_gate_shim.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd -P)"
shim="$root/scripts/hooks/herdr-gate.sh"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-gate-shim.XXXXXX")
trap 'rm -rf "$T"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

[[ -f $shim ]] || fail "missing shim at $shim"
mkdir -p "$T/empty"
# no quaqua anywhere on this PATH (but a real bash, so the stub can run)
noq_path="$T/empty:/usr/bin:/bin"

# ── no quaqua at all: the hook must exit 2 (blocking), not 127 ───────────────
rc=0 err=''
err=$(printf '%s' '{"tool_name":"Bash","tool_input":{"command":"herdr pane split"}}' \
  | env -u QUAQUA_BIN PATH="$noq_path" "$BASH" "$shim" 2>&1 >/dev/null) || rc=$?
[[ $rc -eq 2 ]] || fail "missing binary: exit $rc, want 2 (fail-closed); stderr: $err"
grep -q 'not found' <<<"$err" || fail "missing binary: no diagnostic: $err"
grep -q 'fail-closed' <<<"$err" || fail "missing binary: diagnostic does not say fail-closed: $err"
echo "ok: no quaqua on PATH -> exit 2 with a fail-closed diagnostic"

# ── without the binary the fallback blocks only the guarded commands ─────────
# r1 (fail-open) stays fixed: the fallback still exits 2 on spawn+move. But it
# must not lock the agent out of the shell — a guarded command blocks, every
# other Bash call passes with a warning.
for c in 'herdr pane split' 'sudo herdr workspace close w7' \
         'bash -c '\''herdr agent start x'\''' '/usr/bin/herdr tab create --label y'; do
  rc=0 err=''
  err=$(printf '{"tool_name":"Bash","tool_input":{"command":"%s"}}' "$c" \
    | env QUAQUA_BIN=/nonexistent/quaqua PATH="$noq_path" "$BASH" "$shim" 2>&1 >/dev/null) || rc=$?
  [[ $rc -eq 2 ]] || fail "missing binary, guarded '$c': exit $rc, want 2; stderr: $err"
  grep -q 'not found' <<<"$err" || fail "missing binary, guarded '$c': no diagnostic: $err"
  grep -q 'fail-closed' <<<"$err" || fail "missing binary, guarded '$c': no fail-closed note: $err"
done
echo "ok: missing binary -> guarded spawn/move still exit 2 (fail-closed)"

for c in 'cat x' 'cargo build --locked' 'git status' 'herdr agent list' \
         'herdr pane send-keys w1:p5 enter'; do
  rc=0 err=''
  err=$(printf '{"tool_name":"Bash","tool_input":{"command":"%s"}}' "$c" \
    | env QUAQUA_BIN=/nonexistent/quaqua PATH="$noq_path" "$BASH" "$shim" 2>&1 >/dev/null) || rc=$?
  [[ $rc -eq 0 ]] || fail "missing binary, plain '$c': exit $rc, want 0; stderr: $err"
  grep -q 'not found' <<<"$err" || fail "missing binary, plain '$c': no warning: $err"
done
echo "ok: missing binary -> non-guarded commands pass with a warning"

# ── $QUAQUA_BIN is honoured and gets `gate` + the hook's stdin ───────────────
cat > "$T/stub" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" > "$STUB_CALLS"
cat > "$STUB_STDIN"
exit "${STUB_RC:-0}"
STUB
chmod +x "$T/stub"
: > "$T/calls"; : > "$T/stdin"
printf '%s' '{"tool_name":"Bash","tool_input":{"command":"ls"}}' \
  | STUB_CALLS="$T/calls" STUB_STDIN="$T/stdin" QUAQUA_BIN="$T/stub" PATH="$noq_path" \
    "$BASH" "$shim" >/dev/null 2>&1 || fail "QUAQUA_BIN stub: shim exited non-zero"
[[ $(cat "$T/calls") == "gate" ]] || fail "QUAQUA_BIN stub: argv '$(cat "$T/calls")', want 'gate'"
grep -q '"command":"ls"' "$T/stdin" || fail "QUAQUA_BIN stub: stdin not forwarded"
echo "ok: QUAQUA_BIN honoured, argv=gate, stdin forwarded"

# ── the settings hook runs the shim, not a bare PATH lookup ──────────────────
grep -qF 'scripts/hooks/herdr-gate.sh' "$root/.claude/settings.json" \
  || fail ".claude/settings.json does not route PreToolUse through the shim"
grep -qF 'CLAUDE_PROJECT_DIR' "$root/.claude/settings.json" \
  || fail ".claude/settings.json does not anchor the shim at \$CLAUDE_PROJECT_DIR"
! grep -qF '"command": "quaqua gate"' "$root/.claude/settings.json" \
  || fail ".claude/settings.json still uses the bare 'quaqua gate' PATH lookup"
echo "ok: PreToolUse command is the shim (\$CLAUDE_PROJECT_DIR), not a bare binary lookup"

echo PASS
