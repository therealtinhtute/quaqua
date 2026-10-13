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
#   any worker runs file (builder or reviewer) stops the pipeline as disputed
#   (Lead decides); a reviewer runs file without a 'Verdict: clean | <N>
#   findings [(<B> blocking)]' line stops it as blocked (only blocking findings
#   start a fix round; 'clean' beside a '[blocking]' line is blocked). The brief
#   must be a contract (rules/_all/brief.md): before any 'clean' the contract
#   gate checks changed files against 'Owned scope:' and re-runs every
#   acceptance command on the tip, writing '## Acceptance' (fail -> blocked);
#   the latest builder report must restate every A-id verbatim as pass (C2). A phase report that declares itself
#   unfinished ('Outcome: BLOCKED|PARTIAL|INCOMPLETE|FAILED' or a non-empty
#   '## Blockers' section — e.g. written after the watchdog nudge) never
#   advances: the pipeline stops blocked with the evidence copied in. After
#   BUILD/FIX the branch tip must hold real commits in a clean worktree — the
#   size gate and the reviewer are bound to that exact SHA; a builder that
#   intentionally changed nothing says 'Outcome: no changes' (clean, review
#   skipped). Reviewers are cleaned right after their runs file is read; the
#   caller cleans the builder after re-running its test.
#   Exit codes reuse prompt-worker.sh's watchdog meanings: 0 finished (status
#   line on stdout + runs/pipeline-<task>.md) · 1 usage/state error · 2 cap ·
#   3 stall/gone (also: resumable snapshot/spawn failures) · 4 budget · 5
#   plan-ready pause, a mandatory grid-arrange pause, or a worker blocked on
#   a question. On 2-5 fix the cause (a stuck wait keeps waiting via
#   'prompt-worker.sh --resume <worker>'), then 'pipeline.sh --resume <task>
#   <repo>' continues from runs/pipeline-<task>.state. Every stop appends one
#   line to runs/telemetry.jsonl {ts,task,repo,builder,reviewer,
#   review:run|skip,rounds,review_changed_outcome,status,minutes} — the last
#   line per task is current.
#   Env budgets (sec): PIPELINE_PLAN_BUDGET_SEC 900 · _BUILD_ 2700 · _REVIEW_
#   900 · _FIX_ 1200 · PIPELINE_MAX_SEC 7200 (work time; time between
#   invocations does not count) · PIPELINE_REVIEW_LINES 150.
#   Grid: after each spawn/cleanup its own workers are arranged into tab
#   GRID_TAB (default <repo-basename>-workers); GRID_SKIP=1 with QUAQUA_TEST=1 disables (tests).
set -euo pipefail

USAGE_LAST=29

here="$(cd "$(dirname "$0")" && pwd)"
# sibling entrypoints: pipeline.sh lives in scripts/lead/, the worker scripts it
# drives (spawn/prompt/cleanup/grid) in scripts/worker/ — resolve both from $0.
worker_dir="$(cd "$here/../worker" && pwd)"
w() { printf '%s/%s' "$worker_dir" "$1"; }
runs="$HOME/quaqua/runs"
mkdir -p "$runs"
. "$here/../lib/common.sh"
. "$here/../lib/brief.sh"
inv_start=$(date +%s)

PLAN_BUDGET=${PIPELINE_PLAN_BUDGET_SEC:-900}
BUILD_BUDGET=${PIPELINE_BUILD_BUDGET_SEC:-2700}
REVIEW_BUDGET=${PIPELINE_REVIEW_BUDGET_SEC:-900}
FIX_BUDGET=${PIPELINE_FIX_BUDGET_SEC:-1200}
MAX_SEC=${PIPELINE_MAX_SEC:-7200}
REVIEW_LINES=${PIPELINE_REVIEW_LINES:-150}

file_mtime() {  # GNU first, then BSD/macOS
  stat -c %Y "$1" 2>/dev/null || stat -f %m "$1" 2>/dev/null
}

iso_timestamp() {  # GNU first; BSD versions without -I still emit ISO 8601
  date -Iseconds 2>/dev/null || date '+%Y-%m-%dT%H:%M:%S%z'
}

harness_of() {  # worker kind -> harness family (models.md)
  case $1 in
    tieude|detu|sude)  echo pi ;;
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
  for v in repo brief builder reviewer review rounds base wt base_sha head_sha planfile \
           commit phase round prompted prompt_ts rounds_run review_run review_changed \
           last_findings finished start_elapsed builder_spawned reviewer_spawned \
           lead lead_pane; do
    printf '%s=%q\n' "$v" "${!v}" >> "$state"
  done
  # liveness signal for prompt-worker.sh: a Lead watching this pipeline is
  # waiting (not dead) while the pid below is alive (rules/_all/worker-timebox)
  printf 'pipeline_pid=%s\n' "$$" >> "$state"
}

finish() {  # <status> <exit-code> — status line, telemetry, stdout summary
  local st=$1 code=$2 mins
  start_elapsed=$(elapsed)          # persist work time across pauses/resumes
  # the clock is only re-checked at phase boundaries, so re-check before a
  # success verdict: 'clean' over the whole-pipeline budget is not a success
  # (lock §1: over -> stop + Blockers)
  if [[ $st == clean && $code -eq 0 ]] && (( start_elapsed > MAX_SEC )); then
    st=blocked
    blocker=${blocker:-"total budget ${MAX_SEC}s of work time exceeded — outcome arrived over budget"}
  fi
  mins=$(( start_elapsed / 60 ))
  if (( code == 0 )); then finished=1; fi
  save_state
  plog "status: $st · commit ${commit:-none} · elapsed ${mins}m"
  if [[ -n ${blocker:-} ]]; then printf '\n## Blockers\n- %s\n' "$blocker" >> "$pfile"; fi
  jq -nc --arg ts "$(iso_timestamp)" --arg task "$task" --arg repo "$repo" \
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

contract_gate() {  # scope + acceptance re-run on the frozen tip; appends
  # '## Acceptance' to the run file and stops the pipeline when either fails
  local sha out rc id cmd f bad=0 scope=()
  sha=${head_sha:-$base_sha}   # the tip check_impl froze (base on 'no changes')
  mapfile -t scope < <(brief_scope "$brief_text")
  printf '\n## Acceptance (re-run by pipeline at %s)\n' "${sha:0:7}" >> "$pfile"
  while IFS= read -r f; do
    [[ -n $f ]] || continue
    path_in_scope "$f" "${scope[@]}" || { plog "scope: FAIL $f is outside Owned scope"; bad=1; }
  done < <(git -C "$wt" diff --name-only "$base_sha" "$sha" 2>/dev/null)
  (( bad )) || plog "scope: pass (changed files within Owned scope: ${scope[*]})"
  # C2: the builder's latest report restates every A-id verbatim as passing
  local rep rline
  rep=$(ls -t "$runs/$builder-$task".build.md "$runs/$builder-$task".fix*.md 2>/dev/null | head -1 || true)
  while IFS=$'\t' read -r id cmd; do
    [[ -n $id ]] || continue
    rline=$(grep -E "^[[:space:]]*-?[[:space:]]*$id:" "${rep:-/dev/null}" 2>/dev/null | head -1 || true)
    if [[ $rline == *"\`$cmd\`"* && $rline =~ [Pp][Aa][Ss][Ss] ]]; then
      plog "handback $id: matches the brief and reports pass ($(basename "${rep:-none}"))"
    else
      plog "handback $id: FAIL — $(basename "${rep:-no report}") does not restate \`$cmd\` as pass"; bad=1
    fi
    rc=0; out=$(cd "$wt" && timeout "${PIPELINE_ACCEPT_SEC:-300}" bash -c "$cmd" 2>&1) || rc=$?
    if (( rc == 0 )); then plog "$id: \`$cmd\` → exit 0 — pass"
    else plog "$id: \`$cmd\` → exit $rc — fail"; bad=1; fi
    printf '%s\n' "$out" | tail -3 | sed 's/^/    /' >> "$pfile"
  done < <(brief_acceptance "$brief_text")
  if (( bad )); then
    blocker="contract gate failed at ${sha:0:7} (see '## Acceptance' above) — fix on quaqua/$builder-$task, then '$0 --resume $task $repo'"
    finish blocked 0
  fi
  plog "accepted by contract gate: yes — Lead re-checks, then merges"
}

finish_clean() { contract_gate; finish clean 0; }

check_clock() {  # whole-pipeline budget, evaluated at phase boundaries
  (( $(elapsed) > MAX_SEC )) || return 0
  blocker="total budget ${MAX_SEC}s of work time exceeded — remaining work is in the log above"
  finish blocked 4
}

archive_stale() {  # <file> — a report that predates this phase's prompt is stale
  local f=$1
  mv "$f" "$f.stale.$(date +%s)" \
    && plog "archived stale report $(basename "$f") (predates this phase's prompt)"
}

blocked_outcome() {  # <file> — the report declares itself unfinished (nudge
  # reports carry '## Blockers'; a worker may also write 'Outcome: BLOCKED')
  grep -qiE '^(Outcome|Status|Result):[[:space:]]*(BLOCKED|PARTIAL|INCOMPLETE|FAILED)' "$1" \
    && return 0
  awk '/^## Blockers/{f=1; next} /^## /{f=0} f && NF {x=1} END{exit !x}' "$1"
}

log_blockers() {  # <file> — copy the worker's unfinished-evidence into the run file
  printf '\n## Worker stopped early (from %s)\n' "$(basename "$1")" >> "$pfile"
  { grep -inE '^(Outcome|Status|Result):[[:space:]]*[[:alnum:]]+' "$1" || true
    awk '/^## Blockers/{f=1} /^## / && !/^## Blockers/ && f{exit} f' "$1"; } >> "$pfile"
}

check_impl() {  # <report> [prev-sha] — F2 gate: the delivered implementation is
  # committed on quaqua/$builder-$task and the worktree is clean, so the size
  # gate and the reviewer look at the same immutable snapshot ($head_sha).
  # With [prev-sha] (FIX) the tip must also have advanced past it.
  local report=$1 prev=${2:-} dirty
  head_sha=$(git -C "$wt" rev-parse HEAD 2>/dev/null || echo '?')
  commit=${head_sha:0:7}
  dirty=$(git -C "$wt" status --porcelain 2>/dev/null || echo '?')
  if [[ $head_sha == '?' || $dirty == '?' ]]; then
    blocker="cannot inspect $wt (git rev-parse/status failed) — fix the worktree, then '$0 --resume $task $repo'"
    plog "$blocker"; finish blocked 3
  fi
  if [[ -n $prev && $head_sha == "$prev" ]]; then
    blocker="fix produced no commits past ${prev:0:7} on quaqua/$builder-$task — a no-op answer to findings belongs in '## Challenge'. Commit the fixes, then '$0 --resume $task $repo'"
    plog "$blocker"; finish blocked 3
  fi
  if [[ -z $prev && $head_sha == "$base_sha" ]]; then
    if [[ -z $dirty ]] && grep -qiE '^Outcome:[[:space:]]*no[ -]?(op|changes)' "$report"; then
      plog "build: no commits — builder declared 'Outcome: no changes'; nothing to review"
      finish_clean
    fi
    blocker="builder produced no commits on quaqua/$builder-$task (HEAD == base ${base_sha:0:7}) and did not declare 'Outcome: no changes' — commit the work (or fix the report), then '$0 --resume $task $repo'"
    plog "$blocker"; finish blocked 3
  fi
  if [[ -n $dirty ]]; then
    printf -- '- uncommitted/untracked in %s:\n%s\n' "$wt" "$dirty" >> "$pfile"
    blocker="worktree dirty — the files above are missing from quaqua/$builder-$task, so the gate and the reviewer would see a different snapshot. Commit or clean them, then '$0 --resume $task $repo'"
    finish blocked 3
  fi
}

wait_file() {  # <worker> <prompt> <budget-sec> — prompt once, else resume the wait
  local w=$1 prompt=$2 budget=$3 f rc=0 rem
  f=$runs/$w.md
  rem=$(( MAX_SEC - $(elapsed) ))   # a phase never gets more than is left (F5)
  (( rem < budget )) && budget=$rem
  (( budget < 1 )) && budget=1
  if (( ! prompted )); then
    [[ -e $f ]] && archive_stale "$f"
    prompted=1 prompt_ts=$(date +%s); save_state   # the prompt is sent below
    PROMPT_WORKER_BUDGET_SEC=$budget bash "$(w prompt-worker.sh)" "$w" "$prompt" >/dev/null || rc=$?
  else
    # resume: consume the report only when it postdates the recorded prompt
    if [[ -s $f && $(file_mtime "$f" || echo 0) -lt ${prompt_ts:-0} ]]; then
      archive_stale "$f"
    fi
    PROMPT_WORKER_BUDGET_SEC=$budget bash "$(w prompt-worker.sh)" --resume "$w" >/dev/null || rc=$?
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
  save_state   # refresh pipeline_pid so a watching Lead keeps its exemption
  plog "resumed: $(iso_timestamp) (phase=$phase)"
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
  gaps=$(brief_contract_gaps "$(cat "$brief")")
  [[ -z $gaps ]] || die "brief is not a contract (rules/_all/brief.md) — missing: $(paste -sd';' <<<"$gaps")"
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

  owner=${QUAQUA_OWNER:-${PIPELINE_LEAD:-cos}}
  slots_used=$(count_owner_slots "$runs" "$owner")
  if (( slots_used + 2 > 4 )); then
    echo "error: worker cap exceeded (owner '$owner' has $slots_used slots in use; pipeline reserves 2, cap is 4)" >&2
    exit 2
  fi

  state="$runs/pipeline-$task.state" pfile="$runs/pipeline-$task.md"
  [[ ! -f $state ]] \
    || die "pipeline '$task' already has state — use --resume or a new task slug"
  wt="$(dirname "$repo")/.wt/$builder-$task"
  phase=spawn round=0 prompted=0 prompt_ts=0 rounds_run=0 review_run=skip review_changed=false
  commit='' planfile='' base_sha='' head_sha='' last_findings=0 finished=0 start_elapsed=0
  builder_spawned=0 reviewer_spawned=0
  # shellcheck disable=SC2034  # consumed via ${!v} in save_state
  lead=${PIPELINE_LEAD:-${QUAQUA_OWNER:-}} lead_pane=${HERDR_PANE_ID:-}   # F6: bind the Lead exemption
  save_state
  {
    printf '# pipeline %s\n' "$task"
    printf -- '- task: %s · repo: %s\n' "$task" "$repo"
    printf -- '- builder: %s-%s (%s) · reviewer: %s (%s) · review: %s · max rounds: %d\n' \
      "$builder" "$task" "$bh" "$reviewer" "$rh" "$review" "$rounds"
    printf -- '- started: %s\n' "$(iso_timestamp)"
  } > "$pfile"
fi

brief_text=$(cat "$brief" 2>/dev/null) || die "brief file unreadable: $brief"

# Fanout grid (rules/_all/fanout-agent-grid.md): after every spawn/cleanup the
# pipeline's own workers go to the project workers tab — default
# <repo-basename>-workers (e.g. coucou-island-workers), GRID_TAB overrides.
# --only scopes which panes this call may move to this pipeline's workers, so
# workers of another orchestrator or a sibling project sitting in the same
# (CoS) tab are never touched — while the owner's whole worker population
# still decides the layout, keeping a lone pipeline worker in the project tab.
# Child spawn/cleanup scripts get GRID_SKIP=1: their unfiltered hook would
# sweep the anchor tab. GRID_SKIP=1 + QUAQUA_TEST=1 disables the pipeline's own
# hook (tests only).
grid_tab=${GRID_TAB:-"$(basename "$repo")-workers"}
grid_hook() {  # grid_hook <name>[,<name>...] — returns the grid's rc; the
  # caller decides: pre-prompt failure = pause (MANDATORY rule), post-cleanup
  # failure = warning only
  [[ ${GRID_SKIP:-} == 1 && ${QUAQUA_TEST:-} == 1 ]] && return 0
  local grid; grid="$(w grid-workers.sh)"; local rc=0
  [[ -f $grid ]] || return 0
  bash "$grid" --tab "$grid_tab" --only "$1" >&2 || rc=$?
  if (( rc )); then
    echo "warning: grid-workers.sh failed (exit $rc); arrange panes manually" >&2
  fi
  return "$rc"
}

# ── phase machine ───────────────────────────────────────────────────────────
while :; do
  check_clock
  case $phase in

    spawn)
      # spawn completion is tracked in state — a worktree dir alone is not
      # proof the agent started (spawn-worker.sh reuses a leftover worktree)
      if (( ! builder_spawned )); then
        sargs=(); [[ -n $base ]] && sargs+=(--base "$base")
        if ! PIPELINE_TASK="$task" QUAQUA_OWNER="${lead:-cos}" GRID_SKIP=1 bash "$(w spawn-worker.sh)" "${sargs[@]}" "$builder-$task" "$builder" "$repo" >&2; then
          blocker="spawn-worker.sh failed for builder $builder-$task — fix the cause, then '$0 --resume $task $repo'"
          plog "$blocker"; finish blocked 3
        fi
        builder_spawned=1; save_state
      fi
      if ! grid_hook "$builder-$task"; then
        blocker="grid-workers.sh failed for $builder-$task — arrange the pane per rules/_all/fanout-agent-grid.md, then '$0 --resume $task $repo'"
        plog "$blocker"; save_state; finish blocked 5
      fi
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
      if blocked_outcome "$planfile"; then
        log_blockers "$planfile"
        blocker="builder's plan reports unfinished work — Lead decides how to proceed"
        finish blocked 0
      fi
      phase=build prompted=0; save_state
      printf 'PIPELINE plan-ready: approve %s then resume: %s --resume %s %s\n' \
        "$planfile" "$0" "$task" "$repo" >&2
      finish plan-ready 5 ;;

    build)
      wait_file "$builder-$task" "$brief_text

Pipeline phase: BUILD — your plan is approved ($planfile). Implement it in your worktree, commit on quaqua/$builder-$task, run the tests. If the premise or the plan proves wrong, stop and write '## Challenge' with evidence instead of forcing it. If the work turns out to need no changes at all, say 'Outcome: no changes' in your runs file. Runs file = the PEER.md handback: Outcome, Snapshot, Scope (within Owned scope), Acceptance (each A-id from the brief, verbatim, with its real output and pass|fail), Verification, Unknown / risk. The pipeline re-runs every A-id on your tip." "$BUILD_BUDGET"
      mv "$runs/$builder-$task.md" "$runs/$builder-$task.build.md"
      if grep -q '^## Challenge' "$runs/$builder-$task.build.md"; then
        log_challenge "$runs/$builder-$task.build.md"
        blocker="builder challenged the premise during build — Lead resolves the Challenge above"
        finish disputed 0
      fi
      if blocked_outcome "$runs/$builder-$task.build.md"; then
        log_blockers "$runs/$builder-$task.build.md"
        blocker="builder's build report is unfinished (see above) — Lead decides how to proceed"
        finish blocked 0
      fi
      check_impl "$runs/$builder-$task.build.md"   # sets head_sha/commit; stops when invalid
      plog "build: commit $commit ($builder-$task.build.md)"
      case $review in
        always) review_run=run ;;
        skip)   review_run=skip ;;
        auto)
          lines=$(git -C "$wt" diff --numstat "$base_sha" "$head_sha" 2>/dev/null \
                  | awk '{a+=$1+$2} END{print a+0}')
          dirs=$(git -C "$wt" diff --name-only "$base_sha" "$head_sha" 2>/dev/null \
                 | awk -F/ 'NF>1{print $1; next}{print "."}' | sort -u | wc -l)
          plog "review gate: $lines changed lines, $dirs top-level dirs (run if >$REVIEW_LINES or >1)"
          if (( lines > REVIEW_LINES || dirs > 1 )); then review_run=run; else review_run=skip; fi ;;
      esac
      if [[ $review_run == skip ]]; then
        plog "review: skipped (gate: $review)"
        finish_clean
      fi
      phase=review round=1 prompted=0 reviewer_spawned=0; save_state ;;

    review)
      revname="$reviewer-${task}r${round}"
      if (( ! reviewer_spawned )); then
        # a report that predates this round's reviewer spawn is stale
        [[ -s $runs/$revname.md ]] && archive_stale "$runs/$revname.md"
        if ! PIPELINE_TASK="$task" QUAQUA_OWNER="${lead:-cos}" GRID_SKIP=1 bash "$(w spawn-worker.sh)" --review --base "$head_sha" \
             "$revname" "$reviewer" "$repo" >&2; then
          blocker="spawn-worker.sh failed for reviewer $revname — fix the cause, then '$0 --resume $task $repo'"
          plog "$blocker"; finish blocked 3
        fi
        reviewer_spawned=1; save_state
        plog "review round $round: $revname spawned on $head_sha (quaqua/$builder-$task tip)"
      fi
      if ! grid_hook "$builder-$task,$revname"; then
        blocker="grid-workers.sh failed for $revname — arrange the panes per rules/_all/fanout-agent-grid.md, then '$0 --resume $task $repo'"
        plog "$blocker"; save_state; finish blocked 5
      fi
      wait_file "$revname" "Goal: review the diff for task '$task' and report findings with file:line evidence
Hard constraints: read-only — do not edit or create files outside ~/quaqua/runs; every finding must cite file:line in the diff.
Current design (open to challenge): builder $builder-$task implemented the task on branch quaqua/$builder-$task — your worktree is on its tip; run 'git diff $base_sha' for the full change. Original brief: $brief (readable).
Unknowns: whether the change meets the brief — verify correctness, missing tests, and scope creep against the diff, not the report.
Evidence: findings under '## Spec' (vs Goal, Acceptance, Owned scope) and '## Standards', each '- [blocking|nit] <path:line> — <claim> — <evidence>' (always blocking: a deleted/skipped/weakened test, a write outside Owned scope, an acceptance check that passes without proving the Goal); the runs file MUST end with exactly one line: 'Verdict: clean' (zero blocking) or 'Verdict: <N> findings (<B> blocking)'. Write to a temp file and rename it to ~/quaqua/runs/$revname.md — publish atomically, never a partial file.
Pipeline phase: REVIEW (round $round of $rounds)" "$REVIEW_BUDGET"
      rounds_run=$round
      # a reviewer is a Peer: its Challenge is resolved by the Lead before the
      # verdict is read (a 'Verdict: clean' beside a Challenge is not clean)
      if grep -q '^## Challenge' "$runs/$revname.md"; then
        log_challenge "$runs/$revname.md"
        plog "review round $round: $revname challenged the premise"
        PIPELINE_TASK="$task" GRID_SKIP=1 bash "$(w cleanup-worker.sh)" "$revname" "$repo" >&2 \
          || echo "warning: cleanup $revname failed; clean it manually" >&2
        grid_hook "$builder-$task" || true
        blocker="reviewer $revname challenged the premise — Lead resolves the Challenge above"
        finish disputed 0
      fi
      verdict=$(grep -E '^[[:space:]]*Verdict:' "$runs/$revname.md" | tail -1 || true)
      plog "review round $round: ${verdict:-NO VERDICT LINE} ($revname.md)"
      PIPELINE_TASK="$task" GRID_SKIP=1 bash "$(w cleanup-worker.sh)" "$revname" "$repo" >&2 \
        || echo "warning: cleanup $revname failed; clean it manually" >&2
      grid_hook "$builder-$task" || true
      if [[ $verdict =~ [Vv]erdict:[[:space:]]*([0-9]+)[[:space:]]*[Ff]indings?[[:space:]]*\(([0-9]+)[[:space:]]*blocking\) ]]; then
        last_findings=${BASH_REMATCH[2]}   # nits never start a fix round
        (( last_findings > 0 )) || plog "review round $round: ${BASH_REMATCH[1]} nit(s), 0 blocking — clean"
      elif [[ $verdict =~ [Vv]erdict:[[:space:]]*([0-9]+)[[:space:]]*[Ff]indings? ]]; then
        last_findings=${BASH_REMATCH[1]}
      elif [[ $verdict =~ [Vv]erdict:[[:space:]]*[Cc]lean ]]; then
        if grep -qE '^[[:space:]]*-[[:space:]]*\[blocking\]' "$runs/$revname.md"; then
          blocker="reviewer $revname wrote 'Verdict: clean' beside a [blocking] finding — Lead decides"
          finish blocked 0
        fi
        last_findings=0
      else
        blocker="reviewer $revname runs file has no 'Verdict: clean | <N> findings' line (lock §1) — Lead decides"
        finish blocked 0
      fi
      if (( last_findings == 0 )); then finish_clean; fi
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
      if grep -q '^## Challenge' "$runs/$builder-$task.fix$round.md"; then
        log_challenge "$runs/$builder-$task.fix$round.md"
        blocker="builder disputed review findings — Lead resolves the Challenge above"
        finish disputed 0
      fi
      if blocked_outcome "$runs/$builder-$task.fix$round.md"; then
        log_blockers "$runs/$builder-$task.fix$round.md"
        blocker="builder's fix report is unfinished (see above) — Lead decides how to proceed"
        finish blocked 0
      fi
      check_impl "$runs/$builder-$task.fix$round.md" "$head_sha"  # must advance past the reviewed sha
      plog "fix $round: commit $commit ($builder-$task.fix$round.md)"
      phase=review round=$((round + 1)) prompted=0 reviewer_spawned=0; save_state ;;

    *) die "corrupt state: phase=$phase ($state)" ;;
  esac
done
