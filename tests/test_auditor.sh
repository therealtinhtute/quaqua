#!/usr/bin/env bash
# Sandbox test for auditor.sh: report naming, the codex exec invocation
# (read-only sandbox, -C repo root, -o report path, quansu-tier model), the
# prompt contents, AUDITOR_MODEL/AUDITOR_OUT overrides, codex failure
# handling, and that nothing but the report lands in the repo tree. codex is
# stubbed on PATH (it logs argv and writes a fake final message to the -o
# file); the repo is a temp dir. No real codex, repo, or reports touched.
# Usage: tests/test_auditor.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-auditor.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── sandbox: stub codex + fake repo with scripts/auditor/auditor.sh ──────────
mkdir -p "$T/bin" "$T/repo/scripts/auditor" "$T/repo/memory" "$T/repo/projects"
cp "$src/auditor/auditor.sh" "$T/repo/scripts/auditor/auditor.sh"
cat > "$T/bin/codex" <<'STUB'
#!/usr/bin/env bash
echo "codex $*" >> "$CALLS"
out=""
prev=""
for a in "$@"; do
  [ "$prev" = -o ] && out=$a
  prev=$a
done
if [ -n "$out" ] && [ "${FAKE_RC:-0}" = 0 ]; then
  printf '## Drift\n- none\n\n## Stale docs\n- none\n\nVerdict: clean\n' > "$out"
fi
exit "${FAKE_RC:-0}"
STUB
chmod +x "$T/bin/codex"

# ── precheck runs are hermetic: no real herdr on PATH yet, so stub a failing
#    one. Without this the prechecks would consult the host's herdr, and a
#    host failure would inject a 'herdr agent list failed' flag into fixtures
#    that assert exact flags. The sandbox section below replaces this stub
#    with a working one.
cat > "$T/bin/herdr" <<'HERDR'
#!/usr/bin/env bash
if [[ "$1" == "agent" && "$2" == "list" ]]; then
  printf 'herdr: no agent registry in the precheck sandbox\n' >&2
  exit 1
fi
exit 0
HERDR
chmod +x "$T/bin/herdr"

run() {  # run [VAR=val...]; sets rc, $T/out, $T/err, $T/calls
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" PATH="$T/bin:$PATH" \
    env "$@" bash "$T/repo/scripts/auditor/auditor.sh" >"$T/out" 2>"$T/err" || rc=$?
}

# ── happy path: report file, read-only sandbox invocation, prompt contents ──
before=$(find "$T/repo" -type f | sort)
run
[[ $rc -eq 0 ]] || fail "happy: exit $rc, want 0: $(cat "$T/err")"
report=$(printf '%s\n' "$T"/repo/reports/audit-????-??-??-??.md)
[[ -f $report && $report != *\** ]] || fail "happy: no reports/audit-YYYY-MM-DD-HH.md in repo"
grep -q 'Verdict: clean' "$report" || fail "happy: agent final message not in report"
[[ $(cat "$T/out") == "$report" ]] || fail "happy: stdout not the report path: $(cat "$T/out")"
calls=$(cat "$T/calls")
grep -q 'codex exec ' <<<"$calls" || fail "happy: codex not called with exec"
grep -q -- '--sandbox read-only' <<<"$calls" || fail "happy: no read-only sandbox"
grep -q -- "-C $T/repo" <<<"$calls" || fail "happy: not rooted at repo (-C)"
grep -q -- "-o $report" <<<"$calls" || fail "happy: no -o <report path>"
grep -q -- '-m gpt-6.1-sol' <<<"$calls" || fail "happy: default model not the quansu tier"
for kw in 'memory/' 'projects/' 'git log' 'READ-ONLY' \
    '## Unhandled' '[audit]' '## Lessons' 'runs/'; do
  grep -qF "$kw" <<<"$calls" || fail "happy: prompt missing '$kw'"
done
after=$(find "$T/repo" -type f | sort)
[[ $(comm -13 <(printf '%s\n' "$before") <(printf '%s\n' "$after")) == "$report" ]] \
  || fail "happy: repo tree changed beyond the report file"
echo "ok: happy -> report written, codex exec --sandbox read-only, prompt covers all audits"

# ── overrides: AUDITOR_MODEL + AUDITOR_OUT ──────────────────────────────────
rm -rf "$T/repo/reports"
run AUDITOR_MODEL=other-model AUDITOR_OUT="$T/custom.md"
[[ $rc -eq 0 ]] || fail "overrides: exit $rc: $(cat "$T/err")"
[[ -f $T/custom.md ]] || fail "overrides: AUDITOR_OUT ignored"
grep -q -- '-m other-model' "$T/calls" || fail "overrides: AUDITOR_MODEL ignored"
[[ ! -d $T/repo/reports ]] || fail "overrides: default reports dir still used"
echo "ok: overrides -> model + output path honored"

# ── codex failure: nonzero exit, failure report, stderr passthrough ─────────
run FAKE_RC=3
[[ $rc -eq 3 ]] || fail "failure: exit $rc, want 3"
report=$(printf '%s\n' "$T"/repo/reports/audit-????-??-??-??.md)
grep -q 'FAILED' "$report" || fail "failure: no failure report written"
echo "ok: failure -> exit propagates, stub report notes the failure"

# ── F8 packaging: exactly one Auditor entrypoint with all scheduling assets ──
root=$(cd "$here/.." && pwd)
[[ -f $root/scripts/auditor/systemd/quaqua-auditor.service ]] || fail "pkg: no systemd service shipped"
[[ -f $root/scripts/auditor/systemd/quaqua-auditor.timer ]] || fail "pkg: no systemd timer shipped"
grep -q 'scripts/auditor/auditor.sh' "$root/scripts/auditor/systemd/quaqua-auditor.service" \
  || fail "pkg: service does not exec scripts/auditor/auditor.sh"
grep -q 'OnCalendar' "$root/scripts/auditor/systemd/quaqua-auditor.timer" || fail "pkg: timer has no schedule"
grep -q 'systemd' "$src/auditor/install-auditor.sh" || fail "pkg: installer does not use systemd/"
[[ ! -d "$root/super""visor" ]] || fail "pkg: legacy runner still installed"
[[ -f $root/scripts/auditor/launchd/com.quaqua.auditor.plist.in ]] || fail "pkg: no launchd template shipped"
grep -q 'scripts/auditor/auditor.sh' "$root/scripts/auditor/launchd/com.quaqua.auditor.plist.in" \
  || fail "pkg: launchd template does not use the same auditor entrypoint"
# README.md is GitHub-side only (not tracked in this repo) — launchd install
# coverage is asserted via the template + installer instead.
grep -q 'reports/audit-' "$root/AGENTS.md" \
  || fail "pkg: AGENTS session-start glob missing reports/audit-"
echo "ok: packaging -> systemd + launchd share scripts/auditor/auditor.sh; legacy retired"

# ── pipeline pre-checks: one fixture per flag + clean no-flag cases ─────────
# Fixtures live in the sandbox repo (runs/, memory/); the stub codex logs argv,
# so the deterministic flags are asserted on the embedded prompt text.
mkdir -p "$T/repo/runs"
git -C "$T/repo" init -q
git -C "$T/repo" -c user.email=t@t -c user.name=t commit -qm init --allow-empty
sha=$(git -C "$T/repo" rev-parse --short HEAD)
# flagged: disputed|blocked with no memory handling line
printf -- '- status: disputed · commit %s · elapsed 5m\n' "$sha" \
  > "$T/repo/runs/pipeline-dispute.md"
printf -- '- status: blocked · commit %s · elapsed 9m\n\n## Challenge (from sude-blocked.build.md)\n- disputed a finding\n' "$sha" \
  > "$T/repo/runs/pipeline-blocked.md"
# flagged: clean with commit none / unresolvable
printf -- '- review: skipped (gate: auto)\n- status: clean · commit none · elapsed 3m\n' \
  > "$T/repo/runs/pipeline-nocommit.md"
printf -- '- review: skipped (gate: auto)\n- status: clean · commit deadbeef · elapsed 4m\n' \
  > "$T/repo/runs/pipeline-badsha.md"
# flagged: clean, review ran, no Verdict on the last review-round line AND
# no reviewer runs file behind it
printf -- '- review round 1: NO VERDICT LINE (quansu-noverdictr1.md)\n- status: clean · commit %s · elapsed 6m\n' "$sha" \
  > "$T/repo/runs/pipeline-noverdict.md"
# flagged: same, but the named reviewer file exists without a Verdict line
printf -- '- review round 1: NO VERDICT LINE (quansu-novlineR1.md)\n- status: clean · commit %s · elapsed 6m\n' "$sha" \
  > "$T/repo/runs/pipeline-novline.md"
printf -- '# quansu-novlineR1\n\n## Findings\n- one thing\n' > "$T/repo/runs/quansu-novlineR1.md"
# no flag: the Verdict lives in the reviewer runs file the round line names —
# both the '(file.md)' shape and the '<name> spawned on ...' shape
printf -- '- review round 1: NO VERDICT LINE (quansu-revfiler1.md)\n- status: clean · commit %s · elapsed 6m\n' "$sha" \
  > "$T/repo/runs/pipeline-revfile.md"
printf -- '# quansu-revfiler1\n\nVerdict: clean\n' > "$T/repo/runs/quansu-revfiler1.md"
printf -- '- review round 1: quansu-revspawnr1 spawned on %s (quaqua/sude-revspawn tip)\n- status: clean · commit %s · elapsed 6m\n' "$sha" "$sha" \
  > "$T/repo/runs/pipeline-revspawn.md"
printf -- '# quansu-revspawnr1\n\nVerdict: 2 findings\n' > "$T/repo/runs/quansu-revspawnr1.md"
# flagged: unresolved ## Challenge in a worker runs file
printf -- '# x\n## Challenge\n- premise wrong, evidence\n' > "$T/repo/runs/worker-chal.md"
# flagged: disputed + copied Challenge (also covers sude-copied.build.md)
printf -- '- status: disputed · commit %s · elapsed 4m\n\n## Challenge (from sude-copied.build.md)\n- disputed a finding\n' "$sha" \
  > "$T/repo/runs/pipeline-copied.md"
printf -- '# x\n## Challenge\n- disputed a finding\n' > "$T/repo/runs/sude-copied.build.md"
# no flag: clean with resolvable commit + Verdict; handled disputed; plan-ready
# pause; challenge resolved via memory line; .rules.md dump; phase file whose
# worker name is mentioned in memory
printf -- '- review round 1: Verdict: clean (quansu-okr1.md)\n- status: clean · commit %s · elapsed 7m\n' "$sha" \
  > "$T/repo/runs/pipeline-ok.md"
printf -- '- review: skipped (gate: auto)\n- status: clean · commit %s · elapsed 2m\n' "$sha" \
  > "$T/repo/runs/pipeline-skip.md"
printf -- '- status: disputed · commit %s · elapsed 8m\n' "$sha" \
  > "$T/repo/runs/pipeline-handled.md"
printf -- '- status: plan-ready · commit none · elapsed 1m\n' \
  > "$T/repo/runs/pipeline-paused.md"
printf -- '# x\n## Challenge\n- was wrong, resolved\n' > "$T/repo/runs/worker-ok.md"
printf -- '# x\n## Challenge\n- premise wrong, evidence\n' > "$T/repo/runs/sude-phs.plan.md"
printf -- '## Challenge\n- rule text mentioning it — must not flag\n' > "$T/repo/runs/x.rules.md"
cat > "$T/repo/memory/2026-10-08.md" <<'MEM'
- [quaqua] pipeline-handled disputed — quansu reviewing
- [quaqua] worker-ok done; its Challenge resolved: premise accepted
- [quaqua] sude-phs plan challenged, resolved by CoS
MEM
run
[[ $rc -eq 0 ]] || fail "precheck: exit $rc: $(cat "$T/err")"
calls=$(cat "$T/calls")
for want in \
  'pipeline-dispute.md: status disputed' \
  'pipeline-blocked.md: status blocked' \
  "pipeline-blocked.md: '## Challenge' unresolved" \
  'pipeline-nocommit.md: status clean but commit' \
  "pipeline-badsha.md: status clean but commit 'deadbeef'" \
  "pipeline-noverdict.md: status clean, review ran, but no 'Verdict:' on the last review-round line or in the reviewer runs file it names" \
  "pipeline-novline.md: status clean, review ran, but no 'Verdict:' on the last review-round line or in the reviewer runs file it names" \
  'pipeline-copied.md: status disputed' \
  "pipeline-copied.md: '## Challenge' unresolved" \
  "worker-chal.md: '## Challenge' unresolved" \
  '## Pipeline' ; do
  grep -qF "$want" <<<"$calls" || fail "precheck: prompt missing '$want'"
done
for absent in 'pipeline-ok.md' 'pipeline-skip.md' 'pipeline-handled.md' \
    'pipeline-paused.md' 'worker-ok.md' 'sude-copied.build.md' 'sude-phs' \
    'x.rules' 'pipeline-revfile.md' 'pipeline-revspawn.md' \
    'quansu-revfiler1' 'quansu-revspawnr1' ; do
  grep -qF "$absent" <<<"$calls" && fail "precheck: false flag '$absent'"
done
echo "ok: precheck -> each flag raised; clean/handled/copied cases silent"
echo "ok: precheck Verdict -> accepted from the reviewer runs file (both round-line shapes)"

# ── F9: commits resolve in the pipeline's recorded repo, not this checkout ───
mkdir -p "$T/proj" "$T/proj2"
git -C "$T/proj" init -q
git -C "$T/proj" -c user.email=t@t -c user.name=t commit -qm p1 --allow-empty
psha=$(git -C "$T/proj" rev-parse --short HEAD)
git -C "$T/proj" -c user.email=t@t -c user.name=t commit -qm p2 --allow-empty
psha2=$(git -C "$T/proj" rev-parse --short HEAD)
rm -f "$T"/repo/runs/pipeline-*.md "$T/repo/memory/2026-10-08.md"
# valid commit in the recorded project repo -> no flag
printf -- '- task: xproj · repo: %s\n- review round 1: Verdict: clean (quansu-xprojr1.md)\n- status: clean · commit %s · elapsed 7m\n' \
  "$T/proj" "$psha" > "$T/repo/runs/pipeline-xproj.md"
# a sha that exists in the auditor repo but NOT in the recorded repo -> flag
printf -- '- task: badproj · repo: %s\n- status: clean · commit %s · elapsed 4m\n' \
  "$T/proj" "$sha" > "$T/repo/runs/pipeline-badproj.md"
# a recorded repo that is missing entirely -> separate flag
printf -- '- task: goneproj · repo: %s\n- status: clean · commit %s · elapsed 4m\n' \
  "$T/proj-missing" "$psha" > "$T/repo/runs/pipeline-goneproj.md"
run
[[ $rc -eq 0 ]] || fail "F9: exit $rc: $(cat "$T/err")"
calls=$(cat "$T/calls")
grep -qF 'pipeline-xproj.md' <<<"$calls" \
  && fail "F9: valid project-repo commit flagged: $(grep -o 'pipeline-xproj[^\"]*' <<<"$calls")"
grep -qF "pipeline-badproj.md: status clean but commit '$sha' does not resolve in recorded repo $T/proj" <<<"$calls" \
  || fail "F9: cross-repo false positive not detected"
grep -qF "pipeline-goneproj.md: status clean but recorded repo '$T/proj-missing' is missing" <<<"$calls" \
  || fail "F9: missing recorded repo not flagged separately"
echo "ok: F9 -> commits verified in the recorded repo; missing repo flagged apart"

# ── live-worker sandbox audit check ──────────────────────────────────────────
cat > "$T/bin/herdr" <<'HERDR'
#!/usr/bin/env bash
if [[ "$1" == "agent" && "$2" == "list" ]]; then
  if [[ -n ${HERDR_LIST_RC:-} ]]; then
    [[ -n ${HERDR_LIST_ERR:-} ]] && printf '%s\n' "$HERDR_LIST_ERR" >&2
    exit "$HERDR_LIST_RC"
  fi
  if [[ -n ${HERDR_LIST_OUT:-} ]]; then printf '%s\n' "$HERDR_LIST_OUT"; exit 0; fi
  cat <<'JSON'
{"result":{"agents":[
  {"name":"sude-unsandboxed","agent_status":"idle","pane_id":"p1"},
  {"name":"sude-sandboxed","agent_status":"idle","pane_id":"p2"},
  {"name":"coucou-island-lead","agent_status":"idle","pane_id":"p3"}
]}}
JSON
  exit 0
fi
exit 0
HERDR
chmod +x "$T/bin/herdr"
mkdir -p "$T/repo/runs"
touch "$T/repo/runs/sude-sandboxed.srt.json"
rm -f "$T/repo/runs/sude-unsandboxed.srt.json"
run
[[ $rc -eq 0 ]] || fail "sandbox check: exit $rc: $(cat "$T/err")"
calls=$(cat "$T/calls")
grep -qF "live worker 'sude-unsandboxed': not started through sandbox wrapper (missing runs/sude-unsandboxed.srt.json)" <<<"$calls" \
  || fail "sandbox check: unsandboxed live worker not flagged"
! grep -qF "sude-sandboxed" <<<"$calls" \
  || fail "sandbox check: sandboxed worker falsely flagged"
! grep -qF "coucou-island-lead" <<<"$calls" \
  || fail "sandbox check: Lead falsely flagged"
! grep -qF 'herdr agent list failed' <<<"$calls" \
  || fail "sandbox check: working herdr reported as failed"
echo "ok: live worker sandbox audit -> unsandboxed worker flagged; sandboxed worker and Lead silent"

# ── herdr agent list failure is reported, never silently 'none' ─────────────
# Under launchd/codex the herdr socket call can fail (EPERM). A silent skip
# reads as "no live workers" — the failure itself must become a finding.
run HERDR_LIST_RC=1 HERDR_LIST_ERR='herdr: Operation not permitted'
[[ $rc -eq 0 ]] || fail "herdr EPERM: exit $rc: $(cat "$T/err")"
calls=$(cat "$T/calls")
grep -qF 'herdr agent list failed (exit 1: herdr: Operation not permitted) — live-worker sandbox audit skipped' <<<"$calls" \
  || fail "herdr EPERM: failure not reported: $(cat "$T/calls")"
! grep -qF 'live worker' <<<"$calls" \
  || fail "herdr EPERM: worker scan ran despite the failure"
echo "ok: herdr agent list EPERM -> failure reported, scan skipped"

# herdr succeeds but emits garbage (proxy banner, truncated output) -> also a flag
run HERDR_LIST_OUT='not json'
[[ $rc -eq 0 ]] || fail "herdr garbage: exit $rc: $(cat "$T/err")"
calls=$(cat "$T/calls")
grep -qF 'herdr agent list returned no JSON — live-worker sandbox audit skipped' <<<"$calls" \
  || fail "herdr garbage: non-JSON output not reported: $(cat "$T/calls")"
! grep -qF 'live worker' <<<"$calls" \
  || fail "herdr garbage: worker scan ran despite non-JSON output"
echo "ok: herdr agent list non-JSON -> failure reported, scan skipped"

# ── doc-rot: instruction docs over 100 lines are flagged; logs are not ───────
mkdir -p "$T/repo/rules/_all" "$T/repo/raw"
seq 101 > "$T/repo/LONG.md"
seq 100 > "$T/repo/SHORT.md"
seq 150 > "$T/repo/rules/_all/long-rule.md"
seq 150 > "$T/repo/projects/long-card.md"
seq 150 > "$T/repo/memory/2026-10-11.md"
seq 150 > "$T/repo/raw/2026-10-11-x.md"
run
[[ $rc -eq 0 ]] || fail "doc-rot: exit $rc: $(cat "$T/err")"
calls=$(cat "$T/calls")
for want in 'LONG.md: 101 lines (>100)' 'rules/_all/long-rule.md: 150 lines' \
    'projects/long-card.md: 150 lines'; do
  grep -qF "$want" <<<"$calls" || fail "doc-rot: prompt missing '$want'"
done
for absent in 'SHORT.md' 'memory/2026-10-11.md' 'raw/2026-10-11-x.md'; do
  grep -qF "$absent" <<<"$calls" && fail "doc-rot: false flag '$absent'"
done
echo "ok: doc-rot -> >100-line instruction docs flagged; 100-line doc, logs, raw silent"

echo "PASS"
