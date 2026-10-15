#!/usr/bin/env bash
# Usage: spawn-worker.sh [--base <ref>] [--review] <name> <kind: tieude|detu|sude|ni|suhuynh|truonglao|bacdau|quansu> <repo-path>
#   name = <kind>-<task>, ascii [a-z0-9-], <=20 chars, not ending in -lead
#          (*-lead is the project Lead convention — spawn-lead.sh <slug>-lead)
#   --base <ref>  start branch quaqua/<name> at <ref> instead of <repo> HEAD
#   --review      codex kinds (bacdau|quansu) only: repo read-only, $HOME/quaqua/runs writable
#   Resume-safe: when $wt is already a registered worktree of <repo> (a
#   previous attempt died after creating it) it is reused and startup
#   continues with a fresh pane/agent. A pane this attempt created is closed
#   if the agent never becomes ready; the worktree+branch stay for resume.
#   A dir at $wt that is not a worktree of <repo> is refused, never reused.
set -euo pipefail

USAGE_LAST=5
. "$(dirname "$0")/../lib/common.sh"

pos=()
while (($#)); do
  case $1 in
    --base)   (($# >= 2)) || usage; base=$2; shift 2 ;;
    --base=*) base=${1#--base=}; shift ;;
    --review) review=1; shift ;;
    *)        pos+=("$1"); shift ;;
  esac
done
if [[ ${base+x} && ( -z $base || $base == -* ) ]]; then usage; fi
if ((${#pos[@]} < 3)); then usage; fi
name=${pos[0]} kind=${pos[1]} repo=$(realpath "${pos[2]}")
case $kind in
  tieude|detu|sude|ni|suhuynh|truonglao|bacdau|quansu) ;;
  *) sed -n '2p' "$0" >&2; exit 1 ;;
esac
if [[ ! $name =~ ^$kind-[abcdefghijklmnopqrstuvwxyz0123456789-]+$ ]] || (( ${#name} > 20 )) \
    || [[ $name == *-lead ]]; then
  sed -n '2,3p' "$0" >&2; exit 1
fi
case $kind in bacdau|quansu) ;; *)
  if [[ ${review+x} ]]; then
    echo "error: --review is only supported for codex kinds (bacdau, quansu), not $kind" >&2
    usage
  fi ;;
esac

wt="$(dirname "$repo")/.wt/$name"

# --base must resolve to a commit before anything is created (worktree, rules, pane)
if [[ ${base+x} ]]; then
  git -C "$repo" rev-parse --verify --quiet --end-of-options "${base}^{commit}" >/dev/null \
    || { echo "error: --base '$base' is not a known commit-ish in $repo" >&2; exit 1; }
fi

# Worker cap: max 4 task workers per orchestrator; pipelines reserve 2
owner=${QUAQUA_OWNER:-cos}
runs_dir="$HOME/quaqua/runs"
mkdir -p "$runs_dir"

if [[ -z "${PIPELINE_TASK:-}" ]]; then
  slots_used=$(count_owner_slots "$runs_dir" "$owner" "$name")
  if (( slots_used >= 4 )); then
    echo "error: worker cap exceeded ($slots_used live worker slots for owner '$owner'; cap is 4) — cannot spawn $name" >&2
    exit 1
  fi
fi

if [[ -d $wt ]]; then
  if git -C "$repo" worktree list --porcelain | grep -qxF "worktree $wt"; then
    echo "note: $wt already a registered worktree — reusing it (resumed spawn)" >&2
  else
    echo "error: $wt exists but is not a git worktree of $repo — remove it or pick another name" >&2
    exit 1
  fi
else
  git -C "$repo" worktree add -b "quaqua/$name" "$wt" ${base:+"$base"} >&2
fi

# Push block (AGENTS.md handoff rule): task workers never push — branches are
# merged by the Lead/CoS. Each remote's pushurl is pointed at an unresolvable
# value in the worktree's own config (extensions.worktreeConfig keeps it out
# of the shared config), so `git push` fails before any network while the
# main checkout's push target stays untouched. Runs on add AND on reuse so a
# pre-existing worktree gets retrofitted. A guardrail, not a boundary: a
# worker could still push a brand-new remote or an explicit URL, or unset it.
#
# Fail closed on a repo whose common config carries core.bare=true or
# core.worktree: enabling extensions.worktreeConfig there drops the
# exception that keeps those keys applying to the main worktree only
# (git-worktree(1), CONFIGURATION FILE), so every worktree — including the
# CoS checkout — would break. Refuse rather than spawn a half-broken worker.
if [[ $(git -C "$repo" config --local --bool --get core.bare 2>/dev/null) == true ]] \
    || git -C "$repo" config --local --list 2>/dev/null | grep -q '^core\.worktree='; then
  echo "error: $repo has core.bare=true or core.worktree in its common config;" \
       "enabling extensions.worktreeConfig for the push block would break every" \
       "worktree of it. Move those keys to the main worktree's config.worktree" \
       "(see git-worktree(1), CONFIGURATION FILE) or spawn from another clone." >&2
  exit 1
fi
# --replace-all '' then --add, not a plain set: pushurl is a LIST and a push
# goes to every entry, so a pushurl already in the shared config would
# otherwise still receive the push (verified). The leading empty entry clears
# that inherited list ("" clears the list of urls); the blocked value is then
# ADDED so it survives re-running this on a reused worktree.
git -C "$repo" config extensions.worktreeConfig true
while IFS= read -r _remote; do
  if [[ -n $_remote ]]; then
    git -C "$wt" config --worktree --replace-all "remote.$_remote.pushurl" ''
    git -C "$wt" config --worktree --add "remote.$_remote.pushurl" \
      'PUSH DISABLED - task workers never push (AGENTS.md handoff rule)'
  fi
done < <(git -C "$repo" remote)

# worker rules: rules/_all/*.md + rules/<repo-basename>/*.md -> one file, injected as system prompt
rules_dir="$HOME/quaqua/rules" rf="$HOME/quaqua/runs/$name.rules.md"
mkdir -p "$(dirname "$rf")"   # fresh clones have no runs/ yet — create it before writing
cat "$rules_dir"/_all/*.md "$rules_dir/$(basename "$repo")"/*.md 2>/dev/null > "$rf" || true
codex_rules="developer_instructions=$(jq -Rs . < "$rf")"

# Record owner, grid group (grid-workers.sh groups by owner + project tab),
# pipeline association, sandbox marker, and report class. `review` is the
# spawn metadata prompt-worker.sh classifies the runs report by: a reviewer
# ends with a `Verdict:` line, a builder with `## Lessons` (rules/_all/
# brief.md). Never derived from the brief text — briefs quote that contract.
grid_tab=${GRID_TAB:-"$(basename "$repo")-workers"}
echo "$owner" > "$runs_dir/$name.owner"
echo "$grid_tab" > "$runs_dir/$name.grid"
[[ -n "${PIPELINE_TASK:-}" ]] && echo "$PIPELINE_TASK" > "$runs_dir/$name.pipeline"
iso_now=$(date -Iseconds 2>/dev/null || date '+%Y-%m-%dT%H:%M:%S%z')
review_json=false
[[ ${review+x} ]] && review_json=true
printf '{"name":"%s","kind":"%s","created":"%s","sandbox":true,"owner":"%s","review":%s}\n' \
  "$name" "$kind" "$iso_now" "$owner" "$review_json" > "$runs_dir/$name.srt.json"

# codex access args. --review keeps the repo read-only and $HOME/quaqua/runs writable
# via a permission profile extending the built-in :read-only one. Plain
# `-s read-only` cannot do this: codex only honours --add-dir under
# workspace-write (a read-only sandbox makes runs/ read-only too), and
# --approve-for-me conflicts with an explicit -s.
if [[ ${review+x} ]]; then
  access=(-a never -c 'default_permissions="quaqua-review"'
          -c 'permissions.quaqua-review.extends=":read-only"'
          -c "permissions.quaqua-review.filesystem={\"$HOME/quaqua/runs\"=\"write\"}")
else
  access=(--approve-for-me --add-dir "$HOME/quaqua/runs")
fi

oc_env=()
[[ $kind == tieude ]] && oc_env=(--env "OPENCODE_CONFIG_CONTENT={\"model\":\"opencode-go/deepseek-v4.1-flash\",\"instructions\":[\"$rf\"]}")
split=$(herdr pane split --current --direction right --no-focus --cwd "$wt" --env "QUAQUA_SANDBOX=1" --env "QUAQUA_OWNER=$owner" "${oc_env[@]}")
pane=$(printf '%s\n' "$split" | jq -r '.result.pane.pane_id')

# if the agent never becomes ready, roll back only what this attempt created
# (the pane); the worktree+branch stay so a --resume can reuse them (F4)
spawn_fail() { herdr pane close "$pane" >/dev/null 2>&1 || true; exit "${1:-1}"; }

# Pane tokens (herdr-grid R5): role/group/owner right after split, grid_ready=1
# set last after trust/send-keys handling
grid_group=$(basename "$repo")
herdr pane report-metadata "$pane" --source herdr-grid \
  --token grid_role=worker \
  --token "grid_group=$grid_group" \
  --token "grid_owner=$owner" >/dev/null || spawn_fail

case $kind in
  tieude)
    herdr agent start "$name" --kind opencode --pane "$pane" --timeout 60000 -- --standalone --auto >/dev/null || spawn_fail ;;
  sude|detu)
    case $kind in
      sude) args=(--model google-antigravity/gemini-3.8-flash --thinking high) ;;
      detu) args=(--model google-antigravity/gemini-3.8-flash --thinking high) ;;
    esac
    herdr agent start "$name" --kind omp --pane "$pane" --timeout 60000 -- "${args[@]}" --append-system-prompt "$rf" >/dev/null || spawn_fail ;;
  ni|suhuynh|truonglao)
    case $kind in ni) effort=low ;; suhuynh) effort=medium ;; truonglao) effort=high ;; esac
    herdr agent start "$name" --kind claude --pane "$pane" --timeout 60000 -- --model claude-opus-5-5 --effort "$effort" --append-system-prompt-file "$rf" >/dev/null || spawn_fail ;;
  bacdau)
    herdr agent start "$name" --kind codex --pane "$pane" --timeout 300000 -- "${access[@]}" -m gpt-6-astra -c model_reasoning_effort=medium -c "$codex_rules" >/dev/null || codex_start_rc=$? ;;
  quansu)
    herdr agent start "$name" --kind codex --pane "$pane" --timeout 300000 -- "${access[@]}" -m gpt-6.1-sol -c model_reasoning_effort=high -c "$codex_rules" >/dev/null || codex_start_rc=$? ;;
esac
case $kind in bacdau|quansu)
  # herdr's start timeout is not proof of failure for codex: the TUI can sit
  # on "Trust this folder?" (new worktree, untrusted dir) or spend minutes on
  # app-server-daemon init before the SessionStart hook reports the agent —
  # herdr only sees 'timed out waiting for agent startup'. Fail loudly, then
  # keep going: accept the trust dialog if present and wait for idle.
  if [[ ${codex_start_rc:-0} -ne 0 ]]; then
    echo "warning: herdr agent start $name failed (rc=$codex_start_rc); continuing to trust/idle wait" >&2
  fi
  # new worktree -> codex asks "Trust this folder?"; accept default "Trust and continue".
  # start may already have failed with agent_not_ready while the dialog blocked readiness, so confirm idle here
  if herdr pane wait-output "$pane" --match "Trust this folder" --timeout 30000 >/dev/null 2>&1; then
    herdr pane send-keys "$pane" enter >/dev/null
  fi
  herdr agent wait "$name" --until idle --timeout 300000 >/dev/null || spawn_fail
esac

# grid_ready=1 set last, after trust/send-keys handling (R5)
herdr pane report-metadata "$pane" --source herdr-grid --token grid_ready=1 >/dev/null || spawn_fail

# Grid hook (rules/_all/fanout-agent-grid.md): arrange the new worker's pane
# around the caller's pane. Best-effort: a grid failure warns, never fails the
# spawn. The worker's group tab is GRID_TAB (default <repo-basename>-workers).
# GRID_SKIP=1 is honoured only for callers that grid themselves (pipeline.sh,
# PIPELINE_TASK set) or tests (QUAQUA_TEST=1); otherwise it is ignored.
grid_hook() {
  if [[ "${GRID_SKIP:-}" == 1 ]]; then
    [[ -n "${PIPELINE_TASK:-}" || "${QUAQUA_TEST:-}" == 1 ]] && return 0
    echo "warning: GRID_SKIP=1 ignored outside pipeline.sh/tests (rules/_all/fanout-agent-grid.md)" >&2
  fi
  local grid
  grid="$(dirname "$0")/grid-workers.sh"
  [[ -f "$grid" ]] || return 0
  bash "$grid" --check --tab "$grid_tab" >&2 \
    || echo "warning: grid-workers.sh failed (exit $?); worker $name spawned, arrange panes manually" >&2
}

echo "worker=$name kind=$kind pane=$pane worktree=$wt"
grid_hook
