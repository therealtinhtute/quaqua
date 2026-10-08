#!/usr/bin/env bash
# Sandbox test for cleanup-worker.sh's worktree removal and grid hook. No real
# herdr/runs touched: temp git repo, fake herdr, and a stub grid-workers.sh
# next to a copy of the script.
# Usage: scripts/test_cleanup_worker.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-cleanup-worker.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── sandbox: temp worker repo + fake herdr + stub grid ──────────────────────
mkdir -p "$T/quaqua" "$T/bin" "$T/scripts"
cp "$here/cleanup-worker.sh" "$T/scripts/cleanup-worker.sh"
cat > "$T/scripts/grid-workers.sh" <<'EOF'
#!/usr/bin/env bash
printf 'grid%s\n' "${*:+ $*}" >> "$EVENTS"
exit "${GRID_FAKE_RC:-0}"
EOF

cat > "$T/bin/herdr" <<'EOF'
#!/usr/bin/env bash
echo "herdr $*" >> "${EVENTS:-/dev/null}"
[[ "$1" == agent && "$2" == get ]] && echo '{"result":{"agent":{"pane_id":"pane-1"}}}'
exit 0
EOF
chmod +x "$T/bin/herdr"

git -C "$T/quaqua" init -q
git -C "$T/quaqua" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init

worktree() {  # worktree <repo> <name>; cleanup keeps the branch, so reuse it
  local repo=$1 name=$2 wt="$(dirname "$1")/.wt/$2"
  if git -C "$repo" show-ref -q "refs/heads/quaqua/$name"; then
    git -C "$repo" worktree add -q "$wt" "quaqua/$name"
  else
    git -C "$repo" worktree add -q -b "quaqua/$name" "$wt"
  fi
}
run_cleanup() {  # run_cleanup <name> [VAR=val...]
  local name=$1; shift
  : > "$T/events"
  env PATH="$T/bin:$PATH" EVENTS="$T/events" "$@" \
    bash "$T/scripts/cleanup-worker.sh" "$name" "$T/quaqua"
}

# ── 1. cleanup removes the worktree; grid hook runs after the pane close ────
worktree "$T/quaqua" zz-clean
run_cleanup zz-clean >/dev/null || fail "cleanup failed"
[[ ! -d "$T/.wt/zz-clean" ]] || fail "worktree not removed"
[[ "$(grep '^grid' "$T/events")" == "grid" ]] || fail "grid hook: not called: $(cat "$T/events")"
[[ "$(tail -1 "$T/events")" == "grid" ]] || fail "grid hook: not after the pane close: $(cat "$T/events")"
echo "ok: cleanup -> worktree removed, grid called after the pane is closed"

# ── 2. GRID_TAB selects the project's 2-4 worker tab ────────────────────────
worktree "$T/quaqua" zz-clean
run_cleanup zz-clean GRID_TAB=coucou-workers >/dev/null || fail "cleanup failed with GRID_TAB"
grep -q '^grid --tab coucou-workers$' "$T/events" || fail "GRID_TAB: not passed as --tab: $(cat "$T/events")"
echo "ok: GRID_TAB -> grid-workers.sh --tab coucou-workers"

# ── 3. GRID_SKIP=1 disables the grid hook ───────────────────────────────────
worktree "$T/quaqua" zz-clean
run_cleanup zz-clean GRID_SKIP=1 >/dev/null || fail "cleanup failed with GRID_SKIP"
grep -q '^grid' "$T/events" && fail "GRID_SKIP=1: grid called: $(cat "$T/events")"
echo "ok: GRID_SKIP=1 -> grid not called"

# ── 4. a failing grid hook warns but never blocks the cleanup ───────────────
worktree "$T/quaqua" zz-clean
err=$(run_cleanup zz-clean GRID_FAKE_RC=1 2>&1 >/dev/null) || fail "cleanup failed when the grid hook failed"
[[ "$err" == *"warning: grid-workers.sh failed"* ]] || fail "grid failure: no warning"
[[ ! -d "$T/.wt/zz-clean" ]] || fail "grid failure: worktree not removed"
echo "ok: grid failure -> warning only, cleanup succeeds"

echo "PASS"
