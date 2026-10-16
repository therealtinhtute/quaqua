#!/usr/bin/env bash
# Usage: auditor.sh
#   Read-only drift audit of this repo, run by systemd (Linux) or launchd
#   (macOS), 2x/day. See scripts/auditor/systemd/ and scripts/auditor/launchd/.
#   Builds the prompt, runs `claude -p` (print mode) with a read-only tool
#   whitelist plus a deny rule for git's own `--output` flag and no settings
#   sources, and writes the agent's final message to
#   reports/audit-YYYY-MM-DD-HH.md (stdout). This script is the only writer.
#   Before the prompt it runs cheap deterministic pre-checks on pipeline runs
#   (runs/pipeline-*.md): unhandled disputed|blocked, clean with an
#   unresolvable commit or no reviewer
#   'Verdict:' (on the last review-round line or in the reviewer runs file it
#   names), and unresolved '## Challenge' sections. It also flags any live
#   task worker in herdr not started through the sandbox wrapper (missing
#   runs/<name>.srt.json) — and reports a failing `herdr agent list` instead
#   of silently finding nothing — and flags instruction docs over 100 lines
#   (doc-rot). Findings are embedded in the prompt for the
#   agent to verify, correct, and extend.
#   Env: AUDITOR_MODEL overrides the model (default claude-opus-5-5);
#   AUDITOR_OUT overrides the output path (tests).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
out=${AUDITOR_OUT:-${SUPERVISOR_OUT:-"$root/reports/audit-$(date +%F-%H).md"}}
mkdir -p "$(dirname "$out")"
log=$(mktemp)
herdr_err=$(mktemp)
trap 'rm -f "$log" "$herdr_err"' EXIT

# ── deterministic pipeline-run pre-checks ───────────────────────────────────
# Findings are advisory: the agent re-verifies them against the repo. The
# "handled" marker for a pipeline outcome is a memory/YYYY-MM-DD.md line
# naming pipeline-<task> (same convention as the [audit] report marker).
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
      # pipelines build in their project repo, recorded in the report's
      # '- repo:' header — resolve the commit THERE, not in this checkout
      prepo=$(sed -nE 's/^- task: [^ ]+ · repo: (.+)$/\1/p' "$f" | head -1)
      prepo=${prepo/#\~/"$HOME"}; prepo=${prepo:-$root}
      if [[ ! -d $prepo ]]; then
        flag "runs/$base.md: status clean but recorded repo '$prepo' is missing"
      elif [[ -z $sha || $sha == none || $sha == '?' ]] \
          || ! git -C "$prepo" rev-parse --verify --quiet "${sha}^{commit}" >/dev/null 2>&1; then
        flag "runs/$base.md: status clean but commit '${sha:-none}' does not resolve in recorded repo $prepo"
      fi
      last_rev=$(grep -E 'review round [0-9]+:' "$f" | tail -1 || true)
      # the verdict may live in the reviewer runs file the round line names:
      # either '(<file>.md)' or '<name> spawned on ...' -> runs/<name>.md
      if [[ -n $last_rev && $last_rev != *Verdict:* ]]; then
        revfile=$(grep -oE '\([^)]+\.md\)' <<<"$last_rev" | tail -1 | tr -d '()' || true)
        if [[ -z $revfile && $last_rev == *' spawned on '* ]]; then
          revfile=$(sed -nE 's/.*: ([^ ]+) spawned on .*/\1.md/p' <<<"$last_rev")
        fi
        [[ -n $revfile ]] && grep -qE '^[[:space:]]*Verdict:' "$root/runs/$revfile" \
          || flag "runs/$base.md: status clean, review ran, but no 'Verdict:' on the last review-round line or in the reviewer runs file it names"
      fi ;;
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

# ── deterministic live-worker sandbox pre-checks ────────────────────────────
agents='' herdr_rc=0
agents=$(herdr agent list 2>"$herdr_err") || herdr_rc=$?
if (( herdr_rc != 0 )); then
  herdr_msg=$(head -n 1 "$herdr_err" | cut -c1-200)
  flag "herdr agent list failed (exit $herdr_rc${herdr_msg:+: $herdr_msg}) — live-worker sandbox audit skipped"
elif ! jq -e . >/dev/null 2>&1 <<<"$agents"; then
  flag "herdr agent list returned no JSON — live-worker sandbox audit skipped"
else
  while IFS= read -r w; do
    [[ -n $w ]] || continue
    if [[ ! -f "$root/runs/$w.srt.json" ]]; then
      flag "live worker '$w': not started through sandbox wrapper (missing runs/$w.srt.json)"
    fi
  done < <(jq -r '.result.agents[]? | select((.name // "") | test("^(tieude|detu|sude|ni|suhuynh|truonglao)-") and (test("-lead$") | not)) | .name' <<<"$agents")
fi

# ── deterministic doc-rot pre-check ─────────────────────────────────────────
# Docs agents read as instructions drift from the code they describe once they
# grow long; agents then trust the stale doc over the code. Flag only — the
# CoS decides what to trim. Logs, evidence and reports (memory/YYYY-MM-DD.md,
# raw/, runs/, reports/) are excluded.
for f in "$root"/*.md "$root"/rules/*/*.md "$root"/projects/*.md "$root"/memory/rules.md; do
  [[ -f $f ]] || continue
  n=$(wc -l <"$f"); n=${n//[[:space:]]/}
  if (( n > 100 )); then
    flag "${f#"$root/"}: $n lines (>100) — doc-rot risk, trim or split"
  fi
done

prompt=$(cat <<'EOF'
You are the Auditor for the quaqua repo. READ-ONLY: inspect and report only.
Do not edit, create, or delete any file; do not run mutating commands; do not
contact, prompt, or instruct any agent. If a redirect is needed, flag it in the
report — the CoS routes it.

Audit five things:
1. Drift in CoS activity. Read memory/ files from the last 2 days, projects/*.md,
   and recent `git log` (last ~30 commits). Flag: claimed outcomes the code or
   git history does not back; project "Now"/"Log" entries contradicted by repo
   state; memory rules or lessons that rest on stale facts.
2. Stale docs vs real code/scripts. Flag: AGENTS.md, memory/rules.md, models.md,
   rules/ entries that reference removed scripts/behaviors or describe a flow the
   scripts no longer implement; scripts or docs/plans that contradict each other;
   instruction docs over 100 lines (doc-rot: agents trust them over the code).
3. Unhandled memory. Flag: reports/audit-*.md with no
   `- [audit] <report file>:` line in any memory/YYYY-MM-DD.md (the
   handled marker CoS writes, see AGENTS.md); `## Lessons` lines (other than
   `none`) in runs/*.md from the last 2 days that no rules/ file cites as
   `runs/<name>.md`.
4. Pipeline runs (runs/pipeline-*.md, written by scripts/lead/pipeline.sh; the
   last `- status:` line is the outcome). Flag:
   - `status: disputed|blocked` with no memory/YYYY-MM-DD.md line naming
     `pipeline-<task>` (the CoS handling note, same convention as the
     `[audit]` report marker);
   - `status: clean` whose `commit` is `none`/`?` or that
     `git rev-parse --verify <sha>^{commit}` cannot resolve in the pipeline's
     recorded repo (the `- repo:` header in runs/pipeline-<task>.md — Peers
     build in project repos, not this checkout; a missing recorded repo is a
     separate flag);
   - `status: clean` where review ran (`- review round <N>:` lines — not
     `review: skipped`) but neither the last review-round line nor the
     reviewer runs file it names carries a `Verdict:` line;
   - a `## Challenge` section in any runs/*.md (except *.rules.md — injected
     rule dumps where the word describes a rule, not a challenge) with no
     memory line naming the worker. Challenges are resolved by the Lead
     before acceptance; no handling line = still open. A Challenge already
     copied into a pipeline file's `## Challenge (from <file>)` section is
     covered by that file's flag — do not report it twice.
5. Live workers. Flag any live task worker in herdr not started through the
   sandbox wrapper (missing runs/<name>.srt.json); a `herdr agent list`
   failure is itself a finding — never report 'none' for it.
   Deterministic pre-check — the script already ran the cheap greps; list
   each finding below under `## Pipeline`, `## Workers` or (doc-rot)
   `## Stale docs` unless repo evidence
   contradicts it, and add any flag the greps missed:
__PRECHECK__

Write your whole report as the final message — it becomes the report file.
Format: `## Drift`, `## Stale docs`, `## Unhandled`, `## Pipeline`, and `## Workers`
sections, each `- file: finding + evidence` (cite path/commit), one line each.
No findings in a section → write `- none`. End with one line
`Verdict: clean | N findings`. Terse, technical.
EOF
)
pc=${precheck%$'\n'}
prompt=${prompt//__PRECHECK__/"${pc:-- none}"}

auditor_model=${AUDITOR_MODEL:-claude-opus-5-5}
# Read-only by tool whitelist, not only by prompt (finding 3, suhuynh-r6r1):
#   - no Write/Edit tool is granted and Bash is limited to three read-only git
#     commands;
#   - --disallowedTools removes those tools' own file-writing flag,
#     `git ... --output=<file>`, which the allow pattern would otherwise match;
#   - --setting-sources '' loads no user/project/local settings files, so no
#     ambient allow rule can re-widen the whitelist under dontAsk.
# cwd is the repo root so the agent's relative paths resolve there.
rc=0
(cd "$root" && claude -p --model "$auditor_model" --permission-mode dontAsk \
  --setting-sources "" \
  --allowedTools "Read,Grep,Glob,Bash(git log:*),Bash(git diff:*),Bash(git show:*)" \
  --disallowedTools "Bash(git *--output*)" \
  "$prompt") >"$out" 2>"$log" || rc=$?
if (( rc != 0 )); then
  cat "$log" >&2
  [[ -s $out ]] || printf '# Auditor %s — FAILED\n\n`claude -p` exited %d; see scheduler stderr logs (journal or Library/Logs/quaqua-auditor.log).\n' \
    "$(date '+%F %H:%M')" "$rc" > "$out"
  exit "$rc"
fi
printf '%s\n' "$out"
