#!/usr/bin/env bash
# Sandbox test for cleanup-lead.sh: the team workspace (label <slug>, not the
# caller's) is closed; a Lead in the caller's workspace or a foreign-labelled
# one gets only its pane closed; live Peers block the cleanup; dead Peer owner
# files do not; the INDEX.md owner cell returns to cos. herdr is stubbed on
# PATH and logs every call; the script runs against a temp repo root.
# Usage: scripts/test_cleanup_lead.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-cleanup-lead.XXXXXX")
T=$(cd "$T" && pwd -P)
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

mkdir -p "$T/bin" "$T/scripts" "$T/projects" "$T/runs"
cp "$here/cleanup-lead.sh" "$T/scripts/cleanup-lead.sh"
cat > "$T/INDEX.orig" <<'EOF'
| slug | repo | owner | status | one-line goal |
|------|------|-------|--------|---------------|
| baro | ~/Lab/baro | baro-lead | active | player |
| baro-web | ~/Lab/baro-web | baro-web-lead | active | site |
EOF

cat > "$T/bin/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "$CALLS"
case "${1:-} ${2:-}" in
  "agent get")
    if [[ " ${HERDR_LIVE:-} " == *" ${3:-} "* ]]; then
      printf '{"result":{"agent":{"name":"%s","pane_id":"%s","workspace_id":"%s"}}}\n' \
        "$3" "${LEAD_PANE:-w7:p1}" "${LEAD_PANE%%:*}"
    else
      echo '{"error":{"code":"agent_not_found"}}'; exit 1
    fi ;;
  "workspace get") printf '{"result":{"workspace":{"workspace_id":"%s","label":"%s"}}}\n' "$3" "${WS_LABEL:-baro}" ;;
esac
exit 0
STUB
chmod +x "$T/bin/herdr"

run() {  # run <slug> [VAR=val...]; sets rc and $T/{out,err,calls}
  local slug=$1; shift
  : > "$T/calls"
  cp "$T/INDEX.orig" "$T/projects/INDEX.md"
  printf '# rules\n' > "$T/runs/baro-lead.rules.md"
  rc=0
  CALLS="$T/calls" PATH="$T/bin:$PATH" HERDR_PANE_ID=w9:p13 LEAD_PANE=w7:p1 env "$@" \
    bash "$T/scripts/cleanup-lead.sh" "$slug" >"$T/out" 2>"$T/err" || rc=$?
}

# ── team workspace: closed, owner -> cos, rules dump removed ────────────────
printf 'baro-lead' > "$T/runs/sude-old.owner"   # dead Peer: not registered
run baro HERDR_LIVE=baro-lead
[[ $rc -eq 0 ]] || fail "team: exit $rc: $(cat "$T/err")"
grep -qx 'herdr workspace close w7' "$T/calls" || fail "team: workspace not closed: $(cat "$T/calls")"
! grep -q 'pane close' "$T/calls" || fail "team: pane closed instead of workspace"
grep -qE '^\| baro \| ~/Lab/baro \| cos \|' "$T/projects/INDEX.md" || fail "team: owner not reset"
grep -qE '^\| baro-web \| ~/Lab/baro-web \| baro-web-lead \|' "$T/projects/INDEX.md" \
  || fail "team: other row touched"
[[ ! -e $T/runs/baro-lead.rules.md ]] || fail "team: rules dump kept"
echo "ok: team workspace -> closed, owner reset to cos, dead Peer owner file ignored"

# ── Lead in the caller's workspace (old spawn): pane only, never the CoS ws ──
run baro HERDR_LIVE=baro-lead LEAD_PANE=w9:p1P
[[ $rc -eq 0 ]] || fail "caller ws: exit $rc: $(cat "$T/err")"
grep -qx 'herdr pane close w9:p1P' "$T/calls" || fail "caller ws: pane not closed: $(cat "$T/calls")"
! grep -q 'workspace close' "$T/calls" || fail "caller ws: closed the caller's workspace"
echo "ok: Lead in the caller's workspace -> only its pane closed"

# ── workspace not labelled <slug>: pane only ────────────────────────────────
run baro HERDR_LIVE=baro-lead WS_LABEL=other
[[ $rc -eq 0 ]] || fail "label: exit $rc: $(cat "$T/err")"
grep -qx 'herdr pane close w7:p1' "$T/calls" || fail "label: pane not closed"
! grep -q 'workspace close' "$T/calls" || fail "label: foreign workspace closed"
echo "ok: foreign-labelled workspace -> only the Lead pane closed"

# ── live Peer: refused before any close ─────────────────────────────────────
printf 'baro-lead' > "$T/runs/sude-live.owner"
printf 'cos' > "$T/runs/sude-cos.owner"
run baro "HERDR_LIVE=baro-lead sude-live sude-cos"
[[ $rc -eq 1 ]] || fail "live peer: exit $rc, want 1"
grep -q 'still owns live Peers: sude-live' "$T/err" || fail "live peer: unclear refusal: $(cat "$T/err")"
grep -q 'sude-cos' "$T/err" && fail "live peer: CoS worker counted as the Lead's Peer"
! grep -q 'close' "$T/calls" || fail "live peer: closed something despite refusal"
cmp -s "$T/projects/INDEX.md" "$T/INDEX.orig" || fail "live peer: INDEX touched"
echo "ok: live Peer -> refused, nothing closed, INDEX untouched"

# ── no Lead / bad slug ──────────────────────────────────────────────────────
run baro
[[ $rc -eq 1 ]] && grep -q "no live Lead 'baro-lead'" "$T/err" || fail "no lead: $(cat "$T/err")"
run Bad_slug
[[ $rc -eq 1 ]] && grep -q '^# Usage:' "$T/err" || fail "bad slug: $(cat "$T/err")"
[[ ! -s $T/calls ]] || fail "bad slug: herdr called"
echo "ok: missing Lead and bad slug -> refused"

echo "PASS"
