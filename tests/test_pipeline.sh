#!/usr/bin/env bash
# Sandbox test for pipeline.sh: arg validation, the plan-ready pause (exit 5)
# + --resume, the auto review gate (>150 changed lines or >1 top-level dir),
# clean / disputed / rounds-exhausted / missing-Verdict outcomes, watchdog
# exit propagation, and one telemetry.jsonl line per stop. git and herdr are
# stubbed on PATH (they log calls; herdr fabricates runs files from FAKE_*
# vars keyed on the 'Pipeline phase: X' marker in the prompt). HOME is a temp
# dir. No real herdr, agent, repo, or home touched.
# Usage: tests/test_pipeline.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-pipeline.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2
  { echo '--- out ---'; cat "$T/out"
    echo '--- err ---'; tail -20 "$T/err"
    echo '--- pfile ---'; tail -30 "$T"/home/quaqua/runs/pipeline-*.md
    echo '--- calls ---'; tail -20 "$T/calls"; } >&2 2>/dev/null
  exit 1; }

# ── sandbox: stub git/herdr, temp HOME/quaqua/runs, dummy repo + brief ──────
mkdir -p "$T/bin" "$T/repo" "$T/home/quaqua/runs"
# git stub: rev-parse HEAD is keyed on the -C dir's basename; call #1 is the
# spawn-time base, call #2 the post-build tip, call #3+ the post-fix tip.
# FAKE_BASE_SHA/FAKE_HEAD_SHA/FAKE_HEAD2_SHA shape each; FAKE_PORCELAIN feeds
# `status --porcelain` (empty = clean tree).
cat > "$T/bin/git" <<'STUB'
#!/usr/bin/env bash
echo "git $*" >> "$CALLS"
dir='' prev=''
for a in "$@"; do [ "$prev" = -C ] && dir=$a; prev=$a; done
cf="$GITN-${dir##*/}"; n=$(cat "$cf" 2>/dev/null || echo 0)
case " $* " in
  *"status --porcelain"*)   printf '%s\n' "${FAKE_PORCELAIN:-}" ;;
  *"diff --numstat"*)       printf '%s\n' "${FAKE_NUMSTAT:-}" ;;
  *"diff --name-only"*)     printf '%s\n' "${FAKE_NAMES:-}" ;;
  *"rev-parse HEAD"*)
    n=$((n+1)); echo "$n" > "$cf"
    if (( n <= ${FAKE_BASECALLS:-1} )); then echo "${FAKE_BASE_SHA:-ba5eba5}";
    elif (( n == 2 )); then echo "${FAKE_HEAD_SHA:-c0ffee1deadbeef0000000000000000000000}";
    else echo "${FAKE_HEAD2_SHA:-f00dd00dbabe0000000000000000000000}"; fi ;;
esac
exit 0
STUB
cat > "$T/bin/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "$CALLS"
case "${1:-} ${2:-}" in
  "agent get")
    printf '{"result":{"agent":{"agent_status":"%s","cwd":"%s","pane_id":"%%1"}}}\n' \
      "${FAKE_STATUS:-working}" "${FAKE_CWD:-$HOME}" ;;
  "agent read") printf '%s\n' "${FAKE_PANE:-working}" ;;
  "agent prompt")
    w=$3; content=''
    # builder reports carry the '## Lessons' tail (prompt-worker.sh exit 6
    # without it, rules/_all/brief.md); reviewer reports carry 'Verdict:'.
    case "$4" in
      *"phase: PLAN"*)   content=${FAKE_PLAN_MD:-$'fake plan: step 1\n\n## Lessons\n- none'} ;;
      *"phase: BUILD"*)  content=${FAKE_BUILD_MD:-$'fake build report: committed + tests pass\n\n## Lessons\n- none'} ;;
      *"phase: FIX"*)    content=${FAKE_FIX_MD:-$'fake fix report: findings addressed\n\n## Lessons\n- none'} ;;
      *"phase: REVIEW"*) content=${FAKE_REVIEW_MD:-'Verdict: clean'} ;;
    esac
    if [ -n "$content" ] && [ "${FAKE_SILENT:-0}" != 1 ]; then
      if [ "${FAKE_DELAY:-0}" != 0 ]; then
        ( sleep "$FAKE_DELAY"; printf '%s\n' "$content" > "$HOME/quaqua/runs/$w.md" ) &
      else
        printf '%s\n' "$content" > "$HOME/quaqua/runs/$w.md"
      fi
    fi
    exit "${FAKE_RC:-0}" ;;
  "pane split") printf '{"result":{"pane":{"pane_id":"%%1"}}}\n' ;;
  "pane wait-output") exit 1 ;;   # no codex trust dialog
esac
exit 0
STUB
chmod +x "$T/bin/git" "$T/bin/herdr"

cat > "$T/brief.md" <<'EOF'
Goal: implement the widget
Hard constraints: keep tests passing, no new deps
Current design (open to challenge): extend the existing renderer
Unknowns: none
Evidence: diff + test output
EOF

tele="$T/home/quaqua/runs/telemetry.jsonl"
runsdir="$T/home/quaqua/runs"

# run_pipe [VAR=val...] -- <pipeline.sh args>  →  rc, $T/out, $T/err, $T/calls
run_pipe() {
  local vars=()
  while (($#)) && [[ $1 != -- ]]; do vars+=("$1"); shift; done
  [[ ${1:-} == -- ]] && shift
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" GITN="$T/gitn" HOME="$T/home" PATH="$T/bin:$PATH" QUAQUA_TEST=1 GRID_SKIP=1 \
    PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
    PROMPT_WORKER_IDLE_SEC=9999 PROMPT_WORKER_MAX_SEC=3600 \
    env "${vars[@]}" bash "$src/lead/pipeline.sh" "$@" >"$T/out" 2>"$T/err" || rc=$?
}

# ── arg validation: refused before any spawn/prompt ─────────────────────────
reject() {  # reject <label> <stderr-match> [VAR=val...] -- <pipeline.sh args>
  local label=$1 match=$2; shift 2
  run_pipe "$@"
  [[ $rc -eq 1 ]] || fail "$label: exit $rc, want 1"
  grep -q "$match" "$T/err" || fail "$label: no '$match' on stderr: $(cat "$T/err")"
  ! grep -q 'agent prompt' "$T/calls" || fail "$label: herdr prompt called: $(cat "$T/calls")"
  echo "ok: $label -> exit 1 ($match)"
}

reject "no args"              '^# Usage:' --
reject "bad task chars"       '^# Usage:' -- Bad_Task "$T/repo" --brief "$T/brief.md"
reject "missing --brief"      'brief.*required' -- isl "$T/repo"
reject "missing brief file"   'brief.*required' -- isl "$T/repo" --brief "$T/nope.md"
printf 'just prose\n' > "$T/nobrief.md"
reject "brief lacks headers"  'Goal.*Hard constraints.*Current design' -- isl "$T/repo" --brief "$T/nobrief.md"
reject "same harness"         'different harnesses' -- isl "$T/repo" --brief "$T/brief.md" --builder bacdau --reviewer quansu
reject "same kind"            'different harnesses' -- isl "$T/repo" --brief "$T/brief.md" --builder quansu --reviewer quansu
reject "non-codex reviewer"   'codex kind' -- isl "$T/repo" --brief "$T/brief.md" --builder ni --reviewer sude
reject "bad --review"         'auto|always|skip' -- isl "$T/repo" --brief "$T/brief.md" --review maybe
reject "--rounds 0"           'integer >= 1' -- isl "$T/repo" --brief "$T/brief.md" --rounds 0
reject "builder name >20"     'exceeds 20 chars' -- abcdefghijklmnopq "$T/repo" --brief "$T/brief.md"
reject "reviewer name >20"    'exceeds 20 chars' -- abcdefghijklm "$T/repo" --brief "$T/brief.md"
reject "resume w/o state"     'no pipeline state' -- --resume nope "$T/repo"

# ── clean run: plan pause -> resume -> build -> auto review -> Verdict: clean
run_pipe FAKE_NUMSTAT=$'200\t10\tsrc/a.c\n5\t0\tsrc/b.c' \
         FAKE_NAMES=$'src/a.c\nsrc/b.c' \
         -- isl "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "isl run1: exit $rc, want 5 plan-ready: $(cat "$T/err")"
grep -q 'plan-ready' "$T/err" || fail "isl run1: no plan-ready notice"
[[ -f $runsdir/sude-isl.plan.md ]] || fail "isl run1: plan file not moved aside"
[[ ! -f $runsdir/sude-isl.md ]] || fail "isl run1: runs file not moved"
[[ -f $runsdir/pipeline-isl.state ]] || fail "isl run1: no state file"
grep -q '"task":"isl".*"status":"plan-ready"' "$tele" \
  || fail "isl run1: no plan-ready telemetry line: $(cat "$tele")"
grep -q 'agent start sude-isl --kind omp' "$T/calls" || fail "isl run1: builder not spawned as omp: $(cat "$T/calls")"
! grep -q 'quaqua-review' "$T/calls" || fail "isl run1: builder spawned read-only"
echo "ok: plan-ready -> exit 5, plan parked, state + telemetry written"

# refuse a second fresh start on the same slug (state exists)
run_pipe -- isl "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 1 ]] || fail "isl rerun: exit $rc, want 1"
grep -q 'already has state' "$T/err" || fail "isl rerun: no state error: $(cat "$T/err")"
echo "ok: duplicate slug -> exit 1, --resume hinted"

run_pipe FAKE_NUMSTAT=$'200\t10\tsrc/a.c\n5\t0\tsrc/b.c' \
         FAKE_NAMES=$'src/a.c\nsrc/b.c' \
         FAKE_REVIEW_MD=$'looks good\nVerdict: clean' \
         -- --resume isl "$T/repo"
[[ $rc -eq 0 ]] || fail "isl resume: exit $rc, want 0: $(cat "$T/err")"
grep -q 'status=clean' "$T/out" || fail "isl resume: stdout not clean: $(cat "$T/out")"
# reviewer spawned on the exact committed tip the gate measured (F2), read-only
grep -q 'worktree add -b quaqua/quansu-islr1 .* c0ffee1deadbeef' "$T/calls" \
  || fail "isl resume: reviewer not based on the gated head sha: $(cat "$T/calls")"
grep -q 'commit c0ffee1' "$runsdir/pipeline-isl.md" \
  || fail "isl resume: committed head sha not recorded in the report"
grep -q 'quaqua-review' "$T/calls" \
  || fail "isl resume: reviewer spawned without read-only profile"
! grep -q 'agent start sude-isl' "$T/calls" || fail "isl resume: builder respawned"
grep -q 'pane close' "$T/calls" || fail "isl resume: reviewer pane not closed"
grep -q 'worktree remove' "$T/calls" || fail "isl resume: reviewer worktree kept"
grep -q 'Verdict: clean' "$runsdir/pipeline-isl.md" || fail "isl: verdict not logged"
grep -q 'status: clean' "$runsdir/pipeline-isl.md" || fail "isl: no final status"
tail -1 "$tele" | grep -q '"status":"clean"' || fail "isl: last telemetry not clean"
tail -1 "$tele" | grep -q '"review":"run"' || fail "isl: telemetry review!=run"
tail -1 "$tele" | grep -q '"rounds":1' || fail "isl: telemetry rounds!=1"
echo "ok: clean -> review ran on builder branch (read-only), cleaned, telemetry clean"

# ── review skipped: small single-dir diff under the auto gate ───────────────
run_pipe -- skp "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "skp run1: exit $rc, want 5"
run_pipe FAKE_NUMSTAT=$'20\t5\tAGENTS.md' FAKE_NAMES='AGENTS.md' \
         -- --resume skp "$T/repo"
[[ $rc -eq 0 ]] || fail "skp resume: exit $rc"
grep -q 'status=clean' "$T/out" || fail "skp: stdout not clean"
! grep -q 'quansu' "$T/calls" || fail "skp: reviewer spawned despite skip: $(cat "$T/calls")"
tail -1 "$tele" | grep -q '"review":"skip"' || fail "skp: telemetry review!=skip"
tail -1 "$tele" | grep -q '"rounds":0' || fail "skp: telemetry rounds!=0"
echo "ok: auto gate under threshold -> review skipped, no reviewer spawned"

# ── rounds exhausted: findings both rounds -> blocked, review changed outcome
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         -- rex "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "rex run1: exit $rc, want 5"
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         FAKE_REVIEW_MD=$'src/a.c:10 wrong\nsrc/a.c:20 missing\nVerdict: 2 findings' \
         -- --resume rex "$T/repo"
[[ $rc -eq 0 ]] || fail "rex resume: exit $rc: $(cat "$T/err")"
grep -q 'status=blocked' "$T/out" || fail "rex: stdout not blocked: $(cat "$T/out")"
grep -q 'rounds exhausted' "$runsdir/pipeline-rex.md" || fail "rex: no exhausted note"
grep -q '## Blockers' "$runsdir/pipeline-rex.md" || fail "rex: no Blockers section"
[[ -f $runsdir/sude-rex.fix1.md ]] || fail "rex: fix1 file not parked"
[[ $(grep -c 'agent start quansu-rexr' "$T/calls") -eq 2 ]] \
  || fail "rex: want 2 fresh reviewers: $(grep 'agent start' "$T/calls")"
tail -1 "$tele" | grep -q '"status":"blocked"' || fail "rex: telemetry not blocked"
tail -1 "$tele" | grep -q '"rounds":2' || fail "rex: telemetry rounds!=2"
tail -1 "$tele" | grep -q '"review_changed_outcome":true' \
  || fail "rex: review_changed_outcome!=true"
echo "ok: rounds exhausted -> blocked + Blockers, 2 fresh reviewers, telemetry"

# ── disputed: builder answers findings with a Challenge -> stop as disputed ──
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         -- dsp "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "dsp run1: exit $rc, want 5"
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         FAKE_REVIEW_MD=$'src/a.c:10 wrong\nVerdict: 1 findings' \
         FAKE_FIX_MD=$'looked into it\n\n## Challenge\nfinding 1 is wrong: the spec says X (evidence)\n\n## Lessons\n- none\n' \
         -- --resume dsp "$T/repo"
[[ $rc -eq 0 ]] || fail "dsp resume: exit $rc"
grep -q 'status=disputed' "$T/out" || fail "dsp: stdout not disputed: $(cat "$T/out")"
grep -q '## Challenge' "$runsdir/pipeline-dsp.md" || fail "dsp: challenge not copied"
grep -q 'finding 1 is wrong' "$runsdir/pipeline-dsp.md" || fail "dsp: challenge text missing"
! grep -q 'agent start quansu-dspr2' "$T/calls" || fail "dsp: round-2 reviewer spawned"
tail -1 "$tele" | grep -q '"status":"disputed"' || fail "dsp: telemetry not disputed"
echo "ok: disputed -> Challenge copied into pipeline file, stops before round 2"

# ── watchdog exit propagates: silent builder -> exit 2, telemetry blocked ────
run_pipe FAKE_SILENT=1 PROMPT_WORKER_MAX_SEC=2 \
         -- wdg "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 2 ]] || fail "wdg: exit $rc, want 2 (watchdog propagation): $(cat "$T/err")"
grep -q 'WATCHDOG exit=2' "$T/err" || fail "wdg: no WATCHDOG line"
grep -q 'status: blocked' "$runsdir/pipeline-wdg.md" || fail "wdg: not logged blocked"
tail -1 "$tele" | grep -q '"status":"blocked"' || fail "wdg: telemetry not blocked"
echo "ok: watchdog exit 2 -> propagated, pipeline blocked, telemetry written"

# ── R3: missing Verdict line -> exit 6 at the wait, pipeline blocked ─────────
# prompt-worker.sh rejects the reviewer report before the pipeline can read it
# (R3), so the phase wait fails with watchdog exit 6 and pipeline.sh reports
# blocked like every other watchdog exit. The pipeline's own missing-Verdict
# check stays as a backstop for reports that arrive out of band.
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         -- mvd "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "mvd run1: exit $rc, want 5"
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         FAKE_REVIEW_MD='a report with no verdict at all' \
         -- --resume mvd "$T/repo"
[[ $rc -eq 6 ]] || fail "mvd resume: exit $rc, want 6 (malformed report): $(cat "$T/err")"
grep -q 'status=blocked' "$T/out" || fail "mvd: stdout not blocked: $(cat "$T/out")"
grep -q 'watchdog exit 6' "$runsdir/pipeline-mvd.md" \
  || fail "mvd: exit 6 not recorded in the pipeline file"
grep -q 'WATCHDOG malformed-report' "$T/err" || fail "mvd: no malformed-report line"
# the wait never returned a path, so the reviewer was never cleaned up
! grep -q 'pane close' "$T/calls" || fail "mvd: reviewer cleaned up despite the failed wait"
echo "ok: missing Verdict -> watchdog exit 6, pipeline blocked, no cleanup"

# ── --base passthrough: builder worktree starts at the given ref ─────────────
run_pipe -- bas "$T/repo" --brief "$T/brief.md" --base quaqua/integ
[[ $rc -eq 5 ]] || fail "bas: exit $rc, want 5"
grep -q 'rev-parse --verify .*quaqua/integ' "$T/calls" \
  || fail "bas: --base not verified: $(cat "$T/calls")"
grep -q 'worktree add -b quaqua/sude-bas .* quaqua/integ' "$T/calls" \
  || fail "bas: worktree not based on quaqua/integ: $(cat "$T/calls")"
echo "ok: --base -> builder worktree starts at quaqua/integ"

# ── grid: the pipeline arranges its own workers into the project tab ────────
# copied scripts dir so grid-workers.sh can be swapped for a logging stub;
# run_pipe2 does NOT set GRID_SKIP so the pipeline's own hook is observable.
# Every grid call must carry --only — a call without it is an inner
# spawn/cleanup hook leaking (it would sweep foreign workers in the CoS tab).
mkdir -p "$T/p2/scripts/lead" "$T/p2/scripts/worker"
cp "$src/lead/pipeline.sh" "$T/p2/scripts/lead/"
cp -R "$src/lib" "$T/p2/scripts/"
cp "$src/worker/prompt-worker.sh" "$src/worker/spawn-worker.sh" \
   "$src/worker/cleanup-worker.sh" "$T/p2/scripts/worker/"
cat > "$T/p2/scripts/worker/grid-workers.sh" <<'EOF'
#!/usr/bin/env bash
printf 'grid %s\n' "$*" >> "$CALLS"
if [ -n "${GRID_FAIL_ON:-}" ] && [[ $* == *"$GRID_FAIL_ON"* ]]; then exit 1; fi
exit "${GRID_FAKE_RC:-0}"
EOF
chmod +x "$T/p2/scripts/worker/grid-workers.sh"

run_pipe2() {  # like run_pipe, but against $T/p2/scripts and without GRID_SKIP
  local vars=()
  while (($#)) && [[ $1 != -- ]]; do vars+=("$1"); shift; done
  [[ ${1:-} == -- ]] && shift
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" GITN="$T/gitn" HOME="$T/home" PATH="$T/bin:$PATH" \
    PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
    PROMPT_WORKER_IDLE_SEC=9999 PROMPT_WORKER_MAX_SEC=3600 \
    env "${vars[@]}" bash "$T/p2/scripts/lead/pipeline.sh" "$@" >"$T/out" 2>"$T/err" || rc=$?
}
glines() { grep '^grid ' "$T/calls" || true; }

lineno() { grep -n "$1" "$T/calls" | head -1 | cut -d: -f1; }

run_pipe2 FAKE_NUMSTAT=$'200\t10\tsrc/a.c' FAKE_NAMES=$'src/a.c' \
          -- grd "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "grd run1: exit $rc, want 5: $(cat "$T/err")"
glines | grep -qx 'grid --tab repo-workers --only sude-grd' \
  || fail "grd run1: builder not gridded into repo-workers: $(glines)"
[[ $(lineno 'agent start sude-grd') -lt $(lineno '^grid ') ]] \
  || fail "grd run1: grid ran before the builder spawn: $(cat "$T/calls")"
! glines | grep -v -- '--only' | grep -q . \
  || fail "grd run1: unfiltered grid call leaked (inner hook not skipped): $(glines)"
echo "ok: builder spawn -> grid --tab repo-workers --only sude-grd"

run_pipe2 FAKE_NUMSTAT=$'200\t10\tsrc/a.c' FAKE_NAMES=$'src/a.c' \
          FAKE_REVIEW_MD=$'ok\nVerdict: clean' \
          -- --resume grd "$T/repo"
[[ $rc -eq 0 ]] || fail "grd resume: exit $rc: $(cat "$T/err")"
glines | grep -qx 'grid --tab repo-workers --only sude-grd,quansu-grdr1' \
  || fail "grd resume: reviewer spawn not gridded: $(glines)"
[[ $(glines | grep -cx 'grid --tab repo-workers --only sude-grd') -ge 1 ]] \
  || fail "grd resume: builder-only grid call after reviewer cleanup missing: $(glines)"
! glines | grep -v -- '--only' | grep -q . \
  || fail "grd resume: unfiltered grid call leaked: $(glines)"
echo "ok: reviewer spawn -> --only builder,reviewer; cleanup -> --only builder"

# GRID_TAB overrides the derived <repo-basename>-workers label
run_pipe2 GRID_TAB=my-tab -- gto "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "gto: exit $rc, want 5: $(cat "$T/err")"
glines | grep -qx 'grid --tab my-tab --only sude-gto' \
  || fail "gto: GRID_TAB not honoured: $(glines)"
! glines | grep -v -- '--only' | grep -q . \
  || fail "gto: unfiltered grid call leaked: $(glines)"
echo "ok: GRID_TAB overrides the workers tab label"

# a failing grid PAUSES the pipeline before any prompt (F12, MANDATORY rule)
run_pipe2 GRID_FAKE_RC=1 -- gfx "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "gfx: exit $rc, want 5 (grid pause): $(cat "$T/err")"
grep -q 'warning: grid-workers.sh failed' "$T/err" \
  || fail "gfx: no grid-failure warning: $(cat "$T/err")"
! grep -q 'agent prompt' "$T/calls" \
  || fail "gfx: worker prompted despite grid failure: $(cat "$T/calls")"
grep -q 'status: blocked' "$runsdir/pipeline-gfx.md" \
  || fail "gfx: pause not recorded as blocked"
run_pipe2 -- --resume gfx "$T/repo"
[[ $rc -eq 5 ]] || fail "gfx resume: exit $rc, want 5 plan-ready: $(cat "$T/err")"
grep -q 'agent prompt sude-gfx' "$T/calls" \
  || fail "gfx resume: plan prompt not sent after arrange"
echo "ok: grid failure pauses pre-prompt; resume after arranging continues"

# the reviewer-phase grid failure also pauses before the review prompt
run_pipe2 FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
          -- gfr "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "gfr run1: exit $rc, want 5"
run_pipe2 FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' GRID_FAIL_ON=quansu \
          FAKE_REVIEW_MD=$'ok\nVerdict: clean' -- --resume gfr "$T/repo"
[[ $rc -eq 5 ]] || fail "gfr: exit $rc, want 5 (reviewer-grid pause): $(cat "$T/err")"
grep -q 'grid-workers.sh failed' "$T/err" || fail "gfr: no grid warning"
! grep -q 'agent prompt quansu-gfr' "$T/calls" \
  || fail "gfr: reviewer prompted despite grid failure: $(cat "$T/calls")"
run_pipe2 FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
          FAKE_REVIEW_MD=$'ok\nVerdict: clean' -- --resume gfr "$T/repo"
[[ $rc -eq 0 ]] || fail "gfr resume: exit $rc: $(cat "$T/err")"
grep -q 'status=clean' "$T/out" || fail "gfr: not clean after arrange"
echo "ok: reviewer-spawn grid failure pauses pre-prompt; resume completes review"

# state binds the Lead exemption to this pipeline's orchestrator (F6)
run_pipe2 PIPELINE_LEAD=st6-lead HERDR_PANE_ID=w1:p9 -- st6 "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "st6 run1: exit $rc"
grep -q '^lead=st6-lead' "$runsdir/pipeline-st6.state" || fail "st6: lead not recorded"
grep -q '^lead_pane=w1:p9' "$runsdir/pipeline-st6.state" || fail "st6: lead_pane not recorded"
pid1=$(sed -n 's/^pipeline_pid=//p' "$runsdir/pipeline-st6.state")
run_pipe2 FAKE_NUMSTAT=$'20\t5\tAGENTS.md' FAKE_NAMES='AGENTS.md' -- --resume st6 "$T/repo"
pid2=$(sed -n 's/^pipeline_pid=//p' "$runsdir/pipeline-st6.state")
[[ -n $pid1 && -n $pid2 && $pid1 != "$pid2" ]] \
  || fail "st6: --resume did not refresh pipeline_pid ($pid1 -> $pid2)"
echo "ok: state carries lead/lead_pane; --resume refreshes pipeline_pid"

# ── F1: an unfinished phase report never advances (Outcome/## Blockers) ─────
run_pipe FAKE_PLAN_MD=$'plan v1\n\n## Blockers\n- cannot plan without the spec\n\n## Lessons\n- none\n' \
         -- pb1 "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 0 ]] || fail "pb1: exit $rc, want 0"
grep -q 'status=blocked' "$T/out" || fail "pb1: stdout not blocked: $(cat "$T/out")"
grep -q 'cannot plan without the spec' "$runsdir/pipeline-pb1.md" \
  || fail "pb1: blocker evidence not copied"
! grep -q 'phase: BUILD' "$T/calls" || fail "pb1: build prompted despite blocked plan"
echo "ok: blocked plan report -> blocked, evidence copied, no build prompt"

run_pipe -- pb2 "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "pb2 run1: exit $rc, want 5"
run_pipe FAKE_BUILD_MD=$'Outcome: BLOCKED\n\n## Blockers\n- implementation incomplete\n\n## Lessons\n- none\n' \
         -- --resume pb2 "$T/repo"
[[ $rc -eq 0 ]] || fail "pb2 resume: exit $rc"
grep -q 'status=blocked' "$T/out" || fail "pb2: stdout not blocked: $(cat "$T/out")"
grep -q 'implementation incomplete' "$runsdir/pipeline-pb2.md" || fail "pb2: evidence not copied"
! grep -q 'agent start quansu' "$T/calls" || fail "pb2: reviewer spawned on a blocked build"
tail -1 "$tele" | grep -q '"status":"blocked"' || fail "pb2: telemetry not blocked"
echo "ok: Outcome: BLOCKED build (nudge-style report) -> blocked, no reviewer"

run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' -- pb3 "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "pb3 run1: exit $rc, want 5"
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         FAKE_REVIEW_MD=$'src/a.c:1 wrong\nVerdict: 1 findings' \
         FAKE_FIX_MD=$'tried\n\n## Blockers\n- fix needs schema access\n\n## Lessons\n- none\n' \
         -- --resume pb3 "$T/repo"
[[ $rc -eq 0 ]] || fail "pb3 resume: exit $rc"
grep -q 'status=blocked' "$T/out" || fail "pb3: stdout not blocked: $(cat "$T/out")"
grep -q 'fix needs schema access' "$runsdir/pipeline-pb3.md" || fail "pb3: evidence not copied"
! grep -q 'agent start quansu-pb3r2' "$T/calls" || fail "pb3: round-2 reviewer spawned"
echo "ok: blocked fix report -> blocked, no next review round"

# ── F2: no committed snapshot / dirty tree -> resumable blocked, not clean ───
run_pipe -- ncm "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "ncm run1: exit $rc, want 5"
run_pipe FAKE_HEAD_SHA=ba5eba5 -- --resume ncm "$T/repo"   # tip == base: no commits
[[ $rc -eq 3 ]] || fail "ncm: exit $rc, want 3 (resumable block): $(cat "$T/out")"
grep -q 'status=blocked' "$T/out" || fail "ncm: not blocked"
grep -q 'produced no commits' "$runsdir/pipeline-ncm.md" || fail "ncm: no-commit reason missing"
! grep -q 'agent start quansu' "$T/calls" || fail "ncm: reviewer spawned with no commits"
echo "ok: zero commits after build -> resumable blocked, no reviewer"

run_pipe -- nop "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "nop run1: exit $rc, want 5"
run_pipe FAKE_HEAD_SHA=ba5eba5 FAKE_BUILD_MD=$'looked carefully\nOutcome: no changes\n\n## Lessons\n- none\n' \
         -- --resume nop "$T/repo"
[[ $rc -eq 0 ]] || fail "nop: exit $rc"
grep -q 'status=clean' "$T/out" || fail "nop: declared no-op did not finish clean: $(cat "$T/out")"
grep -q 'no commits.*no changes' "$runsdir/pipeline-nop.md" || fail "nop: no-op note missing"
! grep -q 'agent start quansu' "$T/calls" || fail "nop: reviewer spawned for a no-op"
echo "ok: explicit 'Outcome: no changes' -> clean, review skipped"

run_pipe -- dty "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "dty run1: exit $rc, want 5"
run_pipe FAKE_PORCELAIN=$' M src/a.c\n?? newfile.c' -- --resume dty "$T/repo"
[[ $rc -eq 3 ]] || fail "dty: exit $rc, want 3"
grep -q 'status=blocked' "$T/out" || fail "dty: not blocked"
grep -q 'uncommitted/untracked' "$runsdir/pipeline-dty.md" || fail "dty: dirty evidence not logged"
grep -q '?? newfile.c' "$runsdir/pipeline-dty.md" || fail "dty: untracked file not listed"
! grep -q 'agent start quansu' "$T/calls" || fail "dty: reviewer spawned on dirty tree"
echo "ok: dirty worktree (tracked + untracked) -> blocked, evidence logged"

run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' -- fxc "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "fxc run1: exit $rc, want 5"
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         FAKE_REVIEW_MD=$'src/a.c:1 wrong\nVerdict: 1 findings' \
         FAKE_HEAD2_SHA=c0ffee1deadbeef0000000000000000000000 \
         -- --resume fxc "$T/repo"   # post-fix tip identical to the reviewed tip
[[ $rc -eq 3 ]] || fail "fxc: exit $rc, want 3: $(cat "$T/out")"
grep -q 'no commits past' "$runsdir/pipeline-fxc.md" || fail "fxc: no-advance reason missing"
echo "ok: fix without a new commit -> resumable blocked (no-op fix is a Challenge)"

# ── F3: a reviewer Challenge reaches the Lead before any verdict ─────────────
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' -- rch "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "rch run1: exit $rc, want 5"
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         FAKE_REVIEW_MD=$'the diff contradicts the spec\n\n## Challenge\nspec section 3 requires Y, diff does X (evidence)\n\nVerdict: clean' \
         -- --resume rch "$T/repo"
[[ $rc -eq 0 ]] || fail "rch: exit $rc"
grep -q 'status=disputed' "$T/out" || fail "rch: reviewer challenge did not dispute: $(cat "$T/out")"
grep -q 'spec section 3 requires Y' "$runsdir/pipeline-rch.md" || fail "rch: challenge not copied"
grep -q 'pane close' "$T/calls" || fail "rch: reviewer not cleaned"
echo "ok: reviewer Challenge + clean verdict -> disputed, copied, reviewer cleaned"

run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' -- rchf "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "rchf run1: exit $rc, want 5"
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         FAKE_REVIEW_MD=$'## Challenge\npremise wrong\n\nVerdict: 2 findings' \
         -- --resume rchf "$T/repo"
[[ $rc -eq 0 ]] || fail "rchf: exit $rc"
grep -q 'status=disputed' "$T/out" || fail "rchf: challenge+findings not disputed"
echo "ok: reviewer Challenge + findings verdict -> still disputed"

# ── F5: a phase never gets more than the whole-pipeline remainder ────────────
run_pipe -- clk "$T/repo" --brief "$T/brief.md" --review skip
[[ $rc -eq 5 ]] || fail "clk run1: exit $rc, want 5"
perl -pi -e 's/^start_elapsed=.*/start_elapsed=7180/' "$runsdir/pipeline-clk.state"
run_pipe PIPELINE_MAX_SEC=7200 -- --resume clk "$T/repo"
grep -q 'budget 20s' "$T/err" \
  || fail "clk: phase budget not clamped to remainder (want 20s): $(cat "$T/err")"
[[ $rc -eq 0 ]] || fail "clk: exit $rc inside the allowance"
echo "ok: phase budget clamped to pipeline remainder (20s of 2700s)"

run_pipe -- clk2 "$T/repo" --brief "$T/brief.md" --review skip
[[ $rc -eq 5 ]] || fail "clk2 run1: exit $rc, want 5"
perl -pi -e 's/^start_elapsed=.*/start_elapsed=7190/' "$runsdir/pipeline-clk2.state"
run_pipe PIPELINE_MAX_SEC=7195 FAKE_DELAY=7 -- --resume clk2 "$T/repo"
[[ $rc -eq 4 ]] || fail "clk2: exit $rc, want 4 (wait over remainder): $(cat "$T/out")"
grep -q 'status=blocked' "$T/out" || fail "clk2: not blocked"
grep -q 'budget 5s' "$T/err" || fail "clk2: remainder budget not applied"
echo "ok: file landing past the total budget -> watchdog 4 + blocked, never clean"

# ── F7: a report predating the prompt is archived, never consumed ────────────
printf 'stale content from an old task\n' > "$runsdir/sude-sta.md"
run_pipe -- sta "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "sta: exit $rc, want 5"
ls "$runsdir"/sude-sta.md.stale.* >/dev/null 2>&1 || fail "sta: stale report not archived"
grep -q 'stale content' "$runsdir"/sude-sta.md.stale.* || fail "sta: archive lost content"
grep -q 'agent prompt sude-sta ' "$T/calls" || fail "sta: no fresh prompt after archive"
echo "ok: stale report archived before the fresh prompt"

run_pipe FAKE_SILENT=1 PROMPT_WORKER_MAX_SEC=2 -- fst "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 2 ]] || fail "fst run1: exit $rc, want 2: $(cat "$T/err")"
printf 'stale\n' > "$runsdir/sude-fst.md"
perl -e 'utime time-3600,time-3600,@ARGV or die "utime: $!"' "$runsdir/sude-fst.md"
run_pipe FAKE_SILENT=1 PROMPT_WORKER_MAX_SEC=2 -- --resume fst "$T/repo"
ls "$runsdir"/sude-fst.md.stale.* >/dev/null 2>&1 \
  || fail "fst: report older than the recorded prompt time not archived on resume"
echo "ok: resume archives a report older than the recorded prompt time"

# The paired positive case protects against BSD stat being misread as mtime 0:
# a report arriving after a paused wait must be consumed, not always archived.
run_pipe FAKE_SILENT=1 PROMPT_WORKER_MAX_SEC=2 -- cur "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 2 ]] || fail "cur run1: exit $rc, want 2"
printf 'fresh plan arriving after the prompt\n\n## Lessons\n- none\n' > "$runsdir/sude-cur.md"
run_pipe FAKE_SILENT=1 PROMPT_WORKER_MAX_SEC=2 -- --resume cur "$T/repo"
[[ $rc -eq 5 ]] || fail "cur resume: exit $rc, want 5 plan-ready: $(cat "$T/err")"
grep -q 'fresh plan' "$runsdir/sude-cur.plan.md" || fail "cur: fresh report not consumed"
! compgen -G "$runsdir/sude-cur.md.stale.*" >/dev/null || fail "cur: fresh report archived as stale"
! grep -q 'agent prompt' "$T/calls" || fail "cur: resumed wait re-prompted"
echo "ok: resume consumes a fresh post-prompt report without archiving or re-prompting"

# ── telemetry: every line valid jsonl with the lock §2 fields ────────────────
while IFS= read -r l; do
  jq -e '.ts and .task and .repo and .builder and .reviewer and .review
         and (.rounds|type=="number") and (.review_changed_outcome|type=="boolean")
         and .status and (.minutes|type=="number")' <<<"$l" >/dev/null \
    || fail "telemetry line malformed: $l"
done < "$tele"
echo "ok: telemetry -> every stop line is valid jsonl with all lock §2 fields"

# ══ real-Git fixture: snapshot binding + spawn resume (F2/F4) ══════════════
# Only herdr is stubbed; git is real, so worktrees, commits and SHAs behave
# like production. The stub makes the builder's commit while its prompt runs
# (FAKE_DO_COMMIT=<wt>), and can fail agent start/wait once per name.
mkdir -p "$T/rgbin" "$T/rg/repo"
git -C "$T/rg/repo" init -q
git -C "$T/rg/repo" -c user.email=t@t -c user.name=t commit -qm init --allow-empty
cat > "$T/rgbin/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "$CALLS"
case "${1:-} ${2:-}" in
  "agent get")
    printf '{"result":{"agent":{"agent_status":"working","cwd":"%s","pane_id":"%%1"}}}\n' "$HOME" ;;
  "agent read") printf 'working\n' ;;
  "agent prompt")
    w=$3; content=''
    # same report tails as the stub above ('## Lessons' for builders)
    case "$4" in
      *"phase: PLAN"*)   content=${FAKE_PLAN_MD:-$'fake plan\n\n## Lessons\n- none'} ;;
      *"phase: BUILD"*)  content=${FAKE_BUILD_MD:-$'fake build\n\n## Lessons\n- none'} ;;
      *"phase: FIX"*)    content=${FAKE_FIX_MD:-$'fake fix\n\n## Lessons\n- none'} ;;
      *"phase: REVIEW"*) content=${FAKE_REVIEW_MD:-'Verdict: clean'} ;;
    esac
    case "$4" in
      *"phase: BUILD"*|*"phase: FIX"*)
        if [ -n "${FAKE_DO_COMMIT:-}" ]; then
          ( cd "$FAKE_DO_COMMIT" && echo "$(date +%s)-$$" >> impl.txt \
            && git add impl.txt && git -c user.email=t@t -c user.name=t commit -qm impl ) || true
        fi
        [ -n "${FAKE_DIRTY:-}" ] && echo scratch >> "$FAKE_DO_COMMIT/dirty.txt" ;;
    esac
    if [ -n "$content" ] && [ "${FAKE_SILENT:-0}" != 1 ]; then
      printf '%s\n' "$content" > "$HOME/quaqua/runs/$w.md"
    fi
    exit "${FAKE_RC:-0}" ;;
  "agent start")
    if [ "$3" = "${FAKE_FAIL_START:-}" ] && [ ! -f "$T/failed-$3" ]; then
      : > "$T/failed-$3"; exit 1
    fi ;;
  "agent wait")
    if [ "$3" = "${FAKE_FAIL_WAIT:-}" ] && [ ! -f "$T/failedw-$3" ]; then
      : > "$T/failedw-$3"; exit 1
    fi ;;
  "pane split") printf '{"result":{"pane":{"pane_id":"%%1"}}}\n' ;;
  "pane wait-output") exit 1 ;;
esac
exit 0
STUB
chmod +x "$T/rgbin/herdr"

run_rg() {  # real-git run (only herdr stubbed); sets rc, $T/out, $T/err, $T/calls
  local vars=()
  while (($#)) && [[ $1 != -- ]]; do vars+=("$1"); shift; done
  [[ ${1:-} == -- ]] && shift
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/rgbin:$PATH" QUAQUA_TEST=1 GRID_SKIP=1 \
    PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
    PROMPT_WORKER_IDLE_SEC=9999 PROMPT_WORKER_MAX_SEC=3600 T="$T" \
    env "${vars[@]}" bash "$src/lead/pipeline.sh" "$@" >"$T/out" 2>"$T/err" || rc=$?
}

# F2: the reviewer worktree is created at exactly the committed tip the gate saw
run_rg -- rgb "$T/rg/repo" --brief "$T/brief.md" --review always
[[ $rc -eq 5 ]] || fail "rgb run1: exit $rc, want 5: $(cat "$T/err")"
wt_rgb="$T/rg/.wt/sude-rgb"
run_rg FAKE_DO_COMMIT="$wt_rgb" FAKE_REVIEW_MD=$'ok\nVerdict: clean' \
       -- --resume rgb "$T/rg/repo"
[[ $rc -eq 0 ]] || fail "rgb resume: exit $rc: $(cat "$T/err")"
grep -q 'status=clean' "$T/out" || fail "rgb: not clean: $(cat "$T/out")"
head_rgb=$(git -C "$wt_rgb" rev-parse HEAD)
grep -q "spawned on $head_rgb" "$runsdir/pipeline-rgb.md" \
  || fail "rgb: reviewer not spawned on the committed tip $head_rgb: $(cat "$runsdir/pipeline-rgb.md")"
grep -q 'review round 1: Verdict: clean' "$runsdir/pipeline-rgb.md" || fail "rgb: review did not run"
grep -q "commit ${head_rgb:0:7}" "$runsdir/pipeline-rgb.md" || fail "rgb: tip not recorded"
echo "ok: real git — reviewer spawned on the exact committed tip (gate SHA == base)"

# F2: uncommitted leftovers are missing from the reviewed snapshot -> blocked
run_rg -- rgd "$T/rg/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "rgd run1: exit $rc, want 5"
wt_rgd="$T/rg/.wt/sude-rgd"
run_rg FAKE_DO_COMMIT="$wt_rgd" FAKE_DIRTY=1 -- --resume rgd "$T/rg/repo"
[[ $rc -eq 3 ]] || fail "rgd: exit $rc, want 3 (dirty tree): $(cat "$T/err")"
grep -q 'status=blocked' "$T/out" || fail "rgd: not blocked"
grep -q 'uncommitted/untracked' "$runsdir/pipeline-rgd.md" || fail "rgd: dirty not logged"
echo "ok: real git — dirty worktree blocked (untracked file kept off the branch)"

# F2: a build with zero commits is blocked, not a clean no-op
run_rg -- rgn "$T/rg/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "rgn run1: exit $rc, want 5"
run_rg -- --resume rgn "$T/rg/repo"
[[ $rc -eq 3 ]] || fail "rgn: exit $rc, want 3: $(cat "$T/err")"
grep -q 'produced no commits' "$runsdir/pipeline-rgn.md" || fail "rgn: no-commit reason missing"
echo "ok: real git — zero commits after build is blocked"

run_rg -- rgp "$T/rg/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "rgp run1: exit $rc, want 5"
run_rg FAKE_BUILD_MD=$'checked\nOutcome: no changes\n\n## Lessons\n- none\n' -- --resume rgp "$T/rg/repo"
[[ $rc -eq 0 ]] || fail "rgp: exit $rc"
grep -q 'status=clean' "$T/out" || fail "rgp: declared no-op not clean: $(cat "$T/out")"
echo "ok: real git — declared no-op finishes clean, no reviewer spawned"

# F4: agent-start failure keeps the worktree; --resume finishes the startup
run_rg FAKE_FAIL_START=sude-rsf -- rsf "$T/rg/repo" --brief "$T/brief.md"
[[ $rc -eq 3 ]] || fail "rsf: exit $rc, want 3 (spawn failure): $(cat "$T/err")"
[[ -d $T/rg/.wt/sude-rsf ]] || fail "rsf: worktree should survive the failed start"
run_rg -- --resume rsf "$T/rg/repo"
[[ $rc -eq 5 ]] || fail "rsf resume: exit $rc, want 5 plan-ready: $(cat "$T/err")"
grep -q 'agent start sude-rsf' "$T/calls" || fail "rsf: agent start not retried"
grep -q 'reusing it' "$T/err" || fail "rsf: worktree reuse note missing: $(cat "$T/err")"
echo "ok: failed agent start -> resume reuses worktree, retries start, prompts plan"

# F4 (reviewer): wait-ready failure keeps the worktree; --resume retries
run_rg -- rsv "$T/rg/repo" --brief "$T/brief.md" --review always
[[ $rc -eq 5 ]] || fail "rsv run1: exit $rc, want 5"
run_rg FAKE_DO_COMMIT="$T/rg/.wt/sude-rsv" FAKE_FAIL_WAIT=quansu-rsvr1 \
       -- --resume rsv "$T/rg/repo"
[[ $rc -eq 3 ]] || fail "rsv: exit $rc, want 3 (reviewer spawn failure): $(cat "$T/err")"
[[ -d $T/rg/.wt/quansu-rsvr1 ]] || fail "rsv: reviewer worktree should survive"
run_rg FAKE_REVIEW_MD=$'ok\nVerdict: clean' -- --resume rsv "$T/rg/repo"
[[ $rc -eq 0 ]] || fail "rsv resume: exit $rc: $(cat "$T/err")"
grep -q 'status=clean' "$T/out" || fail "rsv: not clean after reviewer retry"
echo "ok: reviewer spawn failure -> resume reuses worktree, reviews, finishes clean"

echo "PASS"
