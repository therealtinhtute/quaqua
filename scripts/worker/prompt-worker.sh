#!/usr/bin/env bash
# Usage: prompt-worker.sh <name> <task>        submit task brief, wait for runs file
#        prompt-worker.sh --resume <name> [--extend <sec>]
#                                             keep waiting; --extend adds <sec> to
#                                             this name's allowance (persisted in
#                                             runs/.<name>.budget; the cap still wins)
#        prompt-worker.sh --check-brief "<task>"   validate a brief + print its budget
#   Runs `herdr agent prompt <name> --wait --timeout 1800000` with the standard
#   "write summary to ~/quaqua/runs/<name>.md" instruction, then watches the
#   worker until that file exists (poll PROMPT_WORKER_POLL_SEC, default 30).
#   Exit: 0 runs file (path on stdout) · 1 usage · 2 cap PROMPT_WORKER_MAX_SEC
#   (7200) · 3 stall (no pane/worktree change for PROMPT_WORKER_STALL_SEC, 600;
#   or agent idle/done for PROMPT_WORKER_IDLE_SEC, 120; or agent not found /
#   status unknown / unobservable for PROMPT_WORKER_AGENT_GONE_SEC, 60) ·
#   4 time budget · 5 agent blocked on a question/permission.
#   Budget (D3): the brief's `Time budget: <N> min` line sets it,
#   PROMPT_WORKER_BUDGET_SEC overrides when set, neither -> 2700. One soft nudge
#   at PROMPT_WORKER_NUDGE_PCT (80%). A resumed wait continues the allowance
#   persisted in runs/.<name>.budget; --extend <sec> adds to it — never a fresh
#   clock. At the deadline one grace of 25% of the budget (never past the cap)
#   is granted when the last observed pane/worktree change is within 2 polls,
#   then exit 4; granted once per task (runs/.<name>.grace).
#   Lead exemption: a <slug>-lead waits on a backgrounded pipeline.sh whose
#   pane/cwd stay static for a whole build phase, so while an unfinished
#   pipeline state (runs/pipeline-*.state, finished=0) names a live
#   pipeline_pid AND is bound to this watcher (lead=<name> or
#   lead_pane=<its pane>), idle/quiet is treated as waiting — no exit 3.
#   An unrelated project's pipeline never exempts a Lead. Budget/cap and
#   blocked/gone still apply. Consumed budget/cap persists across --resume
#   in runs/.<name>.elapsed — a resumed wait continues the same allowance,
#   never a fresh clock; a new task prompt resets it.
#   Reports publish atomically: a pre-existing runs file is archived on a
#   new task, and the file must be byte-stable across a short window before
#   it counts as complete. stdout carries only the final report path; the
#   herdr submission response goes to runs/.<name>.submit.log.
#   Exits 2-5 print a WATCHDOG status line on stderr; fix the cause, then
#   `--resume` keeps waiting. The script never kills or re-prompts the worker.
set -euo pipefail

# usage prints the leading comment block (lines 2+) — no line numbers to drift
usage() { awk 'NR>1 && /^#/ {print; next} NR>1 {exit}' "$0" >&2; exit 1; }

# Worker briefs must carry the 3 headers from rules/_all/brief.md.
brief_headers_ok() {
  printf '%s\n' "$1" | grep -qF 'Goal:' \
    && printf '%s\n' "$1" | grep -qF 'Hard constraints:' \
    && printf '%s\n' "$1" | grep -qE 'Current design(\s*\(open to challenge\))?:'
}
brief_error() {
  echo "error: task must be a brief with 'Goal:', 'Hard constraints:', 'Current design (open to challenge):' (rules/_all/brief.md)" >&2
  exit 1
}

# D3: seconds from a `Time budget: <N> min` line (empty when absent)
brief_budget_sec() {
  local n
  n=$(printf '%s\n' "$1" \
    | grep -oiE 'time budget:[^0-9]{0,8}[0-9]+[[:space:]]*(min|mins|minutes|m)([^[:alnum:]]|$)' \
    | head -1 | grep -oE '[0-9]+' | head -1) || true
  [[ -n $n ]] && printf '%s\n' $(( n * 60 ))
}

# D3: env PROMPT_WORKER_BUDGET_SEC > the brief's Time budget: > 2700
budget_sec() {
  if [[ -n ${PROMPT_WORKER_BUDGET_SEC:-} ]]; then
    printf '%s\n' "$PROMPT_WORKER_BUDGET_SEC"; return 0
  fi
  local b
  b=$(brief_budget_sec "${1:-}" || true)
  printf '%s\n' "${b:-2700}"
}

if [[ -n ${PROMPT_WORKER_BUDGET_SEC:-} && ! ${PROMPT_WORKER_BUDGET_SEC} =~ ^[0-9]+$ ]]; then
  echo "error: PROMPT_WORKER_BUDGET_SEC must be whole seconds (got '$PROMPT_WORKER_BUDGET_SEC')" >&2
  exit 1
fi

resume=0 extend=''
# D4: validate a brief and report the budget it resolves to — no herdr call
if [[ ${1:-} == --check-brief ]]; then
  (( $# == 2 )) && [[ -n $2 ]] || usage
  brief_headers_ok "$2" || brief_error
  if [[ -n ${PROMPT_WORKER_BUDGET_SEC:-} ]]; then src=env
  elif [[ -n $(brief_budget_sec "$2" || true) ]]; then src=brief
  else src=default; fi
  printf 'budget=%ss source=%s\n' "$(budget_sec "$2")" "$src"
  exit 0
fi
if [[ ${1:-} == --resume ]]; then resume=1; shift; fi
if (( resume )); then
  name=''
  while (( $# )); do
    case $1 in
      --extend)
        [[ ${2:-} =~ ^[1-9][0-9]*$ ]] || usage
        extend=$2; shift 2 ;;
      -*) usage ;;
      *)
        [[ -z $name ]] || usage
        name=$1; shift ;;
    esac
  done
  [[ $name =~ ^[a-z0-9-]+$ ]] || usage
else
  (( $# == 2 )) && [[ $1 =~ ^[a-z0-9-]+$ ]] && [[ -n $2 ]] || usage
  name=$1 task=$2
  brief_headers_ok "$task" || brief_error
fi

out="$HOME/quaqua/runs/$name.md"
submit_log="$HOME/quaqua/runs/.$name.submit.log"
elapsed_file="$HOME/quaqua/runs/.$name.elapsed"
sentinel="$HOME/quaqua/runs/.$name.nudged"
budget_file="$HOME/quaqua/runs/.$name.budget"
grace_file="$HOME/quaqua/runs/.$name.grace"
progress_file="$HOME/quaqua/runs/.$name.progress"
max=${PROMPT_WORKER_MAX_SEC:-7200}     poll=${PROMPT_WORKER_POLL_SEC:-30}
stall=${PROMPT_WORKER_STALL_SEC:-600}  idle_sec=${PROMPT_WORKER_IDLE_SEC:-120}
gone_sec=${PROMPT_WORKER_AGENT_GONE_SEC:-60}
nudge_pct=${PROMPT_WORKER_NUDGE_PCT:-80}
heartbeat=${PROMPT_WORKER_HEARTBEAT_SEC:-300}
(( poll < 1 )) && poll=1
mkdir -p "$(dirname "$out")"

if (( resume )); then
  prev_elapsed=$(cat "$elapsed_file" 2>/dev/null || echo 0)
  [[ $prev_elapsed =~ ^[0-9]+$ ]] || prev_elapsed=0
  # D1/D3: a resumed wait continues the persisted allowance (not a fresh env
  # value); --extend <sec> adds to it for this name
  budget=$(cat "$budget_file" 2>/dev/null || true)
  [[ $budget =~ ^[0-9]+$ ]] || budget=$(budget_sec '')
  if [[ -n $extend ]]; then
    budget=$(( budget + extend ))
    printf '%s\n' "$budget" > "$budget_file"
    printf 'WATCHDOG extend +%ss (allowance %ss, cap %ss)\n' "$extend" "$budget" "$max" >&2
  fi
else
  # a new task gets a fresh report, a fresh clock and a fresh budget: anything
  # already at $out predates this invocation (previous task, crashed run)
  prev_elapsed=0
  budget=$(budget_sec "$task")
  printf '%s\n' "$budget" > "$budget_file"
  rm -f "$elapsed_file" "$sentinel" "$grace_file" "$progress_file"
  if [[ -e $out ]]; then
    mv "$out" "$out.stale.$(date +%s)"
    echo "archived pre-existing $out — a new task needs a fresh report" >&2
  fi
fi
[[ $budget =~ ^[0-9]+$ ]] || { echo "error: budget must be whole seconds (got '$budget')" >&2; exit 1; }

start=$(date +%s)
rem_max=$(( max - prev_elapsed ));       (( rem_max < 1 )) && rem_max=1
rem_budget=$(( budget - prev_elapsed )); (( rem_budget < 1 )) && rem_budget=1
deadline=$(( start + rem_max )) budget_deadline=$(( start + rem_budget ))
nudge_at=$(( start + budget * nudge_pct / 100 - prev_elapsed ))

save_elapsed() { printf '%s\n' $(( prev_elapsed + $(date +%s) - start )) > "$elapsed_file"; }

prompt_pid=''
if (( ! resume )); then
  # Backgrounded so the watchdog below polls from second 0; the wait inside
  # herdr returning early is fine, the loop keeps waiting for the file. A
  # still-running herdr is left behind on early exits: it is a bounded local
  # wait client, never the worker itself, and exits on its own. Its CLI
  # response is kept off stdout (path-only contract) in the submit log.
  herdr agent prompt "$name" "$task. When done, write summary (changes, test cmd+result, risks) to a temp file and rename it to ~/quaqua/runs/$name.md (publish it atomically, never a partial file); reply only the path." \
    --wait --timeout 1800000 > "$submit_log" 2>&1 &
  prompt_pid=$!
fi

# Observe the worker. Sets: status ctx last_tool diffstat pane_text fp watcher_pane.
# Returns 1 when the agent cannot be observed at all (stall checks skipped;
# cap/budget still apply).
status=unknown ctx='?' last_tool='' diffstat='' pane_text='' fp='' watcher_pane=''
hash_text() {  # GNU first, then BSD/macOS; stdin is read only once
  if command -v md5sum >/dev/null 2>&1; then
    md5sum | cut -d' ' -f1
  else
    md5 -q
  fi
}

observe() {
  status=unknown ctx='?' last_tool='' diffstat='' pane_text='' fp='' watcher_pane=''
  local get='' cwd='' body dh='' short unt
  get=$(herdr agent get "$name" 2>/dev/null) || true
  # herdr reports errors (agent_not_found included) on stderr, not stdout —
  # without this a dead agent stayed 'unknown' and unobservable until the
  # budget fired (truonglao-pr1: quiet=1441s, exit 4 at the 1800s budget)
  [[ -z $get ]] && get=$(herdr agent get "$name" 2>&1 >/dev/null) || true
  if [[ -n $get ]]; then
    if jq -e '.result.agent' <<<"$get" >/dev/null 2>&1; then
      status=$(jq -r '.result.agent.agent_status // "unknown"' <<<"$get" 2>/dev/null || echo unknown)
      cwd=$(jq -r '.result.agent.cwd // empty' <<<"$get" 2>/dev/null || true)
      watcher_pane=$(jq -r '.result.agent.pane_id // empty' <<<"$get" 2>/dev/null || true)
    elif jq -e '.error.code == "agent_not_found"' <<<"$get" >/dev/null 2>&1 \
      || [[ $get == *agent_not_found* ]]; then
      status=gone
    fi
  fi
  # recent-* sources fail with agent_not_idle while the agent is working
  # (alt-screen history); fall back to the always-available visible screen.
  # herdr errors print JSON on stdout - treat those as no output.
  pane_text=$(herdr agent read "$name" --source recent-unwrapped --lines 80 2>/dev/null) || true
  if [[ -z $pane_text || $pane_text == '{"error"'* ]]; then
    pane_text=$(herdr agent read "$name" --source visible --lines 80 2>/dev/null) || true
  fi
  [[ $pane_text == '{"error"'* ]] && pane_text=''
  [[ -z $get && -z $pane_text ]] && return 1
  # Fingerprint: drop status-bar rows (spinner/model/quota), collapse digits so
  # timers, % and token counters don't fake progress.
  body=$(printf '%s\n' "$pane_text" \
    | grep -vE '^[─[:space:]│|]*$|👾|[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]' \
    | sed 's/[0-9][0-9]*/N/g')
  ctx=$(grep -E '^[[:space:]]*👾.*·.*[0-9]+%' <<<"$pane_text" | grep -oE '[0-9]+%' | head -1 || true)
  [[ -z $ctx ]] && ctx=$(grep -oE '[0-9]+%' <<<"$pane_text" | head -1 || true); ctx=${ctx:-'?'}
  last_tool=$(grep -E '▸|⏺' <<<"$pane_text" | tail -1 | sed 's/^[[:space:]]*//' | cut -c1-90 || true)
  if [[ -n $cwd ]] && git -C "$cwd" rev-parse --git-dir >/dev/null 2>&1; then
    dh=$( { git -C "$cwd" status --porcelain=v1 2>/dev/null
           git -C "$cwd" diff HEAD --stat 2>/dev/null; } | hash_text)
    short=$(git -C "$cwd" diff HEAD --shortstat 2>/dev/null || true)
    unt=$(git -C "$cwd" status --porcelain=v1 2>/dev/null | grep -c '^??' || true)
    diffstat=${short:-clean}; [[ $unt -gt 0 ]] && diffstat+=", $unt untracked" || true
  fi
  fp="$(hash_text <<<"$body")|$dh"
  return 0
}

question_seen() {  # fallback for agents whose question UI isn't 'blocked'
  tail -15 <<<"$pane_text" | grep -qiE 'type something|esc to abandon|(^|[^[:alpha:]])[yYnN]/[yYnN]([^[:alpha:]]|$)|do you want to|(needs|requires?) (approval|permission)'
}

lead_waiting() {  # a <slug>-lead idle while ITS live pipeline runs is waiting
  [[ $name == *-lead ]] || return 1
  local f pid owner opane
  for f in "$(dirname "$out")"/pipeline-*.state; do
    [[ -f $f ]] || continue
    grep -q '^finished=0$' "$f" || continue
    pid=$(sed -n 's/^pipeline_pid=\([0-9][0-9]*\).*/\1/p' "$f" | head -1)
    [[ -n $pid ]] && kill -0 "$pid" 2>/dev/null || continue
    # the exemption is scoped to the orchestrating Lead: pipeline.sh records
    # lead= (PIPELINE_LEAD) and lead_pane= (the caller's HERDR_PANE_ID). An
    # unrelated project's live pipeline must not suppress this watcher's
    # stall/idle detection.
    owner=$(sed -n 's/^lead=\(.*\)/\1/p' "$f" | head -1)
    opane=$(sed -n 's/^lead_pane=\(.*\)/\1/p' "$f" | head -1)
    [[ -n $owner && $owner == "$name" ]] && return 0
    [[ -n $opane && -n $watcher_pane && $opane == "$watcher_pane" ]] && return 0
  done
  return 1
}

watchdog_exit() {  # <code> <reason> — status line on stderr, stdout stays clean
  local code=$1 reason=$2 now
  now=$(date +%s)
  save_elapsed   # consumed budget/cap carries over to --resume (F5)
  printf 'WATCHDOG exit=%d %s | name=%s elapsed=%ds status=%s ctx=%s quiet=%ds last=%s | diff=%s\n' \
    "$code" "$reason" "$name" "$(( now - start ))" "$status" "$ctx" \
    "$(( now - change_ts ))" "${last_tool:-none}" "${diffstat:-n/a}" >&2
  exit "$code"
}

report_stat() {  # GNU first, then BSD/macOS: size + mtime
  stat -c '%s:%Y' "$1" 2>/dev/null || stat -f '%z:%m' "$1" 2>/dev/null
}

report_ready() {  # -s AND byte-stable across a short window — a partial
  # publish (writer mid-write) is not a complete phase acknowledgement
  [[ -s $out ]] || return 1
  local a b
  a=$(report_stat "$out") || return 1
  sleep 0.4
  b=$(report_stat "$out") || return 1
  [[ $a == "$b" ]]
}

prev_fp='' fp_seen=0 change_ts=$start idle_since=0 last_hb=$start nudged=0 blind_warned=0 gone_since=0
grace_used=0 last_progress=0
[[ -f $sentinel ]] && nudged=1
[[ -f $grace_file ]] && grace_used=1
# D2: a change observed just before the previous invocation gave up still
# counts as progressing when the CoS resumes promptly
[[ -f $progress_file ]] && last_progress=$(cat "$progress_file" 2>/dev/null || echo 0)
[[ $last_progress =~ ^[0-9]+$ ]] || last_progress=0
echo "waiting for $out (poll ${poll}s, stall ${stall}s, idle ${idle_sec}s, budget ${budget}s, cap ${max}s)" >&2
while ! report_ready; do
  now=$(date +%s)
  (( now >= deadline )) && watchdog_exit 2 "error: no $out after ${max}s"
  if observe; then
    # gone clock: not_found, or status unknown with nothing on the pane —
    # must hold for AGENT_GONE_SEC so a transient blip never kills the wait
    if [[ $status == gone || ( $status == unknown && -z $pane_text ) ]]; then
      (( gone_since )) || gone_since=$now
      (( now - gone_since >= gone_sec )) && \
        watchdog_exit 3 "agent gone: status=$status for $(( now - gone_since ))s"
    else
      gone_since=0
    fi
    [[ $status == blocked ]] && watchdog_exit 5 "agent blocked (question/permission pending)"
    question_seen && watchdog_exit 5 "pane shows a pending question/permission prompt"
    # progress is read before the deadline check so a change in this very
    # observation counts; the first snapshot only sets the baseline
    if [[ $fp != "$prev_fp" ]]; then
      prev_fp=$fp
      if (( fp_seen )); then
        change_ts=$now last_progress=$now
        printf '%s\n' "$now" > "$progress_file"
      fi
      fp_seen=1
    fi
    if (( now >= budget_deadline )); then
      # D2: one grace (25% of the allowance, never past the cap) when the last
      # observed change is within 2 polls — a worker mid-step is not a lost
      # cause; granted once per task, nudge is never re-sent
      if (( ! grace_used && last_progress > 0 && now - last_progress <= poll * 2 )); then
        grace=$(( budget / 4 )); (( grace < 1 )) && grace=1
        (( budget_deadline + grace > deadline )) && grace=$(( deadline - budget_deadline ))
        if (( grace > 0 )); then
          budget_deadline=$(( budget_deadline + grace ))
          grace_used=1; : > "$grace_file"
          printf 'WATCHDOG grace +%ss (progressing)\n' "$grace" >&2
        fi
      fi
      (( now >= budget_deadline )) && watchdog_exit 4 "time budget ${budget}s exceeded"
    fi
    if lead_waiting; then
      # a Lead watching a live pipeline: a static pane/cwd is expected for a
      # whole pipeline phase — neither idle nor stall means dead here
      idle_since=0
    elif [[ $status == idle || $status == done ]]; then
      (( idle_since )) || idle_since=$now
      if (( now - idle_since >= idle_sec )); then
        watchdog_exit 3 "agent $status for $(( now - idle_since ))s without run file"
      fi
    else
      idle_since=0
    fi
    if ! lead_waiting && (( now - change_ts >= stall )); then
      watchdog_exit 3 "stall: no pane/worktree change for $(( now - change_ts ))s"
    fi
  else
    # nothing observable at all — same gone clock, or a dead agent waits for
    # the full budget again; cap/budget still apply
    (( gone_since )) || gone_since=$now
    (( now - gone_since >= gone_sec )) && \
      watchdog_exit 3 "agent gone: unobservable (no get/read) for $(( now - gone_since ))s"
    (( blind_warned )) || { printf 'WATCHDOG observe failed (no agent get/read); only cap/budget/agent-gone apply\n' >&2; blind_warned=1; }
    (( now >= budget_deadline )) && watchdog_exit 4 "time budget ${budget}s exceeded (agent unobservable)"
  fi
  if (( ! nudged && now >= nudge_at )); then
    herdr agent prompt "$name" "Time check: you are past ${nudge_pct}% of this task's time budget. Wrap up now: finish the current step, commit your work, and write ~/quaqua/runs/$name.md (via a temp file + rename) with what is done plus a Blockers section for the rest. Reply only the path." \
      >/dev/null 2>&1 || true
    : > "$sentinel"; nudged=1
    printf 'WATCHDOG nudge sent (once, at %s%% of %ss budget)\n' "$nudge_pct" "$budget" >&2
  fi
  if (( now - last_hb >= heartbeat )); then
    printf 'WATCHDOG heartbeat | name=%s elapsed=%ds status=%s ctx=%s quiet=%ds last=%s\n' \
      "$name" "$(( now - start ))" "$status" "$ctx" "$(( now - change_ts ))" "${last_tool:-none}" >&2
    last_hb=$now
  fi
  left=$(( deadline - now )); (( left < 1 )) && left=1
  sleep "$(( left < poll ? left : poll ))"
done
# Bounded grace so a prompt that is still submitting finishes before we report
# success (a slow herdr --wait is left behind, harmless).
if [[ -n $prompt_pid ]]; then
  for ((i = 0; i < 20; i++)); do
    jobs -pr | grep -qx "$prompt_pid" || break
    sleep 0.1
  done
fi
save_elapsed
printf '%s\n' "$out"
