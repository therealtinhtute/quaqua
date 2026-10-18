#!/usr/bin/env bash
# Documentation/bootstrap consistency gate (F10/F11): every file AGENTS.md
# treats as mandatory must exist (case-sensitively), consolidated mechanics
# references exist, no stale role names/scheduling instructions survive, and the
# advertised report/skill contracts match what the scripts implement.
# Runs against the real repo root — no sandbox needed, nothing is written.
# Usage: tests/test_docs.sh   (exit 0 = pass)
# Local deviations from upstream: README.md is GitHub-side only (untracked
# here) and memory/ is tracked here, so the README assertions and the memory/
# exclusion are dropped/kept accordingly.
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
! grep -rn 'quaqua-[a-z*-]*-orchestrator' "$root/AGENTS.md" \
    "$root/models.md" \
  || fail "stale 'quaqua-<slug>-orchestrator' role name survives"
# rename leaves no dangling refs in tracked docs, scripts (incl. the auditor's
# systemd/launchd units), or tests
# (memory/ is tracked here but is dated history — excluded, as in test_worker_cap.sh)
! grep -rn 'super''visor' "$root/scripts" "$root/tests" \
    --exclude="test_docs.sh" --exclude="test_worker_cap.sh" \
  || fail "dangling legacy references survive in docs/scripts"
# acceptance authority: Lead merges into the integration branch, anh iu owns push
grep -q 'quaqua/<slug>-integ' "$root/AGENTS.md" || fail "integration-branch acceptance missing from AGENTS.md"
echo "ok: consolidated orchestration references; one role naming; acceptance authority consistent"

# ── entrypoints live in role dirs; every scripts/*.sh reference resolves ──────
# Phase 2 moved the entrypoints to scripts/{worker,lead,auditor,workspace}/ (hooks/
# unchanged). A flat file left behind, or any tracked reference to a path that
# does not exist (a stale flat entrypoint path, a typo), fails here. This very
# comment is scanned too: never spell a dead path out in this file.
flat=$(find "$root/scripts" -maxdepth 1 -name '*.sh' -print)
[[ -z $flat ]] || fail "entrypoint left flat in scripts/ (role dir required): $flat"
missing=()
while IFS= read -r f; do
  while IFS= read -r ref; do
    ref="scripts/${ref#*scripts/}"
    [[ -e $root/$ref ]] || missing+=("$f: $ref")
  done < <(grep -aoE '(^|[^A-Za-z0-9_./-])scripts/[A-Za-z0-9_./-]+\.sh' "$root/$f" 2>/dev/null | sort -u)
done < <(git -C "$root" ls-files | grep -v -E '^(scripts/hooks/|scripts/auditor/|docs/research/)')
(( ${#missing[@]} == 0 )) || fail "dangling scripts/*.sh reference(s): ${missing[*]}"
echo "ok: entrypoints in role dirs; every scripts/*.sh reference resolves"

# ── report contract: worker -> '## Lessons', pipeline reviewer -> 'Verdict:' ──
grep -q '## Lessons' "$root/rules/_all/brief.md" || fail "brief.md drops the Lessons tail"
grep -q 'Verdict:' "$root/rules/_all/brief.md" || fail "brief.md ignores the reviewer Verdict tail"
grep -q 'Verdict: clean' "$root/crates/quaqua/src/pipeline/mod.rs" || fail "pipeline lost the Verdict contract"
echo "ok: report-tail contract documented for workers and pipeline reviewers"

# ── scheduling assets: exactly one Auditor entrypoint (F8/F10) ───────────────
[[ -f $root/scripts/auditor/systemd/quaqua-auditor.service && -f $root/scripts/auditor/systemd/quaqua-auditor.timer \
   && -f $root/scripts/auditor/launchd/com.quaqua.auditor.plist.in ]] \
  || fail "systemd or launchd auditor scheduling assets missing"
grep -q 'scripts/auditor/auditor.sh' "$root/scripts/auditor/systemd/quaqua-auditor.service" \
  || fail "auditor service does not exec scripts/auditor/auditor.sh"
grep -q 'scripts/auditor/auditor.sh' "$root/scripts/auditor/launchd/com.quaqua.auditor.plist.in" \
  || fail "launchd template does not exec the same auditor entrypoint"
# README.md is GitHub-side only (not tracked here) — install coverage is
# asserted via the shipped template + installer instead (test_install_auditor.sh).
grep -q 'reports/audit-\*\.md\|reports/audit-' "$root/AGENTS.md" \
  || fail "AGENTS session-start glob != scripts/auditor/auditor.sh output"
echo "ok: auditor entrypoint + scheduling assets agree"

# ── routing consistency: review findings flow through the pipeline ────────────
! grep -q 'to CoS → sude fix\|to CoS -> sude fix' "$root/models.md" \
  || fail "models.md still routes review findings around the pipeline"
echo "ok: review findings route back to the builder via pipeline.sh"

echo "PASS"
