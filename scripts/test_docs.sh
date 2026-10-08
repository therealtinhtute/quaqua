#!/usr/bin/env bash
# Documentation/bootstrap consistency gate (F10/F11): every file AGENTS.md
# treats as mandatory must exist (case-sensitively), consolidated mechanics
# references exist, no stale role names/scheduling instructions survive, and the
# advertised report/skill contracts match what the scripts implement.
# Runs against the real repo root — no sandbox needed, nothing is written.
# Usage: scripts/test_docs.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root=$(cd "$here/.." && pwd)

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── mandatory referenced files exist, case-sensitively (F11) ─────────────────
# AGENTS.md reads identity.md first and mandates these rules/skills.
for f in identity.md rules/_all/brief.md rules/_all/fanout-agent-grid.md \
         rules/_all/worker-timebox.md .claude/skills/herdr-orchestration/SKILL.md; do
  [[ -f $root/$f ]] || fail "mandatory file missing (case-sensitive): $f"
done
grep -q 'identity\.md' "$root/AGENTS.md" || fail "AGENTS.md does not reference identity.md"
echo "ok: identity.md + mandatory rules/_all files + orchestration skill exist"

# ── one authority model + consolidated command/judgment references ─────────
for f in operations.md judgment.md recovery.md; do
  [[ -f $root/.claude/skills/herdr-orchestration/references/$f ]] \
    || fail "orchestration reference missing: $f"
  grep -qF "references/$f" "$root/.claude/skills/herdr-orchestration/SKILL.md" \
    || fail "orchestration skill omits reference: $f"
done
# the retired Lead name must not reappear in contracts (amendments name the
# replacement, never the old label)
! grep -rn 'quaqua-[a-z*-]*-orchestrator' "$root/AGENTS.md" "$root/README.md" \
    "$root/spec-raw" "$root/models.md" \
  || fail "stale 'quaqua-<slug>-orchestrator' role name survives"
# acceptance authority: Lead merges into the integration branch, anh iu owns push
grep -q 'quaqua/<slug>-integ' "$root/AGENTS.md" || fail "integration-branch acceptance missing from AGENTS.md"
echo "ok: consolidated orchestration references; one role naming; acceptance authority consistent"

# ── report contract: worker -> '## Lessons', pipeline reviewer -> 'Verdict:' ──
grep -q '## Lessons' "$root/rules/_all/brief.md" || fail "brief.md drops the Lessons tail"
grep -q 'Verdict:' "$root/rules/_all/brief.md" || fail "brief.md ignores the reviewer Verdict tail"
grep -q 'Verdict: clean' "$root/scripts/pipeline.sh" || fail "pipeline.sh lost the Verdict contract"
echo "ok: report-tail contract documented for workers and pipeline reviewers"

# ── scheduling assets: exactly one Auditor entrypoint (F8/F10) ───────────────
[[ -f $root/systemd/quaqua-supervisor.service && -f $root/systemd/quaqua-supervisor.timer \
   && -f $root/launchd/com.quaqua.supervisor.plist.in ]] \
  || fail "systemd or launchd auditor scheduling assets missing"
grep -q 'scripts/supervisor.sh' "$root/systemd/quaqua-supervisor.service" \
  || fail "auditor service does not exec scripts/supervisor.sh"
grep -q 'scripts/supervisor.sh' "$root/launchd/com.quaqua.supervisor.plist.in" \
  || fail "launchd template does not exec the same auditor entrypoint"
grep -q 'install-supervisor.sh' "$root/README.md" || fail "README install skips the auditor installer"
grep -qi 'launchd' "$root/README.md" || fail "README install omits launchd"
grep -qi 'systemd' "$root/README.md" || fail "README install omits systemd"
grep -q 'reports/supervisor-\*\.md\|reports/supervisor-' "$root/AGENTS.md" \
  || fail "AGENTS session-start glob != scripts/supervisor.sh output"
echo "ok: auditor entrypoint + scheduling assets + README install agree"

# ── routing consistency: quansu findings flow through the pipeline ────────────
! grep -q 'to CoS → sude fix\|to CoS -> sude fix' "$root/models.md" \
  || fail "models.md still routes review findings around the pipeline"
echo "ok: quansu findings route back to the builder via pipeline.sh"

echo "PASS"
