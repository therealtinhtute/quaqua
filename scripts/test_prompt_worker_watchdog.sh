#!/usr/bin/env bash
# Sandbox test for prompt-worker.sh watchdog: --resume mode, --extend (D1),
# progress grace (D2), stall detection
# (exit 3), idle-without-runfile (exit 3), agent gone/unknown for
# AGENT_GONE_SEC (exit 3), time budget + one-shot nudge
# (exit 4), blocked/question (exit 5). herdr is stubbed on PATH: it logs calls,
# answers agent get/read/prompt from FAKE_* vars. HOME is a temp dir. No real
# herdr, agent, or home touched.
# Budgets in these tests are seconds (PROMPT_WORKER_BUDGET_SEC), so a grace of
# 25% of a 4s allowance is 1s — the code paths are the real ones, no clock stub.
# Usage: scripts/test_prompt_worker_watchdog.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-prompt-watchdog.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── sandbox: stub herdr, temp HOME/quaqua/runs ──────────────────────────────
mkdir -p "$T/bin" "$T/home/quaqua/runs"
cat > "$T/bin/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "$CALLS"
gone_now() {  # is this agent gone this call? (FAKE_GONE always; _AFTER n> ; _AT once)
  n=$(grep -c "^herdr agent get" "$CALLS" 2>/dev/null || true)
  [ "${FAKE_GONE:-0}" = 1 ] && return 0
  [ -n "${FAKE_GONE_AFTER:-}" ] && [ "$n" -gt "$FAKE_GONE_AFTER" ] && return 0
  [ -n "${FAKE_GONE_AT:-}" ] && [ "$n" -eq "$FAKE_GONE_AT" ] && return 0
  return 1
}
case "${1:-} ${2:-}" in
  "agent get")
    # real herdr prints error JSON on stderr, not stdout — keep it that way so
    # the watcher's stderr capture is what is actually tested
    gcalls=$(grep -c '^herdr agent get' "$CALLS" 2>/dev/null || true)
    if [ -n "${FAKE_BLANK_AT:-}" ] && [ "$gcalls" -eq "$FAKE_BLANK_AT" ]; then
      exit 1   # silent failure once -> status=unknown for a single poll
    fi
    if gone_now; then
      printf '{"error":{"code":"agent_not_found","message":"agent target %s not found"}}\n' "$3" >&2; exit 1
    fi
    printf '{"result":{"agent":{"agent_status":"%s","cwd":"%s","pane_id":"%s"}}}\n' \
      "${FAKE_STATUS:-working}" "${FAKE_CWD:-$HOME}" "${FAKE_WATCHER_PANE:-%1}" ;;
  "agent read")
    gone_now && { printf '{"error":{"code":"agent_not_found"}}\n' >&2; exit 1; }
    if [ "${FAKE_READ_ERR:-0}" = 1 ] && [[ $* == *recent-unwrapped* ]]; then
      printf '{"error":{"code":"agent_not_idle"}}\n'; exit 1
    fi
    if [ "${FAKE_ROTATE:-0}" = 1 ]; then
      # Each read runs in a new process. Hash its PID so digit normalization
      # cannot erase the changing text; no GNU date %N/md5sum dependency.
      printf 'tool line %s\n' "$(perl -MDigest::MD5=md5_hex -e 'print md5_hex($ARGV[0])' "$$")"
    else
      printf '%s\n' "${FAKE_PANE:-some pane text}"
    fi ;;
  "agent prompt")
    if [ "${FAKE_DELAY:-0}" != 0 ]; then
      ( sleep "$FAKE_DELAY"; echo "fake summary" > "$HOME/quaqua/runs/$FAKE_NAME.md" ) &
    fi
    exit "${FAKE_RC:-0}" ;;
esac
exit 0
STUB
chmod +x "$T/bin/herdr"

brief='Goal: do the thing
Hard constraints: keep tests passing
Current design (open to challenge): via the existing helper'

# run <name> [VAR=val...] — full submit+watch; sets rc, $T/out, $T/err, $T/calls
run() {
  local name=$1; shift
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" FAKE_NAME="$name" \
    env "$@" bash "$here/prompt-worker.sh" "$name" "$brief" >"$T/out" 2>"$T/err" || rc=$?
}
# resume <name> [--extend <sec>] [VAR=val...] — resume wait; same capture
resume() {
  local name=$1; shift
  local vars=() wargs=()
  while (( $# )); do
    if [[ $1 =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then vars+=("$1"); else wargs+=("$1"); fi
    shift
  done
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" FAKE_NAME="$name" \
    env "${vars[@]}" bash "$here/prompt-worker.sh" --resume "$name" "${wargs[@]}" >"$T/out" 2>"$T/err" || rc=$?
}
sentinel() { [[ -f $T/home/quaqua/runs/.$1.nudged ]]; }
nudges()   { grep -c 'agent prompt.*Time check' "$T/calls" || true; }

# ── --resume arg validation ─────────────────────────────────────────────────
rc=0; CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
  bash "$here/prompt-worker.sh" --resume >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 1 ]] || fail "--resume no name: exit $rc, want 1"
grep -q '^# Usage:' "$T/err" || fail "--resume no name: no usage on stderr"
rc=0; CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
  bash "$here/prompt-worker.sh" --resume 'bad_name' >"$T/out" 2>"$T/err" || rc=$?
[[ $rc -eq 1 ]] || fail "--resume bad name: exit $rc, want 1"
for bad in '--extend 0' '--extend abc' '--extend'; do
  rc=0; CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
    bash "$here/prompt-worker.sh" --resume sude-ext $bad >"$T/out" 2>"$T/err" || rc=$?
  [[ $rc -eq 1 ]] || fail "--resume $bad: exit $rc, want 1"
done
echo "ok: --resume arg validation -> exit 1, usage on stderr"

# ── resume happy path: file already there, no prompt ever sent ──────────────
echo done > "$T/home/quaqua/runs/sude-rdone.md"
resume sude-rdone
[[ $rc -eq 0 ]] || fail "sude-rdone: exit $rc: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/sude-rdone.md" ]] \
  || fail "sude-rdone: stdout not the runs path"
grep -q 'agent prompt' "$T/calls" && fail "sude-rdone: --resume still prompted"
echo "ok: --resume -> waits without re-prompting, prints runs path"

# ── resume waits for the file, cap still exit 2 ──────────────────────────────
resume sude-rnever FAKE_DELAY=0 PROMPT_WORKER_MAX_SEC=2 PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_BUDGET_SEC=9999
[[ $rc -eq 2 ]] || fail "sude-rnever: exit $rc, want 2: $(cat "$T/err")"
grep -q 'WATCHDOG exit=2' "$T/err" || fail "sude-rnever: no WATCHDOG line"
echo "ok: --resume no file -> exit 2 at cap with WATCHDOG line"

# ── STALL: static pane, working status -> exit 3 ────────────────────────────
run sude-stall FAKE_DELAY=0 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=2 \
  PROMPT_WORKER_BUDGET_SEC=9999 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 3 ]] || fail "sude-stall: exit $rc, want 3: $(cat "$T/err")"
grep -q 'WATCHDOG exit=3 stall' "$T/err" || fail "sude-stall: no stall line: $(cat "$T/err")"
echo "ok: static pane + working -> exit 3 stall"

# ── progress resets the stall clock: rotating pane outlives STALL_SEC ────────
run sude-busy FAKE_DELAY=0 FAKE_ROTATE=1 PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_STALL_SEC=1 PROMPT_WORKER_BUDGET_SEC=3 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-busy: exit $rc, want 4 (changing pane must beat stall): $(cat "$T/err")"
grep -q 'WATCHDOG exit=4' "$T/err" || fail "sude-busy: no budget line"
echo "ok: changing pane -> no stall, exit 4 at budget"

# ── BUDGET: one nudge past the pct mark, then exit 4; sentinel written ──────
# (NUDGE_PCT=30 puts the nudge inside the first poll window — deterministic
# instead of relying on iteration timing landing in a sub-second slot)
run sude-budget FAKE_DELAY=0 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=3 PROMPT_WORKER_NUDGE_PCT=30 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-budget: exit $rc, want 4: $(cat "$T/err")"
[[ $(nudges) -eq 1 ]] || fail "sude-budget: $(nudges) nudges, want 1: $(cat "$T/calls")"
sentinel sude-budget || fail "sude-budget: no nudge sentinel"
grep -q 'WATCHDOG nudge sent' "$T/err" || fail "sude-budget: no nudge notice"
echo "ok: budget -> one nudge past the pct mark, sentinel, exit 4"

# ── nudge is once per task: resume does not re-nudge ────────────────────────
resume sude-budget PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=2 PROMPT_WORKER_NUDGE_PCT=30 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-budget resume: exit $rc, want 4: $(cat "$T/err")"
[[ $(nudges) -eq 0 ]] || fail "sude-budget resume: re-nudged: $(cat "$T/calls")"
echo "ok: --resume -> no second nudge (sentinel persists)"

# ── D2: pane progressing at the deadline -> one grace, then exit 4 ─────────
run sude-grace FAKE_DELAY=0 FAKE_ROTATE=1 PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_STALL_SEC=1 PROMPT_WORKER_BUDGET_SEC=4 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-grace: exit $rc, want 4: $(cat "$T/err")"
grep -q 'WATCHDOG grace +1s (progressing)' "$T/err" \
  || fail "sude-grace: no grace line: $(cat "$T/err")"
grep -qE 'elapsed=[5-9]s ' "$T/err" \
  || fail "sude-grace: exited before the grace ran: $(cat "$T/err")"
echo "ok: progressing at the deadline -> grace +1s (25% of 4s), exit 4 after it"

# ── D2: no progress at the deadline -> exit 4 immediately, no grace ────────
run sude-quiet FAKE_DELAY=0 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=3 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-quiet: exit $rc, want 4: $(cat "$T/err")"
! grep -q 'WATCHDOG grace' "$T/err" \
  || fail "sude-quiet: grace granted on a static pane: $(cat "$T/err")"
grep -qE 'elapsed=[0-3]s ' "$T/err" \
  || fail "sude-quiet: did not exit at the deadline: $(cat "$T/err")"
echo "ok: static pane at the deadline -> exit 4 at once, no grace"

# ── D2: grace is once per task: the resume does not grant a second one ─────
resume sude-grace PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-grace resume: exit $rc, want 4: $(cat "$T/err")"
! grep -q 'WATCHDOG grace' "$T/err" \
  || fail "sude-grace resume: grace granted twice: $(cat "$T/err")"
echo "ok: grace granted once per task (resume does not re-grant)"

# ── D2: grace never pushes past the cap -> exit 2 ──────────────────────────
run sude-gracecap FAKE_DELAY=0 FAKE_ROTATE=1 PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_STALL_SEC=1 PROMPT_WORKER_BUDGET_SEC=4 PROMPT_WORKER_MAX_SEC=5
[[ $rc -eq 2 ]] || fail "sude-gracecap: exit $rc, want 2 (cap): $(cat "$T/err")"
grep -q 'WATCHDOG grace' "$T/err" \
  || fail "sude-gracecap: grace never granted before the cap: $(cat "$T/err")"
echo "ok: grace clamped by the cap -> exit 2 at the cap"

# ── D1: --resume without --extend keeps the old fast exit 4 ────────────────
run sude-noext FAKE_DELAY=0 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=2 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-noext run1: exit $rc, want 4: $(cat "$T/err")"
resume sude-noext PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-noext resume: exit $rc, want 4: $(cat "$T/err")"
grep -qE 'elapsed=[0-2]s ' "$T/err" \
  || fail "sude-noext resume: took too long — budget restocked without --extend: $(cat "$T/err")"
echo "ok: --resume without --extend -> exit 4 fast (allowance unchanged)"

# ── D1: --extend adds to the persisted allowance and keeps waiting ─────────
resume sude-noext --extend 4 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-noext extend: exit $rc, want 4: $(cat "$T/err")"
grep -q 'WATCHDOG extend +4s' "$T/err" || fail "sude-noext extend: no extend line"
grep -qE 'elapsed=[3-9]s ' "$T/err" \
  || fail "sude-noext extend: did not wait out the extension: $(cat "$T/err")"
[[ "$(cat "$T/home/quaqua/runs/.sude-noext.budget")" == 6 ]] \
  || fail "sude-noext extend: budget sidecar not updated: $(cat "$T/home/quaqua/runs/.sude-noext.budget")"
echo "ok: --extend 4 -> waits 4s more, then exit 4 (allowance persisted in .budget)"

# ── D1: --extend cannot beat the hard cap -> exit 2 ────────────────────────
run sude-extcap FAKE_DELAY=0 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=2 PROMPT_WORKER_MAX_SEC=4
[[ $rc -eq 4 ]] || fail "sude-extcap run1: exit $rc, want 4: $(cat "$T/err")"
resume sude-extcap --extend 300 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_MAX_SEC=4
[[ $rc -eq 2 ]] || fail "sude-extcap: exit $rc, want 2 (cap is absolute): $(cat "$T/err")"
grep -q 'WATCHDOG extend +300s' "$T/err" || fail "sude-extcap: no extend line"
echo "ok: --extend past the cap -> still exit 2 at PROMPT_WORKER_MAX_SEC"

# ── QUESTION: agent_status=blocked -> exit 5 ────────────────────────────────
run sude-blocked FAKE_DELAY=0 FAKE_STATUS=blocked PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_BUDGET_SEC=9999
[[ $rc -eq 5 ]] || fail "sude-blocked: exit $rc, want 5: $(cat "$T/err")"
grep -q 'WATCHDOG exit=5' "$T/err" || fail "sude-blocked: no blocked line"
[[ $(nudges) -eq 0 ]] || fail "sude-blocked: nudged a blocked agent"
echo "ok: agent_status=blocked -> exit 5, no nudge"

# ── QUESTION fallback: working status but pane shows a question ──────────────
run sude-ask FAKE_DELAY=0 FAKE_STATUS=working PROMPT_WORKER_POLL_SEC=1 \
  FAKE_PANE='Choose an option:
> A
> B
Type something.' PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_BUDGET_SEC=9999
[[ $rc -eq 5 ]] || fail "sude-ask: exit $rc, want 5: $(cat "$T/err")"
echo "ok: pane question fallback -> exit 5"

# ── IDLE: agent done/idle without run file -> fast exit 3 ───────────────────
run sude-idle FAKE_DELAY=0 FAKE_STATUS=idle PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_IDLE_SEC=1 PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_BUDGET_SEC=9999
[[ $rc -eq 3 ]] || fail "sude-idle: exit $rc, want 3: $(cat "$T/err")"
grep -q 'agent idle' "$T/err" || fail "sude-idle: no idle reason: $(cat "$T/err")"
echo "ok: agent idle without run file -> exit 3"

# ── read fallback: recent-unwrapped errors -> visible still observed ────────
run sude-vis FAKE_DELAY=0 FAKE_READ_ERR=1 PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_STALL_SEC=2 PROMPT_WORKER_BUDGET_SEC=9999 FAKE_PANE='static screen'
[[ $rc -eq 3 ]] || fail "sude-vis: exit $rc, want 3 via visible fallback: $(cat "$T/err")"
echo "ok: recent-unwrapped error -> falls back to visible, stall still fires"

# ── agent gone: cleaned mid-wait -> fast exit 3, no blind stall ─────────────
run sude-gone FAKE_DELAY=0 FAKE_GONE=1 PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_AGENT_GONE_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=9999
[[ $rc -eq 3 ]] || fail "sude-gone: exit $rc, want 3: $(cat "$T/err")"
grep -q 'agent gone' "$T/err" || fail "sude-gone: no gone reason: $(cat "$T/err")"
echo "ok: agent_not_found (stderr) -> exit 3 agent gone"

# ── agent vanishes mid-wait: healthy polls, then not_found -> exit 3 within ──
#    AGENT_GONE_SEC instead of the budget (truonglao-pr1 regression)
run sude-vanish FAKE_DELAY=0 FAKE_GONE_AFTER=2 PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_AGENT_GONE_SEC=2 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=9999 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 3 ]] || fail "sude-vanish: exit $rc, want 3: $(cat "$T/err")"
grep -q 'agent gone' "$T/err" || fail "sude-vanish: no gone reason: $(cat "$T/err")"
grep -qE 'elapsed=[0-9]s ' "$T/err" \
  || fail "sude-vanish: took too long — gone clock did not fire fast: $(cat "$T/err")"
echo "ok: agent vanishes mid-wait -> exit 3 within AGENT_GONE_SEC"

# ── transient agent_not_found once, then recovers -> no exit 3 ──────────────
run sude-blink FAKE_DELAY=5 FAKE_GONE_AT=2 PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_AGENT_GONE_SEC=3 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=30 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 0 ]] || fail "sude-blink: exit $rc, want 0 (transient blip must not trip agent-gone): $(cat "$T/err")"
! grep -q 'exit=3' "$T/err" || fail "sude-blink: exit 3 on a transient blip: $(cat "$T/err")"
echo "ok: single agent_not_found then recovery -> no exit 3"

# ── transient silent get failure once (status=unknown) then recovers -> ok ──
run sude-unk FAKE_DELAY=5 FAKE_BLANK_AT=2 PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_AGENT_GONE_SEC=3 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=30 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 0 ]] || fail "sude-unk: exit $rc, want 0 (one unknown poll must not trip agent-gone): $(cat "$T/err")"
! grep -q 'exit=3' "$T/err" || fail "sude-unk: exit 3 on a transient unknown: $(cat "$T/err")"
echo "ok: single unknown-status poll then recovery -> no exit 3"

# ── status line carries elapsed/status/context/last/diff ─────────────────────
run sude-stat FAKE_DELAY=0 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=1 \
  FAKE_PANE='Thinking...
▸ bash cargo test
── ⠧ Working ──
👾 SWE-2 · max · ━── 37% ·' PROMPT_WORKER_BUDGET_SEC=9999
[[ $rc -eq 3 ]] || fail "sude-stat: exit $rc, want 3"
grep -q 'elapsed=[0-9]' "$T/err" || fail "sude-stat: no elapsed"
grep -q 'status=working' "$T/err" || fail "sude-stat: no status"
grep -q 'ctx=37%' "$T/err" || fail "sude-stat: ctx not parsed: $(cat "$T/err")"
grep -q 'bash cargo test' "$T/err" || fail "sude-stat: no last tool: $(cat "$T/err")"
echo "ok: WATCHDOG line has elapsed/status/ctx/last-tool"

# ── heartbeat line on stderr while waiting ──────────────────────────────────
run sude-hb FAKE_DELAY=0 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_HEARTBEAT_SEC=1 \
  PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_BUDGET_SEC=3
[[ $rc -eq 4 ]] || fail "sude-hb: exit $rc, want 4"
grep -q 'WATCHDOG heartbeat' "$T/err" || fail "sude-hb: no heartbeat line: $(cat "$T/err")"
echo "ok: heartbeat prints elapsed/status while waiting"

# ── Lead exemption: <slug>-lead idle on a live pipeline is waiting, not dead ──
# pipeline.sh writes runs/pipeline-<task>.state with finished=0 + pipeline_pid
# while running; a <name>-lead agent idle+static beside it is suppressing both
# exit-3 paths (idle and stall) — budget/cap still bound the wait.
sleep 300 & live_pid=$!
true & dead_pid=$!; wait $dead_pid 2>/dev/null || true
pipe_state() { printf 'finished=%s\npipeline_pid=%s\nlead=%s\nlead_pane=%s\n' "$1" "$2" "$3" "$4" \
  > "$T/home/quaqua/runs/pipeline-isl.state"; }

pipe_state 0 "$live_pid" isl-lead '%1'
run isl-lead FAKE_DELAY=0 FAKE_STATUS=idle PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_IDLE_SEC=1 PROMPT_WORKER_STALL_SEC=1 \
  PROMPT_WORKER_BUDGET_SEC=4 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "isl-lead: exit $rc, want 4 (live pipeline = waiting): $(cat "$T/err")"
! grep -q 'exit=3' "$T/err" || fail "isl-lead: exit 3 fired on a waiting Lead: $(cat "$T/err")"
echo "ok: -lead + own live pipeline -> idle+stall exit 3 suppressed, exit 4 at budget"

# a finished pipeline state does not protect the Lead
pipe_state 1 "$live_pid" isl-lead '%1'
run isl-lead FAKE_DELAY=0 FAKE_STATUS=idle PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_IDLE_SEC=1 PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_BUDGET_SEC=9999
[[ $rc -eq 3 ]] || fail "isl-lead finished: exit $rc, want 3: $(cat "$T/err")"
grep -q 'agent idle' "$T/err" || fail "isl-lead finished: no idle reason: $(cat "$T/err")"
echo "ok: -lead + finished=1 pipeline -> idle exit 3 as usual"

# a paused pipeline (dead pid, e.g. plan-ready stop) is the Lead's turn: normal watchdog
pipe_state 0 "$dead_pid" isl-lead '%1'
run isl-lead FAKE_DELAY=0 FAKE_STATUS=idle PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_IDLE_SEC=1 PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_BUDGET_SEC=9999
[[ $rc -eq 3 ]] || fail "isl-lead paused: exit $rc, want 3: $(cat "$T/err")"
grep -q 'agent idle' "$T/err" || fail "isl-lead paused: no idle reason: $(cat "$T/err")"
echo "ok: -lead + dead pipeline pid -> idle exit 3 as usual"

# F6: an unrelated project's live pipeline must not protect this Lead
pipe_state 0 "$live_pid" beta-lead '%9'
run isl-lead FAKE_DELAY=0 FAKE_STATUS=idle PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_IDLE_SEC=1 PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_BUDGET_SEC=9999
[[ $rc -eq 3 ]] || fail "isl-lead foreign: exit $rc, want 3 (foreign pipeline no exemption): $(cat "$T/err")"
grep -q 'agent idle' "$T/err" || fail "isl-lead foreign: no idle reason: $(cat "$T/err")"
echo "ok: foreign lead's live pipeline -> idle exit 3 as usual"

# the exemption is only for *-lead agents: a plain worker beside the same live
# pipeline still gets the fast idle exit
pipe_state 0 "$live_pid" isl-lead '%1'
run sude-isl FAKE_DELAY=0 FAKE_STATUS=idle PROMPT_WORKER_POLL_SEC=1 \
  PROMPT_WORKER_IDLE_SEC=1 PROMPT_WORKER_STALL_SEC=9999 PROMPT_WORKER_BUDGET_SEC=9999
[[ $rc -eq 3 ]] || fail "sude-isl: exit $rc, want 3 (exemption is *-lead only): $(cat "$T/err")"
echo "ok: non-lead worker with live pipeline -> idle exit 3 as usual"
kill "$live_pid" 2>/dev/null || true; rm -f "$T/home/quaqua/runs/pipeline-isl.state"

# ── F5: consumed budget persists across --resume — no fresh clock ───────────
run sude-persist FAKE_DELAY=0 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=3 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-persist run1: exit $rc, want 4: $(cat "$T/err")"
[[ -s $T/home/quaqua/runs/.sude-persist.elapsed ]] || fail "sude-persist: no elapsed sidecar"
el=$(cat "$T/home/quaqua/runs/.sude-persist.elapsed")
(( el >= 3 )) || fail "sude-persist: elapsed $el < 3"
# resume with a big budget: only ~1s of the 4s total remains -> fast exit 4,
# never a fresh 9999s clock
resume sude-persist PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=4 PROMPT_WORKER_MAX_SEC=60
[[ $rc -eq 4 ]] || fail "sude-persist resume: exit $rc, want 4 fast: $(cat "$T/err")"
grep -q 'elapsed=[0-2]s' "$T/err" \
  || fail "sude-persist resume: took too long — resume restocked the budget: $(cat "$T/err")"
echo "ok: --resume continues the same allowance (elapsed $el carried over), no fresh clock"

# a fresh task prompt resets the clock (new invocation, new allowance)
rm -f "$T/home/quaqua/runs/.sude-persist.elapsed"
run sude-persist FAKE_WRITE=0 FAKE_DELAY=0 PROMPT_WORKER_POLL_SEC=1 PROMPT_WORKER_STALL_SEC=9999 \
  PROMPT_WORKER_BUDGET_SEC=2 PROMPT_WORKER_MAX_SEC=5
[[ $rc -eq 4 ]] || fail "sude-persist fresh: exit $rc, want 4"
echo "ok: fresh prompt -> clock reset to the new task's allowance"

echo "PASS"
