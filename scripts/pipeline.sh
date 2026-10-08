#!/usr/bin/env bash
# Usage: pipeline.sh <task> <repo> --brief <file> [--builder sude] [--reviewer quansu]
#                    [--review auto|always|skip] [--rounds 2] [--base <ref>]
#        pipeline.sh --resume <task> <repo>
#   Build -> review -> fix loop (spec-raw/quaqua-slp-slim-lock.md §1-2) on top of
#   spawn-worker.sh / prompt-worker.sh / cleanup-worker.sh; counts as 2 workers
#   toward the cap. <task> is a slug ([a-z0-9-]): builder <builder>-<task>, a
#   fresh reviewer <reviewer>-<task>r<N> each round (spawn-worker.sh --review).
#   Flow: builder writes a plan -> exit 5 'plan-ready' (the Lead approves by
#   resuming) -> build -> review (auto: diff vs the spawn base is >150 changed
#   lines or touches >1 top-level dir; always/skip override) -> findings go
#   straight back to the builder for a fix, then a fresh reviewer; at most
#   <rounds> review rounds. Ends clean | disputed | blocked. '## Challenge' in
#   any builder runs file stops the pipeline as disputed (Lead decides); a
#   reviewer runs file without a 'Verdict: clean | <N> findings' line stops it
#   as blocked. Reviewers are cleaned right after their runs file is read; the
#   caller cleans the builder after re-running its test.
#   Exit codes reuse prompt-worker.sh's watchdog meanings: 0 finished (status
#   line on stdout + runs/pipeline-<task>.md) · 1 usage/state error · 2 cap ·
#   3 stall/gone · 4 budget · 5 plan-ready pause or a worker blocked on a
#   question. On 2-5 fix the cause (a stuck wait keeps waiting via
#   'prompt-worker.sh --resume <worker>'), then 'pipeline.sh --resume <task>
#   <repo>' continues from runs/pipeline-<task>.state. Every stop appends one
#   line to runs/telemetry.jsonl {ts,task,repo,builder,reviewer,
#   review:run|skip,rounds,review_changed_outcome,status,minutes} — the last
#   line per task is current.
#   Env budgets (sec): PIPELINE_PLAN_BUDGET_SEC 900 · _BUILD_ 2700 · _REVIEW_
#   900 · _FIX_ 1200 · PIPELINE_MAX_SEC 7200 (work time; time between
#   invocations does not count) · PIPELINE_REVIEW_LINES 150.
#   Grid: after each spawn/cleanup its own workers are arranged into tab
#   GRID_TAB (default <repo-basename>-workers); GRID_SKIP=1 disables.
set -euo pipefail

usage() { sed -n '2,29p' "$0" >&2; exit 1; }
die()   { echo "error: $*" >&2; exit 1; }

here="$(cd "$(dirname "$0")" && pwd)"
runs="$HOME/quaqua/runs"
mkdir -p "$runs"
inv_start=$(date +%s)

PLAN_BUDGET=${PIPELINE_PLAN_BUDGET_SEC:-900}
BUILD_BUDGET=${PIPELINE_BUILD_BUDGET_SEC:-2700}
REVIEW_BUDGET=${PIPELINE_REVIEW_BUDGET_SEC:-900}
FIX_BUDGET=${PIPELINE_FIX_BUDGET_SEC:-1200}
MAX_SEC=${PIPELINE_MAX_SEC:-7200}
REVIEW_LINES=${PIPELINE_REVIEW_LINES:-150}

harness_of() {  # worker kind -> harness family (models.md)
  case $1 in
    tieude|tieunhi|sude)  echo pi ;;
    ni|suhuynh|truonglao) echo claude ;;
    bacdau|quansu)        echo codex ;;
    *) return 1 ;;
  esac
}

plog() { printf -- '- %s\n' "$*" >> "$pfile"; }

elapsed() { echo $(( start_elapsed + $(date +%s) - inv_start )); }

save_state() {
  local v
  : > "$state"
  for v in repo brief builder reviewer review rounds base wt base_sha planfile \
           commit phase round prompted rounds_run review_run review_changed \
           last_findings finished start_elapsed; do
    printf '%s=%q\n' "$v" "${!v}" >> "$state"
  done
  # liveness signal for prompt-worker.sh: a Lead watching this pipeline is
  # waiting (not dead) while the pid below is alive (rules/_all/worker-timebox)
  printf 'pipeline_pid=%s\n' "$$" >> "$state"
}

finish() {  # <status> <exit-code> — status line, telemetry, stdout summary
  local st=$1 code=$2 mins
  mins=$(( $(elapsed) / 60 ))
  start_elapsed=$(elapsed)          # persist work time across pauses/resumes
  if (( code == 0 )); then finished=1; fi
  save_state
  plog "status: $st · commit ${commit:-none} · elapsed ${mins}m"
  if [[ -n ${blocker:-} ]]; then printf '\n## Blockers\n- %s\n' "$blocker" >> "$pfile"; fi
  jq -nc --arg ts "$(date -Iseconds)" --arg task "$task" --arg repo "$repo" \
    --arg builder "$builder" --arg reviewer "$reviewer" --arg review "$review_run" \
    --arg status "$st" --argjson rounds "$rounds_run" \
    --argjson changed "$review_changed" --argjson minutes "$mins" \
    '{ts:$ts,task:$task,repo:$repo,builder:$builder,reviewer:$reviewer,
      review:$review,rounds:$rounds,review_changed_outcome:$changed,
      status:$status,minutes:$minutes}' >> "$runs/telemetry.jsonl"
  printf 'pipeline-%s: status=%s rounds=%d commit=%s run=%s\n' \
    "$task" "$st" "$rounds_run" "${commit:-none}" "$pfile"
  exit "$code"
}

check_clock() {  # whole-pipeline budget, evaluated at phase boundaries
  (( $(elapsed) > MAX_SEC )) || return 0
  blocker="total budget ${MAX_SEC}s of work time exceeded — remaining work is in the log above"
  finish blocked 4
}

wait_file() {  # <worker> <prompt> <budget-sec> — prompt once, else resume the wait
  local w=$1 prompt=$2 budget=$3 rc=0
  [[ -s $runs/$w.md ]] && return 0
  if (( prompted )); then
    PROMPT_WORKER_BUDGET_SEC=$budget bash "$here/prompt-worker.sh" --resume "$w" >/dev/null || rc=$?
  else
    prompted=1; save_state   # the prompt is sent by this call; record it first
    PROMPT_WORKER_BUDGET_SEC=$budget bash "$here/prompt-worker.sh" "$w" "$prompt" >/dev/null || rc=$?
  fi
  if (( rc )); then
    blocker="$phase wait for $w: watchdog exit $rc — fix the cause ('prompt-worker.sh --resume $w' keeps a live wait going), then '$0 --resume $task $repo'"
    plog "$blocker"
    finish blocked "$rc"
  fi
}

log_challenge() {  # <file> — copy the worker's Challenge section into the run file
  printf '\n## Challenge (from %s)\n' "$(basename "$1")" >> "$pfile"
  awk '/^## Challenge/{f=1} /^## / && !/^## Challenge/ && f{exit} f' "$1" >> "$pfile"
}

# ── args ────────────────────────────────────────────────────────────────────
if [[ ${1:-} == --resume ]]; then
  (( $# == 3 )) && [[ $2 =~ ^[a-z0-9-]+$ ]] || usage
  task=$2 want_repo=$(realpath "$3")
  state="$runs/pipeline-$task.state" pfile="$runs/pipeline-$task.md"
  [[ -f $state ]] || die "no pipeline state for '$task' ($state) — nothing to resume"
  # shellcheck disable=SC1090
  . "$state"
  if [[ $repo != "$want_repo" ]]; then
    die "state for '$task' is on repo $repo, not $want_repo"
  fi
  if (( finished )); then
    die "pipeline '$task' already finished — see $pfile"
  fi
  plog "resumed: $(date -Iseconds) (phase=$phase)"
else
  builder=sude reviewer=quansu review=auto rounds=2 brief='' base='' pos=()
  while (($#)); do
    case $1 in
      --brief)     (($# >= 2)) || usage; brief=$2; shift 2 ;;
      --brief=*)   brief=${1#--brief=}; shift ;;
      --builder)   (($# >= 2)) || usage; builder=$2; shift 2 ;;
      --builder=*) builder=${1#--builder=}; shift ;;
      --reviewer)   (($# >= 2)) || usage; reviewer=$2; shift 2 ;;
      --reviewer=*) reviewer=${1#--reviewer=}; shift ;;
      --review)    (($# >= 2)) || usage; review=$2; shift 2 ;;
      --review=*)  review=${1#--review=}; shift ;;
      --rounds)    (($# >= 2)) || usage; rounds=$2; shift 2 ;;
      --rounds=*)  rounds=${1#--rounds=}; shift ;;
      --base)      (($# >= 2)) || usage; base=$2; shift 2 ;;
      --base=*)    base=${1#--base=}; shift ;;
      *)           pos+=("$1"); shift ;;
    esac
  done
  (( ${#pos[@]} == 2 )) || usage
  task=${pos[0]} repo=$(realpath "${pos[1]}")
  [[ $task =~ ^[a-z0-9-]+$ ]] || usage
  [[ -n $brief && -f $brief ]] || die "--brief <file> required and must exist (got: '${brief:-none}')"
  brief=$(realpath "$brief")
  grep -qF 'Goal:' "$brief" && grep -qF 'Hard constraints:' "$brief" \
    && grep -qE 'Current design(\s*\(open to challenge\))?:' "$brief" \
    || die "brief must carry the Goal:/Hard constraints:/Current design headers (rules/_all/brief.md)"
  bh=$(harness_of "$builder")  || die "unknown builder kind: $builder"
  rh=$(harness_of "$reviewer") || die "unknown reviewer kind: $reviewer"
  case $reviewer in bacdau|quansu) ;; *)
    die "reviewer must be a codex kind (bacdau|quansu) — spawn-worker.sh --review refuses $reviewer" ;; esac
  [[ $bh != "$rh" ]] \
    || die "builder and reviewer must be different harnesses (models.md): $builder/$reviewer are both $bh"
  case $review in auto|always|skip) ;; *) die "--review must be auto|always|skip" ;; esac
  [[ $rounds =~ ^[0-9]+$ ]] && (( rounds >= 1 )) || die "--rounds must be an integer >= 1"
  (( ${#builder} + 1 + ${#task} <= 20 )) \
    || die "builder name $builder-$task exceeds 20 chars (models.md Naming)"
  (( ${#reviewer} + 1 + ${#task} + 1 + ${#rounds} <= 20 )) \
    || die "reviewer name $reviewer-${task}r$rounds exceeds 20 chars"
  state="$runs/pipeline-$task.state" pfile="$runs/pipeline-$task.md"
  [[ ! -f $state ]] \
    || die "pipeline '$task' already has state — use --resume or a new task slug"
  wt="$(dirname "$repo")/.wt/$builder-$task"
  phase=spawn round=0 prompted=0 rounds_run=0 review_run=skip review_changed=false
  commit='' planfile='' base_sha='' last_findings=0 finished=0 start_elapsed=0
  save_state
  {
    printf '# pipeline %s\n' "$task"
    printf -- '- task: %s · repo: %s\n' "$task" "$repo"
    printf -- '- builder: %s-%s (%s) · reviewer: %s (%s) · review: %s · max rounds: %d\n' \
      "$builder" "$task" "$bh" "$reviewer" "$rh" "$review" "$rounds"
    printf -- '- started: %s\n' "$(date -Iseconds)"
  } > "$pfile"
fi

brief_text=$(cat "$brief" 2>/dev/null) || die "brief file unreadable: $brief"

# Fanout grid (rules/_all/fanout-agent-grid.md): after every spawn/cleanup the
# pipeline's own workers go to the project workers tab — default
# <repo-basename>-workers (e.g. coucou-island-workers), GRID_TAB overrides.
# --only scopes the grid's sweep to this pipeline's workers, so workers of
# another orchestrator sitting in the same (CoS) tab are never touched. Child
# spawn/cleanup scripts get GRID_SKIP=1: their unfiltered hook would sweep the
# anchor tab. GRID_SKIP=1 disables the pipeline's own hook too (tests).
grid_tab=${GRID_TAB:-"$(basename "$repo")-workers"}
grid_hook() {  # grid_hook <name>[,<name>...] — best-effort, never fails a phase
  [[ ${GRID_SKIP:-} == 1 ]] && return 0
  local grid="$here/grid-workers.sh"
  [[ -f $grid ]] || return 0
  bash "$grid" --tab "$grid_tab" --only "$1" >&2 \
    || echo "warning: grid-workers.sh failed (exit $?); arrange panes manually" >&2
}

# ── phase machine ───────────────────────────────────────────────────────────
while :; do
  check_clock
  case $phase in

    spawn)
      if [[ ! -d $wt ]]; then
        sargs=(); [[ -n $base ]] && sargs+=(--base "$base")
        if ! GRID_SKIP=1 bash "$here/spawn-worker.sh" "${sargs[@]}" "$builder-$task" "$builder" "$repo" >&2; then
          blocker="spawn-worker.sh failed for builder $builder-$task — fix the cause, then '$0 --resume $task $repo'"
          plog "$blocker"; finish blocked 3
        fi
      fi
      grid_hook "$builder-$task"
      base_sha=$(git -C "$wt" rev-parse HEAD 2>/dev/null || echo unknown)
      plog "builder $builder-$task spawned (base $base_sha)"
      phase=plan prompted=0; save_state ;;

    plan)
      wait_file "$builder-$task" "$brief_text

Pipeline phase: PLAN — write only the implementation plan (steps, files to touch, risks) as your runs file; do not change code yet. The plan is approved before you build. If the premise is wrong, write '## Challenge' with evidence instead of planning around it." "$PLAN_BUDGET"
      planfile="$runs/$builder-$task.plan.md"
      mv "$runs/$builder-$task.md" "$planfile"
      plog "plan: $(basename "$planfile")"
      if grep -q '^## Challenge' "$planfile"; then
        log_challenge "$planfile"
        blocker="builder challenged the premise at plan stage — Lead resolves the Challenge above"
        finish disputed 0
      fi
      phase=build prompted=0; save_state
      printf 'PIPELINE plan-ready: approve %s then resume: %s --resume %s %s\n' \
        "$planfile" "$0" "$task" "$repo" >&2
      finish plan-ready 5 ;;

    build)
      wait_file "$builder-$task" "$brief_text

Pipeline phase: BUILD — your plan is approved ($planfile). Implement it in your worktree, commit on quaqua/$builder-$task, run the tests. If the premise or the plan proves wrong, stop and write '## Challenge' with evidence instead of forcing it." "$BUILD_BUDGET"
      mv "$runs/$builder-$task.md" "$runs/$builder-$task.build.md"
      commit=$(git -C "$wt" rev-parse --short HEAD 2>/dev/null || echo '?')
      plog "build: commit $commit ($builder-$task.build.md)"
      if grep -q '^## Challenge' "$runs/$builder-$task.build.md"; then
        log_challenge "$runs/$builder-$task.build.md"
        blocker="builder challenged the premise during build — Lead resolves the Challenge above"
        finish disputed 0
      fi
      if [[ $commit == "${base_sha:0:7}" ]]; then
        plog "note: branch tip == base — builder may not have committed"
      fi
      case $review in
        always) review_run=run ;;
        skip)   review_run=skip ;;
        auto)
          lines=$(git -C "$wt" diff --numstat "$base_sha" 2>/dev/null \
                  | awk '{a+=$1+$2} END{print a+0}')
          dirs=$(git -C "$wt" diff --name-only "$base_sha" 2>/dev/null \
                 | awk -F/ 'NF>1{print $1; next}{print "."}' | sort -u | wc -l)
          plog "review gate: $lines changed lines, $dirs top-level dirs (run if >$REVIEW_LINES or >1)"
          if (( lines > REVIEW_LINES || dirs > 1 )); then review_run=run; else review_run=skip; fi ;;
      esac
      if [[ $review_run == skip ]]; then
        plog "review: skipped (gate: $review)"
        finish clean 0
      fi
      phase=review round=1 prompted=0; save_state ;;

    review)
      revname="$reviewer-${task}r${round}"
      if [[ ! -d "$(dirname "$repo")/.wt/$revname" && ! -s $runs/$revname.md ]]; then
        if ! GRID_SKIP=1 bash "$here/spawn-worker.sh" --review --base "quaqua/$builder-$task" \
             "$revname" "$reviewer" "$repo" >&2; then
          blocker="spawn-worker.sh failed for reviewer $revname — fix the cause, then '$0 --resume $task $repo'"
          plog "$blocker"; finish blocked 3
        fi
        plog "review round $round: $revname spawned on quaqua/$builder-$task"
      fi
      grid_hook "$builder-$task,$revname"
      wait_file "$revname" "Goal: review the diff for task '$task' and report findings with file:line evidence
Hard constraints: read-only — do not edit or create files outside ~/quaqua/runs; every finding must cite file:line in the diff.
Current design (open to challenge): builder $builder-$task implemented the task on branch quaqua/$builder-$task — your worktree is on its tip; run 'git diff $base_sha' for the full change. Original brief: $brief (readable).
Unknowns: whether the change meets the brief — verify correctness, missing tests, and scope creep against the diff, not the report.
Evidence: a findings list, then the runs file MUST end with exactly one line: 'Verdict: clean' or 'Verdict: <N> findings'.
Pipeline phase: REVIEW (round $round of $rounds)" "$REVIEW_BUDGET"
      verdict=$(grep -E '^[[:space:]]*Verdict:' "$runs/$revname.md" | tail -1 || true)
      rounds_run=$round
      plog "review round $round: ${verdict:-NO VERDICT LINE} ($revname.md)"
      GRID_SKIP=1 bash "$here/cleanup-worker.sh" "$revname" "$repo" >&2 \
        || echo "warning: cleanup $revname failed; clean it manually" >&2
      grid_hook "$builder-$task"
      if [[ $verdict =~ [Vv]erdict:[[:space:]]*([0-9]+)[[:space:]]*[Ff]indings? ]]; then
        last_findings=${BASH_REMATCH[1]}
      elif [[ $verdict =~ [Vv]erdict:[[:space:]]*[Cc]lean ]]; then
        last_findings=0
      else
        blocker="reviewer $revname runs file has no 'Verdict: clean | <N> findings' line (lock §1) — Lead decides"
        finish blocked 0
      fi
      if (( last_findings == 0 )); then finish clean 0; fi
      review_changed=true
      if (( round >= rounds )); then
        blocker="review rounds exhausted: $last_findings findings still open after $rounds round(s) — Lead decides"
        finish blocked 0
      fi
      phase=fix prompted=0; save_state ;;

    fix)
      revname="$reviewer-${task}r${round}"
      wait_file "$builder-$task" "Goal: resolve the $last_findings review findings for task '$task'
Hard constraints: stay in your worktree on quaqua/$builder-$task; commit each fix; keep tests green. A finding that is wrong is NOT fixed — dispute it under '## Challenge' in your runs file with evidence.
Current design (open to challenge): reviewer $revname reported $last_findings findings — full list in $runs/$revname.md (readable); your worktree and branch are unchanged.
Unknowns: which findings are real — verify each against the code before changing anything.
Evidence: committed fixes + test output, or '## Challenge' per wrong finding.
Pipeline phase: FIX (after review round $round)" "$FIX_BUDGET"
      mv "$runs/$builder-$task.md" "$runs/$builder-$task.fix$round.md"
      commit=$(git -C "$wt" rev-parse --short HEAD 2>/dev/null || echo '?')
      plog "fix $round: commit $commit ($builder-$task.fix$round.md)"
      if grep -q '^## Challenge' "$runs/$builder-$task.fix$round.md"; then
        log_challenge "$runs/$builder-$task.fix$round.md"
        blocker="builder disputed review findings — Lead resolves the Challenge above"
        finish disputed 0
      fi
      phase=review round=$((round + 1)) prompted=0; save_state ;;

    *) die "corrupt state: phase=$phase ($state)" ;;
  esac
done
