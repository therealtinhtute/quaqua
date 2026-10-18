#!/usr/bin/env bash
# Sandbox test for prompt-worker.sh: arg validation, the exact herdr prompt
# text, waiting for ~/quaqua/runs/<name>.md after herdr returns early, the
# PROMPT_WORKER_MAX_SEC cap, stale-report archiving on a new task, byte-
# stability before a report counts as complete, the stdout path-only
# contract, and budget resolution (--check-brief, brief `Time budget:`,
# PROMPT_WORKER_BUDGET_SEC override). herdr is stubbed on PATH (it logs calls
# and fabricates the runs file via FAKE_WRITE/FAKE_DELAY/FAKE_PARTS); HOME is a
# temp dir. No real herdr, agent, or home touched.
# Usage: tests/test_prompt_worker.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/.." && pwd)/scripts"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-prompt-worker.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── sandbox: stub herdr, temp HOME/quaqua/runs ──────────────────────────────
mkdir -p "$T/bin" "$T/home/quaqua/runs"
cat > "$T/bin/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "$CALLS"
# A well-formed builder report: rules/_all/brief.md requires the '## Lessons'
# tail (prompt-worker.sh exits 6 without it). FAKE_CONTENT overrides it for
# the malformed/reviewer cases.
content=${FAKE_CONTENT:-$'fake summary\n\n## Lessons\n- none'}
case "${1:-} ${2:-}" in
  "api snapshot")
    echo '{"snapshot":{"workspaces":[],"tabs":[],"panes":[],"layouts":[],"agents":[]}}'
    exit 0 ;;
  "agent get")
    printf '{"result":{"agent":{"agent_status":"%s","cwd":"%s","pane_id":"%s"}}}\n' \
      "${FAKE_STATUS:-working}" "${FAKE_CWD:-$HOME}" "${FAKE_WATCHER_PANE:-%1}"
    exit 0 ;;
  "agent read")
    if [ "${FAKE_ROTATE:-0}" = 1 ]; then
      n=$(cat "$HOME/.rotate" 2>/dev/null || echo 0); n=$(( n + 1 ))
      printf '%s\n' "$n" > "$HOME/.rotate"
      s=''; m=$n
      while [ "$m" -gt 0 ]; do
        printf -v ch "\\$(printf '%03o' $(( 97 + m % 26 )))"
        s="$s$ch"; m=$(( m / 26 ))
      done
      printf 'tool line %s\n' "$s"
    else
      printf '%s\n' "${FAKE_PANE:-some pane text}"
    fi
    exit 0 ;;
  "agent prompt")
    [ -n "${FAKE_STDOUT:-}" ] && printf '%s\n' "$FAKE_STDOUT"
    [ "${FAKE_WRITE:-0}" = 1 ] && printf '%s\n' "$content" > "$HOME/quaqua/runs/$FAKE_NAME.md"
    if [ "${FAKE_PARTS:-0}" = 1 ]; then
      ( printf 'part1\n\n## Lessons\n- none\n' > "$HOME/quaqua/runs/$FAKE_NAME.md"; sleep 0.2
        echo part2 >> "$HOME/quaqua/runs/$FAKE_NAME.md" ) &
    fi
    if [ "${FAKE_DELAY:-0}" != 0 ]; then
      ( sleep "$FAKE_DELAY"; printf '%s\n' "$content" > "$HOME/quaqua/runs/$FAKE_NAME.md" ) &
    fi
    exit "${FAKE_RC:-0}" ;;
esac
exit "${FAKE_RC:-0}"
STUB
chmod +x "$T/bin/herdr"
CLK=(PROMPT_WORKER_CLOCK_FILE="$T/clock" PROMPT_WORKER_SLEEP_CMD="$T/bin/advance-clock")
printf '%s\n' 1700000000 > "$T/clock"
cat > "$T/bin/advance-clock" <<'ADV'
#!/usr/bin/env bash
n=$(cat "$PROMPT_WORKER_CLOCK_FILE")
printf '%s\n' "$(( n + ${1:-1} ))" > "$PROMPT_WORKER_CLOCK_FILE"
ADV
chmod +x "$T/bin/advance-clock"


prompt() {  # prompt <name> <task> [VAR=val...]; sets rc, $T/out, $T/err, $T/calls
  local name=$1 task=$2; shift 2
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" FAKE_NAME="$name" \
    env "$@" "${QUAQUA_BIN:-quaqua}" worker prompt "$name" "$task" >"$T/out" 2>"$T/err" || rc=$?
}

reject() {  # reject <label> [args...]
  local label=$1; shift
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
    "${QUAQUA_BIN:-quaqua}" worker prompt "$@" >"$T/out" 2>"$T/err" || rc=$?
  [[ $rc -eq 1 ]] || fail "$label: exit $rc, want 1"
  grep -q '^# Usage:' "$T/err" || fail "$label: no usage on stderr"
  echo "ok: $label -> exit 1, usage on stderr"
}

# resume <name> [VAR=val...] — --resume the wait (same capture as prompt)
resume() {
  local name=$1; shift
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
    env "$@" "${QUAQUA_BIN:-quaqua}" worker prompt --resume "$name" >"$T/out" 2>"$T/err" || rc=$?
}

# malformed <label> <name> <missing> — exit 6 + the WATCHDOG line, no path
malformed() {
  local label=$1 name=$2 missing=$3
  [[ $rc -eq 6 ]] || fail "$label: exit $rc, want 6: $(cat "$T/err")"
  grep -q 'WATCHDOG malformed-report' "$T/err" \
    || fail "$label: no 'WATCHDOG malformed-report' on stderr: $(cat "$T/err")"
  grep -qF "missing=$missing" "$T/err" || fail "$label: wrong missing= field: $(cat "$T/err")"
  [[ ! -s $T/out ]] || fail "$label: printed a path for a malformed report: $(cat "$T/out")"
  echo "ok: $label -> exit 6, WATCHDOG malformed-report, no path on stdout"
}

brief='Goal: do the thing
Hard constraints: keep tests passing
Current design (open to challenge): via the existing helper'

reject_brief() {  # reject_brief <label> <task>: missing headers -> exit 1
  local label=$1 task=$2
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
    "${QUAQUA_BIN:-quaqua}" worker prompt sude-brief "$task" >"$T/out" 2>"$T/err" || rc=$?
  [[ $rc -eq 1 ]] || fail "$label: exit $rc, want 1"
  grep -q 'Goal.*Hard constraints.*Current design' "$T/err" \
    || fail "$label: no brief error on stderr: $(cat "$T/err")"
  echo "ok: $label -> exit 1, brief error on stderr"
}

# ── arg validation: rejected before herdr is called ──────────────────────────
: > "$T/calls"
reject "no args"
reject "one arg" sude-x
reject "bad name chars" 'bad_name' "do the thing"
reject "empty task" sude-x ""
[[ ! -s $T/calls ]] || fail "rejected args still called herdr: $(cat "$T/calls")"

# ── brief check: tasks missing any of the 3 headers rejected before herdr ────
reject_brief "no headers" "fix the flaky test"
reject_brief "missing constraints" "Goal: x
Current design (open to challenge): y"
reject_brief "missing current design" "Goal: x
Hard constraints: y"
reject_brief "missing goal" "Hard constraints: y
Current design: z"
[[ ! -s $T/calls ]] || fail "non-brief task still called herdr: $(cat "$T/calls")"

# ── D4: --check-brief validates + prints the resolved budget, no herdr ──────
check_brief() {  # check_brief <task> [VAR=val...]; sets rc, $T/out, $T/err
  local task=$1; shift
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
    env "$@" "${QUAQUA_BIN:-quaqua}" worker prompt --check-brief "$task" >"$T/out" 2>"$T/err" || rc=$?
}
check_brief "$brief"
[[ $rc -eq 0 ]] || fail "check-brief valid: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == 'budget=2700s source=default' ]] \
  || fail "check-brief valid: wrong budget line: $(cat "$T/out")"
check_brief "$brief
Time budget: 25 min"
[[ $rc -eq 0 ]] || fail "check-brief brief-budget: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == 'budget=1500s source=brief' ]] \
  || fail "check-brief brief-budget: 25 min not parsed: $(cat "$T/out")"
check_brief "$brief
Time budget: 25 min" PROMPT_WORKER_BUDGET_SEC=60
[[ "$(cat "$T/out")" == 'budget=60s source=env' ]] \
  || fail "check-brief env override: $(cat "$T/out")"
check_brief "fix the flaky test"
[[ $rc -eq 1 ]] || fail "check-brief invalid: exit $rc, want 1"
grep -q 'Goal.*Hard constraints.*Current design' "$T/err" \
  || fail "check-brief invalid: no brief error: $(cat "$T/err")"
rc=0; CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
  "${QUAQUA_BIN:-quaqua}" worker prompt --check-brief >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 1 ]] || fail "check-brief no arg: exit $rc, want 1"
[[ ! -s $T/calls ]] || fail "--check-brief called herdr: $(cat "$T/calls")"
echo "ok: --check-brief -> validates headers, resolves budget (brief/env/default), no herdr"

# ── D3: the brief's Time budget: sets the watch budget, env overrides it ────
prompt sude-bbrief "$brief
Time budget: 25 min" PROMPT_WORKER_MAX_SEC=2 PROMPT_WORKER_POLL_SEC=1
[[ $rc -eq 2 ]] || fail "sude-bbrief: exit $rc, want 2 (cap): $(cat "$T/err")"
grep -q 'budget 1500s' "$T/err" \
  || fail "sude-bbrief: brief budget not used: $(cat "$T/err")"
[[ "$(cat "$T/home/quaqua/runs/.sude-bbrief.budget")" == 1500 ]] \
  || fail "sude-bbrief: budget sidecar not persisted"
prompt sude-bover "$brief
Time budget: 25 min" PROMPT_WORKER_BUDGET_SEC=3 PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-bover: exit $rc, want 4 (env budget 3s): $(cat "$T/err")"
grep -q 'budget 3s' "$T/err" || fail "sude-bover: env did not override brief: $(cat "$T/err")"
echo "ok: brief Time budget -> watch budget; PROMPT_WORKER_BUDGET_SEC overrides"


# ── happy path: file written during the prompt, fresh report returned ──────
prompt sude-quick "$brief" FAKE_WRITE=1
[[ $rc -eq 0 ]] || fail "sude-quick: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/sude-quick.md" ]] \
  || fail "sude-quick: stdout not the runs path: $(cat "$T/out")"
grep -qF "herdr agent prompt sude-quick $brief. When done, write summary (changes, test cmd+result, risks) to a temp file and rename it to ~/quaqua/runs/sude-quick.md (publish it atomically, never a partial file); reply only the path. --wait --timeout 1800000" "$T/calls" \
  || fail "sude-quick: wrong herdr call: $(cat "$T/calls")"
grep -qF -- '--wait --timeout 1800000' "$T/calls" || fail "sude-quick: no --wait/--timeout"
[[ "$(cat "$T/home/quaqua/runs/sude-quick.md")" == *"fake summary"* ]] \
  || fail "sude-quick: returned file is not the fresh report"
echo "ok: sude-quick -> prompt sent verbatim, runs path printed"

# ── a pre-existing report is stale: archived, a fresh prompt is sent ─────────
echo "OLD leftover report" > "$T/home/quaqua/runs/sude-stale.md"
prompt sude-stale "$brief" FAKE_WRITE=1
[[ $rc -eq 0 ]] || fail "sude-stale: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/home/quaqua/runs/sude-stale.md")" == *"fake summary"* ]] \
  || fail "sude-stale: stale file consumed instead of the fresh report"
ls "$T"/home/quaqua/runs/sude-stale.md.stale.* >/dev/null 2>&1 \
  || fail "sude-stale: old report not archived"
grep -q 'OLD leftover report' "$T"/home/quaqua/runs/sude-stale.md.stale.* \
  || fail "sude-stale: archive lost the old content"
grep -q 'agent prompt sude-stale' "$T/calls" || fail "sude-stale: no fresh prompt sent"
echo "ok: sude-stale -> old report archived, fresh prompt + fresh report"

# ── a slow multi-write publisher: only the complete file counts ──────────────
prompt sude-parts "$brief" FAKE_PARTS=1 PROMPT_WORKER_POLL_SEC=1
[[ $rc -eq 0 ]] || fail "sude-parts: exit $rc, want 0: $(cat "$T/err")"
grep -q 'part2' "$T/home/quaqua/runs/sude-parts.md" \
  || fail "sude-parts: report consumed mid-write (part2 missing)"
echo "ok: sude-parts -> waited for a byte-stable file before accepting"

# ── herdr's CLI response never reaches stdout: path only (F13) ───────────────
prompt sude-noisy "$brief" FAKE_WRITE=1 FAKE_STDOUT='HERDR_SUBMISSION_RESPONSE'
[[ $rc -eq 0 ]] || fail "sude-noisy: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/sude-noisy.md" ]] \
  || fail "sude-noisy: stdout polluted beyond the path: $(cat "$T/out")"
grep -q 'HERDR_SUBMISSION_RESPONSE' "$T/home/quaqua/runs/.sude-noisy.submit.log" \
  || fail "sude-noisy: submit response not captured in the log"
echo "ok: sude-noisy -> stdout stays the report path, CLI response goes to the submit log"

# ── early return: herdr exits before the file exists; script keeps waiting ──
prompt sude-slow "Goal: slow task
Hard constraints: none
Current design: poll" FAKE_DELAY=1 PROMPT_WORKER_POLL_SEC=1
[[ $rc -eq 0 ]] || fail "sude-slow: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/sude-slow.md" ]] \
  || fail "sude-slow: stdout not the runs path: $(cat "$T/out")"
grep -q "waiting for $T/home/quaqua/runs/sude-slow.md" "$T/err" \
  || fail "sude-slow: no waiting notice on stderr"
echo "ok: sude-slow -> kept polling after herdr returned, printed runs path"

# ── early return AND herdr nonzero: still waits for the file ────────────────
prompt sude-flaky "Goal: flaky herdr
Hard constraints: none
Current design: retry" FAKE_DELAY=1 FAKE_RC=1 PROMPT_WORKER_POLL_SEC=1
[[ $rc -eq 0 ]] || fail "sude-flaky: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/sude-flaky.md" ]] \
  || fail "sude-flaky: stdout not the runs path"
echo "ok: sude-flaky -> nonzero herdr still waited, printed runs path"

# ── cap: no file within PROMPT_WORKER_MAX_SEC -> nonzero, error on stderr ───
prompt sude-never "Goal: never finishes
Hard constraints: none
Current design: cap" PROMPT_WORKER_MAX_SEC=2 PROMPT_WORKER_POLL_SEC=1
[[ $rc -eq 2 ]] || fail "sude-never: exit $rc, want 2 (out: $(cat "$T/out") err: $(cat "$T/err"))"
[[ ! -s $T/out ]] || fail "sude-never: printed a path on timeout: $(cat "$T/out")"
grep -q "error: no $T/home/quaqua/runs/sude-never.md" "$T/err" \
  || fail "sude-never: no timeout error on stderr"
echo "ok: sude-never -> exit 2 after cap, no path printed"

# ── R3: report-shape gate (rules/_all/brief.md) ─────────────────────────────
# The class comes from the spawn metadata runs/<name>.srt.json, never from the
# brief text: this very suite's brief is a builder brief and would still say
# nothing about which tail is required.

# builder (no metadata at all) without '## Lessons' -> exit 6
prompt sude-noles "Goal: build it
Hard constraints: none
Current design: x" FAKE_WRITE=1 \
  FAKE_CONTENT=$'work done\n\nVerdict: clean'   # a Verdict tail is NOT a builder tail
malformed "builder missing Lessons" sude-noles "'## Lessons' section"

# builder with the section -> exit 0
prompt sude-less "Goal: build it
Hard constraints: none
Current design: x" FAKE_WRITE=1 \
  FAKE_CONTENT=$'work done\n\n## Lessons\n- none'
[[ $rc -eq 0 ]] || fail "builder with Lessons: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/sude-less.md" ]] \
  || fail "builder with Lessons: stdout not the runs path"
echo "ok: builder report with '## Lessons' -> exit 0"

# reviewer (srt.json review:true) without 'Verdict:' -> exit 6
printf '{"name":"suhuynh-rv1","kind":"suhuynh","review":true}\n' \
  > "$T/home/quaqua/runs/suhuynh-rv1.srt.json"
prompt suhuynh-rv1 "Goal: review it
Hard constraints: read-only
Current design: diff" FAKE_WRITE=1 \
  FAKE_CONTENT=$'findings here\n\n## Lessons\n- none'   # a Lessons tail is NOT a reviewer tail
malformed "reviewer missing Verdict" suhuynh-rv1 "'Verdict:' line"

# reviewer with 'Verdict:' -> exit 0 (indented and trailing-space forms count)
printf '{"name":"suhuynh-rv2","kind":"suhuynh","review":true}\n' \
  > "$T/home/quaqua/runs/suhuynh-rv2.srt.json"
prompt suhuynh-rv2 "Goal: review it
Hard constraints: read-only
Current design: diff" FAKE_WRITE=1 \
  FAKE_CONTENT=$'findings here\n  Verdict: 2 findings'
[[ $rc -eq 0 ]] || fail "reviewer with Verdict: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/suhuynh-rv2.md" ]] \
  || fail "reviewer with Verdict: stdout not the runs path"
echo "ok: reviewer report with 'Verdict:' -> exit 0"

# metadata that says review:false is a builder report
printf '{"name":"sude-nof","kind":"sude","review":false}\n' \
  > "$T/home/quaqua/runs/sude-nof.srt.json"
prompt sude-nof "Goal: build it
Hard constraints: none
Current design: x" FAKE_WRITE=1 \
  FAKE_CONTENT=$'work done\nVerdict: clean'
malformed "review:false is a builder" sude-nof "'## Lessons' section"

# --resume re-validates: the malformed file still fails, then a rewritten
# report (temp+rename) is accepted without a fresh prompt
resume sude-noles
malformed "--resume on a still-malformed report" sude-noles "'## Lessons' section"
! grep -q 'agent prompt' "$T/calls" || fail "--resume re-prompted: $(cat "$T/calls")"
printf '%s\n' 'rewritten' '' '## Lessons' '- none' \
  > "$T/home/quaqua/runs/.sude-noles.tmp" \
  && mv "$T/home/quaqua/runs/.sude-noles.tmp" "$T/home/quaqua/runs/sude-noles.md"
resume sude-noles
[[ $rc -eq 0 ]] || fail "--resume after the rewrite: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/sude-noles.md" ]] \
  || fail "--resume after the rewrite: stdout not the runs path"
! grep -q 'agent prompt' "$T/calls" || fail "--resume after the rewrite re-prompted"
echo "ok: --resume re-validates the report (still 6, then accepted after the rewrite)"

# ── pre-prompt grid check: failing grid check blocks prompt ─────────────────
prompt tieude-1 "Goal: fail grid
Hard constraints: none
Current design: fail" HERDR_STUB_SNAPSHOT="$(dirname "$here")/tests/fixtures/herdr-grid/1-worker.json" GRID_CHECK_TIMEOUT_SEC=0 QUAQUA_OWNER=cos
[[ $rc -eq 1 ]] || fail "failing grid check: exit $rc, want 1"
grep -q 'grid check failed' "$T/err" || fail "failing grid check: no error message on stderr: $(cat "$T/err")"
! grep -q 'agent prompt' "$T/calls" || fail "failing grid check: worker prompted despite failure: $(cat "$T/calls")"
echo "ok: pre-prompt grid check failure -> exit 1, no prompt sent"

# ── omp watchdog: (a) false positive — status=idle but pane rotating -> no exit 3 ──
printf '%s\n' 1700000000 > "$T/clock"
prompt sude-omprot "$brief" "${CLK[@]}" FAKE_DELAY=0 FAKE_STATUS=idle FAKE_ROTATE=1 \
  PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_IDLE_SEC=2 \
  PROMPT_WORKER_BUDGET_SEC=5 PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-omprot: exit $rc, want 4 (budget exceeded, not idle exit 3): $(cat "$T/err")"
! grep -q 'exit=3' "$T/err" || fail "sude-omprot: exit 3 false-fired on active omp worker: $(cat "$T/err")"
echo "ok: omp active worker with status=idle -> idle_since reset on pane change, no exit 3"

# ── omp watchdog: (a) true positive — status=idle and static pane -> exit 3 ───
printf '%s\n' 1700000000 > "$T/clock"
prompt sude-ompidle "$brief" "${CLK[@]}" FAKE_DELAY=0 FAKE_STATUS=idle \
  FAKE_PANE='finished turn with no run file' \
  PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_IDLE_SEC=2 \
  PROMPT_WORKER_BUDGET_SEC=9999 PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 3 ]] || fail "sude-ompidle: exit $rc, want 3 (true idle): $(cat "$T/err")"
grep -q 'WATCHDOG exit=3' "$T/err" || fail "sude-ompidle: no exit=3 on stderr: $(cat "$T/err")"
grep -q 'agent idle' "$T/err" || fail "sude-ompidle: no agent idle reason: $(cat "$T/err")"
echo "ok: true idle without run file -> exit 3 after idle_sec"

# ── omp watchdog: (b) false positive — cat AGENTS.md in historical output -> no exit 5 ──
agents_output=$'6. `quaqua worker cleanup <name> <repo>` right after `runs/<name>.md` is read + its test re-run passes — every worker, no approval needed (branch is kept; merge still needs approval). Then `quaqua grid reconcile`.

## Rules
No push, branch delete, or main merge without anh iu'\''s approval. A Lead merges Peer branches into its project integration branch (acceptance). Max 4 task workers per orchestrator (the CoS and each Lead) — a pipeline counts as 2. One task per worker.
Only the CoS writes memory/rules.md, rules/, AGENTS.md; Leads propose changes by message.
projects/INDEX.md owner column says who drives each project (cos or the Lead name, <slug>-lead).
Every redirect — from anh iu or from the Auditor (cron, read-only, reports to the CoS only) — flows through the CoS; workers and Leads never take orders from reports.
────────────────────────────────────────────────────────────────────────────────────────────
 ✦ Gemini 3.8 Flash · high · ──────── 7% · ϟ 356k tpm · ⌥ quaqua/tieude-cleanfix2'

printf '%s\n' 1700000000 > "$T/clock"
prompt sude-agentfp "$brief" "${CLK[@]}" FAKE_DELAY=0 FAKE_STATUS=working \
  FAKE_PANE="$agents_output" \
  PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_BUDGET_SEC=2 \
  PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-agentfp: exit $rc, want 4 (budget, not exit 5): $(cat "$T/err")"
! grep -q 'exit=5' "$T/err" || fail "sude-agentfp: exit 5 false-fired on AGENTS.md text: $(cat "$T/err")"
echo "ok: AGENTS.md text with '\''needs approval'\'' -> no exit 5 false positive"

# ── omp watchdog: (b) true positive — real question prompt -> exit 5 ────────
printf '%s\n' 1700000000 > "$T/clock"
prompt sude-realask "$brief" "${CLK[@]}" FAKE_DELAY=0 FAKE_STATUS=working \
  FAKE_PANE=$'Choose an option:
> A
> B
Type something.' \
  PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_BUDGET_SEC=9999 \
  PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 5 ]] || fail "sude-realask: exit $rc, want 5 (true question): $(cat "$T/err")"
grep -q 'WATCHDOG exit=5' "$T/err" || fail "sude-realask: no exit=5 on stderr: $(cat "$T/err")"
grep -q 'question/permission prompt' "$T/err" || fail "sude-realask: wrong reason: $(cat "$T/err")"

printf '%s\n' 1700000000 > "$T/clock"
prompt sude-realyn "$brief" "${CLK[@]}" FAKE_DELAY=0 FAKE_STATUS=working \
  FAKE_PANE=$'Tool bash requires permission to run:
  git push origin main
Allow this tool? [y/N]' \
  PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_BUDGET_SEC=9999 \
  PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 5 ]] || fail "sude-realyn: exit $rc, want 5 (true y/n question): $(cat "$T/err")"
grep -q 'WATCHDOG exit=5' "$T/err" || fail "sude-realyn: no exit=5 on stderr: $(cat "$T/err")"
echo "ok: real question / permission prompt -> exit 5"

echo "PASS"
