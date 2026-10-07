#!/usr/bin/env bash
# com.quaqua.supervisor — read-only drift & stale-doc review of ~/quaqua.
#
# Runs every 2 days via launchd. Collects herdr agent state + workspace docs,
# pre-verifies referenced paths/commands, then asks codex (gpt-6.1-sol, effort high)
# to produce /tmp/sup-YYYY-MM-DD.md. Fires a herdr notification (osascript fallback).
#
# Read-only contract: the only file this script writes is its report under /tmp
# (plus launchd-managed logs under supervisor/logs/). codex runs `exec` in a
# read-only sandbox from an empty scratch dir, ephemeral, user config ignored;
# every file it needs is read by this wrapper and passed on stdin. It never deletes anything, never calls
# `herdr agent prompt`/`send-keys`, never edits docs.

# Needs bash >= 4 (mapfile, assoc arrays). Re-exec if launched by /bin/bash 3.2.
if [ "${BASH_VERSINFO[0]:-0}" -lt 4 ]; then
  for _b in /opt/zerobrew/bin/bash /opt/homebrew/bin/bash /usr/local/bin/bash; do
    [ -x "$_b" ] && exec "$_b" "$0" "$@"
  done
  echo "supervisor.sh requires bash >= 4" >&2
  exit 1
fi
unset _b

set -uo pipefail

# --- Environment (launchd gives a bare PATH) ----------------------------------
export PATH="/usr/bin:/bin:/usr/sbin:/sbin"
export LANG="${LANG:-en_US.UTF-8}"

SUP_DIR="$HOME/quaqua/supervisor"
WORKSPACE="$HOME/quaqua"
TODAY="$(date +%F)"
OUT="/tmp/sup-${TODAY}.md"
MODEL="gpt-6.1-sol"
EFFORT="high"
CODEX="/opt/homebrew/bin/codex"
HERDR="/opt/zerobrew/bin/herdr"
JQ="/usr/bin/jq"
OSASCRIPT="/usr/bin/osascript"
MAX_AGENTS=8
AGENT_LINES=300
AGENT_READ_MAX_BYTES=40000
DOC_MAX_BYTES=60000
CODEX_TIMEOUT=900       # seconds for the codex call
CMD_TIMEOUT=15          # seconds for helper calls

cd "$SUP_DIR" 2>/dev/null || true
log() { printf '%s [sup] %s\n' "$(date '+%H:%M:%S')" "$*" >&2; }

# --- Resolve codex -----------------------------------------------------------------
if [ ! -x "$CODEX" ]; then
  log "FATAL: codex not found at $CODEX"
  exit 1
fi
export PATH="$(dirname "$CODEX"):$PATH"
log "codex: $CODEX ($("$CODEX" --version 2>/dev/null || echo '?'))"

# --- Kill-after-N-seconds (macOS has no `timeout`) ------------------------------
with_timeout() {
  local secs="$1"; shift
  "$@" &
  local pid=$!
  ( sleep "$secs"; kill -9 "$pid" 2>/dev/null ) &
  local killer=$!
  wait "$pid" 2>/dev/null
  local rc=$?
  kill "$killer" 2>/dev/null
  wait "$killer" 2>/dev/null
  return "$rc"
}

# --- Input 1: herdr agent list + per-agent terminal reads -----------------------
HERDR_OK=0
AGENT_LIST_JSON=""
AGENT_READS=""
agents=()

if [ -x "$HERDR" ]; then
  if AGENT_LIST_JSON="$(with_timeout "$CMD_TIMEOUT" "$HERDR" agent list 2>/dev/null)" \
     && printf '%s' "$AGENT_LIST_JSON" | "$JQ" -e '.result.agents' >/dev/null 2>&1; then
    HERDR_OK=1
  else
    log "herdr agent list failed — server down? Continuing without live agents."
    AGENT_LIST_JSON=""
  fi
else
  log "herdr not found at $HERDR"
fi

if [ "$HERDR_OK" = 1 ]; then
  mapfile -t agents < <(printf '%s' "$AGENT_LIST_JSON" | "$JQ" -c \
    '.result.agents[] | {pane: .pane_id, name: (.name // .terminal_title_stripped // "?"), agent, status: .agent_status, cwd}' \
    2>/dev/null | head -n "$MAX_AGENTS")
  for row in "${agents[@]}"; do
    target="$(printf '%s' "$row" | "$JQ" -r '.pane // .name')"
    label="$(printf '%s' "$row" | "$JQ" -r '"\(.name) [\(.agent)/\(.status)] \(.cwd)"')"
    read_out="$(with_timeout "$CMD_TIMEOUT" "$HERDR" agent read "$target" \
      --source recent-unwrapped --lines "$AGENT_LINES" 2>/dev/null | head -c "$AGENT_READ_MAX_BYTES")"
    AGENT_READS+=$'\n'"### agent: ${label} (target=${target})"$'\n'
    AGENT_READS+="${read_out:-<read failed>}"$'\n'
  done
  log "herdr: ${#agents[@]} agent(s), reads collected"
fi

# --- Input 2: workspace docs ----------------------------------------------------
now="$(date +%s)"
DOC_MANIFEST=""
DOC_BODIES=""
collect_doc() {
  local f="$1" mt sz age
  mt="$(stat -f %m "$f" 2>/dev/null || echo 0)"
  sz="$(stat -f %z "$f" 2>/dev/null || echo 0)"
  age=$(( (now - mt) / 86400 ))
  DOC_MANIFEST+="${f}|$(stat -f '%Sm' -t '%Y-%m-%d %H:%M' "$f" 2>/dev/null)|${age}d|${sz}B"$'\n'
  DOC_BODIES+=$'\n'"### FILE: ${f}  (mtime age ${age}d)"$'\n'
  DOC_BODIES+="$(head -c "$DOC_MAX_BYTES" "$f")"$'\n'
}

for f in "$WORKSPACE"/*.md; do [ -f "$f" ] && collect_doc "$f"; done
while IFS= read -r f; do collect_doc "$f"; done \
  < <(find "$WORKSPACE/memory" -name '*.md' -mtime -2 2>/dev/null | sort)
for f in "$WORKSPACE"/projects/*.md; do [ -f "$f" ] && collect_doc "$f"; done

# --- Input 3: dead-reference precheck --------------------------------------------
# Verify referenced paths/commands/flags HERE so the model gets facts. Only
# definite misses are reported — unverifiable flags are skipped (empty help
# output means "unknown", never "dead"), and flag checks only run for an
# allowlist of harmless CLIs.
SAFE_CMDS="pi herdr git jq node npm brew launchctl plutil osascript find stat"
SAFE_CMDS_RE="$(printf '%s' "$SAFE_CMDS" | tr ' ' '|')"
REFS_DEAD=""
REFS_CHECKED=0
declare -A HELP_CACHE=()

record_dead() { REFS_DEAD+="$1|$2|$3|$4|$5"$'\n'; }

check_one_ref() {
  local f="$1" tok="$2" kind="" ok="" line=""
  case "$tok" in
    //*|*://*) return 0 ;;                          # URLs, not paths
    ~*|/*)
      kind="path"
      [ -e "${tok/#\~/$HOME}" ] && ok=yes || ok=no ;;
    *--*)
      local cmd flag helptarget key
      helptarget="$(printf '%s' "$tok" | sed 's/--.*//' | awk '{$1=$1};1')"
      cmd="${helptarget%% *}"
      case " $SAFE_CMDS " in *" $cmd "*) ;; *) return 0 ;; esac
      flag="$(printf '%s' "$tok" | grep -oE '\--[a-zA-Z-]+' | head -1)"
      [ -z "$flag" ] && return 0
      kind="cmd-flag"
      key="$(printf '%s' "$helptarget" | tr ' ' '_')"
      if [ -z "${HELP_CACHE[$key]+x}" ]; then
        # shellcheck disable=SC2086 # intentional split: helptarget is "cmd [sub]"
        HELP_CACHE[$key]="$(with_timeout "$CMD_TIMEOUT" $helptarget -h 2>&1 | head -120)"
        [ -z "${HELP_CACHE[$key]}" ] && \
          HELP_CACHE[$key]="$(with_timeout "$CMD_TIMEOUT" $helptarget --help 2>&1 | head -120)"
      fi
      if [ -z "${HELP_CACHE[$key]}" ]; then
        return 0                                    # unverifiable, not "dead"
      fi
      printf '%s' "${HELP_CACHE[$key]}" | grep -qF -- "$flag" && ok=yes || ok=no ;;
    */*|*.md|*.ts|*.sh|*.json|*.toml|*.mjs|*.js|*.py|*.txt)
      kind="rel-path"
      local d; d="$(dirname "$f")"
      if   [ -e "$d/$tok" ]; then ok=yes
      elif [ -e "$WORKSPACE/$tok" ]; then ok=yes
      elif [ -e "$HOME/$tok" ]; then ok=yes
      else ok=no; fi ;;
    [a-zA-Z]*)
      case "$tok" in
        *[!a-zA-Z0-9_-]*) return 0 ;;
      esac
      [ "${#tok}" -gt 30 ] && return 0
      kind="cmd"
      command -v "$tok" >/dev/null 2>&1 && ok=yes || ok=no ;;
    *) return 0 ;;
  esac
  REFS_CHECKED=$((REFS_CHECKED + 1))
  if [ "$ok" = no ]; then
    line="$(grep -nF -- "$tok" "$f" 2>/dev/null | head -1 | cut -d: -f1)"
    record_dead "$f" "${line:-?}" "$tok" "$kind" "not found"
  fi
}

ref_check_file() {
  local f="$1"
  local toks tok
  toks="$( { grep -oE '`[^`]{1,120}`' "$f" 2>/dev/null | tr -d '`'
            grep -oE '(~|/)[A-Za-z0-9_./-]{2,}' "$f" 2>/dev/null
            grep -oE "\b($SAFE_CMDS_RE)[[:space:]]+([a-z-]+[[:space:]]+)?--[a-zA-Z-]+" "$f" 2>/dev/null
          } | sort -u | head -40 )"
  while IFS= read -r tok; do
    tok="${tok%%[.,;:)\]]*}"; tok="${tok##[(\[]*}"
    [ -z "$tok" ] && continue
    check_one_ref "$f" "$tok"
  done <<< "$toks"
}

for f in "$WORKSPACE"/*.md "$WORKSPACE"/projects/*.md; do
  [ -f "$f" ] && ref_check_file "$f"
done
while IFS= read -r f; do ref_check_file "$f"; done \
  < <(find "$WORKSPACE/memory" -name '*.md' -mtime -2 2>/dev/null | sort)
DEAD_COUNT="$(printf '%s' "$REFS_DEAD" | grep -c . || true)"
log "refcheck: $REFS_CHECKED refs checked, $DEAD_COUNT dead"

# --- Prompt ----------------------------------------------------------------------
read -r -d '' INSTR <<'INSTR' || true
You are a read-only supervisor for a personal agent workspace at ~/quaqua.
Produce a markdown review with EXACTLY these sections, in this order:

# Supervisor review — __DATE__
## Drift
Bullet points: inconsistencies between what live agents are doing and what the
docs claim, config drift, agents stuck/idle long past their task, duplicated or
abandoned work. Say "none observed" if clean.
## Stale docs
A table: | file | line | why | suggested action |
Flag a doc when its content references files/commands/flags that no longer
exist (the dead-reference precheck below is verified evidence — use it), when
two files contradict each other, or when a file is old relative to the code or
activity it describes (mtime ages are in the manifest; the workspace is NOT a
git repo). No fixed line-count rule — judge content, not age alone.
## Decisions for me
Numbered list of concrete choices the owner must make (keep/update/delete,
which source of truth wins). Nothing here should be auto-fixable — this run is
advisory only.

Rules: do not run commands or read files; everything you may cite is below. Do not invent file
contents or agent states. Keep it under 150 lines. End with a single line:
SUMMARY: <one sentence, plain text, <=120 chars>
INSTR
INSTR="${INSTR//__DATE__/$TODAY}"

PROMPT="${INSTR}

=== INPUT: herdr agent list (JSON) ===
${AGENT_LIST_JSON:-<herdr unavailable — no live agent data>}

=== INPUT: terminal snapshots (recent-unwrapped, ${AGENT_LINES} lines each) ===
${AGENT_READS:-<none>}

=== INPUT: doc manifest (path|mtime|age|size) ===
${DOC_MANIFEST:-<none>}

=== INPUT: doc contents ===
${DOC_BODIES:-<none>}

=== INPUT: dead-reference precheck (file|line|reference|kind|problem) ===
${REFS_DEAD:-<none found>} (${REFS_CHECKED} refs checked)"

# --- Run codex — read-only sandbox, empty cwd, no session ---------------------
log "running codex review -> $OUT"
SCRATCH="/tmp/sup-codex"
LAST="$SCRATCH/last.md"
mkdir -p "$SCRATCH" && : > "$LAST"
{
  printf '<!-- generated %s by com.quaqua.supervisor | model=%s/%s agents=%s refs_checked=%s dead=%s -->\n\n' \
    "$(date '+%Y-%m-%d %H:%M:%S')" "$MODEL" "$EFFORT" "${#agents[@]}" "$REFS_CHECKED" "$DEAD_COUNT"
  printf '%s' "$PROMPT" | with_timeout "$CODEX_TIMEOUT" "$CODEX" exec \
    -m "$MODEL" -c "model_reasoning_effort=\"$EFFORT\"" \
    -s read-only -C "$SCRATCH" --skip-git-repo-check --ephemeral \
    --ignore-user-config --ignore-rules --color never \
    -o "$LAST" - >/dev/null
  rc=$?
  [ -s "$LAST" ] && cat "$LAST"
  if [ "$rc" -ne 0 ]; then
    printf '\n\n> **supervisor run failed** (codex exit %s) — check %s/logs/\n' "$rc" "$SUP_DIR"
  fi
} > "$OUT" 2>>"$SUP_DIR/logs/stderr.log"

if [ ! -s "$OUT" ]; then
  printf '# Supervisor review — %s\n\n**Run produced no output.** See %s/logs/.\n' "$TODAY" "$SUP_DIR" > "$OUT"
fi

# --- Notify ----------------------------------------------------------------------
SUMMARY="$(grep -m1 '^SUMMARY:' "$OUT" | sed 's/^SUMMARY:[[:space:]]*//' | cut -c1-160)"
[ -z "$SUMMARY" ] && SUMMARY="review written to $OUT"

NOTIFIED=0
if [ "$HERDR_OK" = 1 ]; then
  if with_timeout "$CMD_TIMEOUT" "$HERDR" notification show \
      "quaqua supervisor" --body "$SUMMARY" >/dev/null 2>&1; then
    NOTIFIED=1
  fi
fi
if [ "$NOTIFIED" = 0 ]; then
  "$OSASCRIPT" -e "display notification \"$SUMMARY\" with title \"quaqua supervisor\"" \
    >/dev/null 2>&1 || log "WARN: notification failed (herdr + osascript)"
fi

log "done: $OUT | notified=$([ "$NOTIFIED" = 1 ] && echo herdr || echo osascript/failed)"
exit 0
