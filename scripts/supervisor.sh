#!/usr/bin/env bash
# Usage: supervisor.sh
#   Read-only drift audit of this repo, run by the quaqua-supervisor systemd
#   user timer (2x/day, see systemd/quaqua-supervisor.timer). Builds the prompt,
#   runs `codex exec` in a read-only sandbox, and writes the agent's final
#   message to reports/supervisor-YYYY-MM-DD-HH.md via --output-last-message.
#   The agent cannot edit anything; this script is the only writer.
#   Before the prompt it runs cheap deterministic pre-checks on pipeline runs
#   (runs/pipeline-*.md, spec-raw/quaqua-slp-slim-lock.md §3): unhandled
#   disputed|blocked, clean with an unresolvable commit or no reviewer
#   'Verdict:', and unresolved '## Challenge' sections. Findings are embedded
#   in the prompt for the agent to verify, correct, and extend.
#   Env: SUPERVISOR_MODEL overrides the model (default gpt-6.1-sol, the quansu
#   tier); SUPERVISOR_OUT overrides the output path (tests).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
out=${SUPERVISOR_OUT:-"$root/reports/supervisor-$(date +%F-%H).md"}
mkdir -p "$(dirname "$out")"
log=$(mktemp)
trap 'rm -f "$log"' EXIT

# ── deterministic pipeline-run pre-checks ───────────────────────────────────
# Findings are advisory: the agent re-verifies them against the repo. The
# "handled" marker for a pipeline outcome is a memory/YYYY-MM-DD.md line
# naming pipeline-<task> (same convention as the [supervisor] report marker).
# runs/*.rules.md are rule dumps injected into worker prompts — they always
# contain a literal '## Challenge' describing the rule, so they are excluded.
precheck=''
flag() { precheck+="- $*"$'\n'; }

mem_mentions() {  # <needle...> — true when some memory/*.md file names a needle
  local f n
  for f in "$root"/memory/*.md; do
    [[ -e $f ]] || continue
    for n in "$@"; do grep -qF -- "$n" "$f" && return 0; done
  done
  return 1
}

for f in "$root"/runs/pipeline-*.md; do
  [[ -e $f ]] || continue
  base=$(basename "$f" .md)
  sline=$(grep -E '^- status: ' "$f" | tail -1 || true)
  st=$(sed -nE 's/^- status: ([a-z-]+).*/\1/p' <<<"$sline")
  case $st in
    disputed|blocked)
      mem_mentions "$base" \
        || flag "runs/$base.md: status $st but no memory/YYYY-MM-DD.md line names $base" ;;
    clean)
      sha=$(sed -nE 's/.*commit ([^ ]+).*/\1/p' <<<"$sline")
      if [[ -z $sha || $sha == none || $sha == '?' ]] \
          || ! git -C "$root" rev-parse --verify --quiet "${sha}^{commit}" >/dev/null 2>&1; then
        flag "runs/$base.md: status clean but commit '${sha:-none}' does not resolve in this repo"
      fi
      last_rev=$(grep -E 'review round [0-9]+:' "$f" | tail -1 || true)
      [[ -z $last_rev || $last_rev == *Verdict:* ]] \
        || flag "runs/$base.md: status clean, review ran, but no 'Verdict:' on the last review-round line" ;;
  esac   # plan-ready pauses and unfinished runs are not outcomes — skip
done
for f in "$root"/runs/*.md; do
  [[ -e $f ]] || continue
  case $f in *.rules.md) continue ;; esac
  grep -q '^## Challenge' "$f" || continue
  base=$(basename "$f" .md)
  copied=''   # already copied into a pipeline file's '## Challenge (from X)':
  for p in "$root"/runs/pipeline-*.md; do  # that file's own flag covers it
    [[ -e $p && $p != "$f" ]] || continue
    grep -qF "(from $base.md)" "$p" && { copied=1; break; }
  done
  [[ -z $copied ]] || continue
  short=${base%.plan}; short=${short%.build}; short=${short%.fix[0-9]}
  if [[ $short == "$base" ]]; then
    mem_mentions "$base" \
      || flag "runs/$base.md: '## Challenge' unresolved — no memory/YYYY-MM-DD.md line names $base"
  else   # pipeline phase file (<worker>-<task>.plan|build|fixN) — try worker name
    mem_mentions "$base" "$short" \
      || flag "runs/$base.md: '## Challenge' unresolved — no memory/YYYY-MM-DD.md line names $base or $short"
  fi
done

prompt=$(cat <<'EOF'
You are the Auditor for the quaqua repo. READ-ONLY: inspect and report only.
Do not edit, create, or delete any file; do not run mutating commands; do not
contact, prompt, or instruct any agent. If a redirect is needed, flag it in the
report — the CoS routes it.

Audit four things:
1. Drift in CoS activity. Read memory/ files from the last 2 days, projects/*.md,
   and recent `git log` (last ~30 commits). Flag: claimed outcomes the code or
   git history does not back; project "Now"/"Log" entries contradicted by repo
   state; memory rules or lessons that rest on stale facts.
2. Stale docs vs real code/scripts. Flag: AGENTS.md, memory/rules.md, models.md,
   rules/ entries that reference removed scripts/behaviors or describe a flow the
   scripts no longer implement; scripts or docs/plans that contradict each other.
3. Unhandled memory. Flag: reports/supervisor-*.md with no
   `- [supervisor] <report file>:` line in any memory/YYYY-MM-DD.md (the
   handled marker CoS writes, see AGENTS.md); `## Lessons` lines (other than
   `none`) in runs/*.md from the last 2 days that no rules/ file cites as
   `runs/<name>.md`.
4. Pipeline runs (runs/pipeline-*.md, written by scripts/pipeline.sh; the
   last `- status:` line is the outcome). Flag:
   - `status: disputed|blocked` with no memory/YYYY-MM-DD.md line naming
     `pipeline-<task>` (the CoS handling note, same convention as the
     `[supervisor]` report marker);
   - `status: clean` whose `commit` is `none`/`?` or that
     `git rev-parse --verify <sha>^{commit}` cannot resolve in this repo;
   - `status: clean` where review ran (`- review round <N>:` lines — not
     `review: skipped`) but neither the last review-round line nor the
     reviewer runs file it names carries a `Verdict:` line;
   - a `## Challenge` section in any runs/*.md (except *.rules.md — injected
     rule dumps where the word describes a rule, not a challenge) with no
     memory line naming the worker. Challenges are resolved by the Lead
     before acceptance; no handling line = still open. A Challenge already
     copied into a pipeline file's `## Challenge (from <file>)` section is
     covered by that file's flag — do not report it twice.
   Deterministic pre-check — the script already ran the cheap greps; list
   each finding below under `## Pipeline` unless repo evidence contradicts
   it, and add any flag the greps missed:
__PRECHECK__

Write your whole report as the final message — it becomes the report file.
Format: `## Drift`, `## Stale docs`, `## Unhandled` and `## Pipeline`
sections, each `- file: finding + evidence` (cite path/commit), one line each.
No findings in a section → write `- none`. End with one line
`Verdict: clean | N findings`. Terse, technical.
EOF
)
pc=${precheck%$'\n'}
prompt=${prompt//__PRECHECK__/${pc:-- none}}

args=(exec --ephemeral -C "$root" --sandbox read-only
  -c project_doc_max_bytes=0
  -o "$out")
[[ -n ${SUPERVISOR_MODEL:-} ]] && args+=(-m "$SUPERVISOR_MODEL") || args+=(-m gpt-6.1-sol)
args+=("$prompt")

rc=0
codex "${args[@]}" 2>"$log" >/dev/null || rc=$?
if (( rc != 0 )); then
  cat "$log" >&2
  [[ -s $out ]] || printf '# Supervisor %s — FAILED\n\n`codex exec` exited %d; stderr went to the journal.\n' \
    "$(date '+%F %H:%M')" "$rc" > "$out"
  exit "$rc"
fi
printf '%s\n' "$out"
