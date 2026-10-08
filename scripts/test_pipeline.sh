#!/usr/bin/env bash
# Sandbox test for pipeline.sh: arg validation, the plan-ready pause (exit 5)
# + --resume, the auto review gate (>150 changed lines or >1 top-level dir),
# clean / disputed / rounds-exhausted / missing-Verdict outcomes, watchdog
# exit propagation, and one telemetry.jsonl line per stop. git and herdr are
# stubbed on PATH (they log calls; herdr fabricates runs files from FAKE_*
# vars keyed on the 'Pipeline phase: X' marker in the prompt). HOME is a temp
# dir. No real herdr, agent, repo, or home touched.
# Usage: scripts/test_pipeline.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-pipeline.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── sandbox: stub git/herdr, temp HOME/quaqua/runs, dummy repo + brief ──────
mkdir -p "$T/bin" "$T/repo" "$T/home/quaqua/runs"
cat > "$T/bin/git" <<'STUB'
#!/usr/bin/env bash
echo "git $*" >> "$CALLS"
case " $* " in
  *"diff --numstat"*)       printf '%s\n' "${FAKE_NUMSTAT:-}" ;;
  *"diff --name-only"*)     printf '%s\n' "${FAKE_NAMES:-}" ;;
  *"rev-parse --short HEAD"*) echo "${FAKE_COMMIT_SHA:-c0ffee1}" ;;
  *"rev-parse HEAD"*)       echo "${FAKE_BASE_SHA:-ba5eba5}" ;;
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
    case "$4" in
      *"phase: PLAN"*)   content=${FAKE_PLAN_MD:-fake plan: step 1} ;;
      *"phase: BUILD"*)  content=${FAKE_BUILD_MD:-fake build report: committed + tests pass} ;;
      *"phase: FIX"*)    content=${FAKE_FIX_MD:-fake fix report: findings addressed} ;;
      *"phase: REVIEW"*) content=${FAKE_REVIEW_MD:-'Verdict: clean'} ;;
    esac
    if [ -n "$content" ] && [ "${FAKE_SILENT:-0}" != 1 ]; then
      printf '%s\n' "$content" > "$HOME/quaqua/runs/$w.md"
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
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" GRID_SKIP=1 \
    PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
    PROMPT_WORKER_IDLE_SEC=9999 PROMPT_WORKER_MAX_SEC=3600 \
    env "${vars[@]}" bash "$here/pipeline.sh" "$@" >"$T/out" 2>"$T/err" || rc=$?
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
grep -q 'agent start sude-isl --kind pi' "$T/calls" || fail "isl run1: builder not spawned as pi: $(cat "$T/calls")"
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
# reviewer spawned on the builder branch, read-only profile, then cleaned
grep -q 'worktree add -b quaqua/quansu-islr1 .* quaqua/sude-isl' "$T/calls" \
  || fail "isl resume: reviewer not based on builder branch: $(cat "$T/calls")"
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
         FAKE_FIX_MD=$'looked into it\n\n## Challenge\nfinding 1 is wrong: the spec says X (evidence)\n' \
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

# ── missing Verdict line -> blocked (reviewer broke the contract) ────────────
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         -- mvd "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "mvd run1: exit $rc, want 5"
run_pipe FAKE_NUMSTAT=$'300\t0\tsrc/a.c' FAKE_NAMES='src/a.c' \
         FAKE_REVIEW_MD='a report with no verdict at all' \
         -- --resume mvd "$T/repo"
[[ $rc -eq 0 ]] || fail "mvd resume: exit $rc"
grep -q 'status=blocked' "$T/out" || fail "mvd: stdout not blocked"
grep -q 'Verdict' "$runsdir/pipeline-mvd.md" || fail "mvd: no verdict complaint logged"
grep -q 'pane close' "$T/calls" || fail "mvd: reviewer not cleaned after bad verdict"
echo "ok: missing Verdict -> blocked, reviewer still cleaned up"

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
mkdir -p "$T/p2/scripts"
cp "$here/pipeline.sh" "$here/prompt-worker.sh" "$here/spawn-worker.sh" \
   "$here/cleanup-worker.sh" "$T/p2/scripts/"
cat > "$T/p2/scripts/grid-workers.sh" <<'EOF'
#!/usr/bin/env bash
printf 'grid %s\n' "$*" >> "$CALLS"
exit "${GRID_FAKE_RC:-0}"
EOF
chmod +x "$T/p2/scripts/grid-workers.sh"

run_pipe2() {  # like run_pipe, but against $T/p2/scripts and without GRID_SKIP
  local vars=()
  while (($#)) && [[ $1 != -- ]]; do vars+=("$1"); shift; done
  [[ ${1:-} == -- ]] && shift
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
    PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
    PROMPT_WORKER_IDLE_SEC=9999 PROMPT_WORKER_MAX_SEC=3600 \
    env "${vars[@]}" bash "$T/p2/scripts/pipeline.sh" "$@" >"$T/out" 2>"$T/err" || rc=$?
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

# a failing grid never fails the pipeline (best-effort hook)
run_pipe2 GRID_FAKE_RC=1 -- gfx "$T/repo" --brief "$T/brief.md"
[[ $rc -eq 5 ]] || fail "gfx: exit $rc, want 5 despite grid failure: $(cat "$T/err")"
grep -q 'warning: grid-workers.sh failed' "$T/err" \
  || fail "gfx: no grid-failure warning: $(cat "$T/err")"
echo "ok: grid failure warns, pipeline continues"

# ── telemetry: every line valid jsonl with the lock §2 fields ────────────────
while IFS= read -r l; do
  jq -e '.ts and .task and .repo and .builder and .reviewer and .review
         and (.rounds|type=="number") and (.review_changed_outcome|type=="boolean")
         and .status and (.minutes|type=="number")' <<<"$l" >/dev/null \
    || fail "telemetry line malformed: $l"
done < "$tele"
echo "ok: telemetry -> every stop line is valid jsonl with all lock §2 fields"

echo "PASS"
