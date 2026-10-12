#!/usr/bin/env bash
# PreToolUse(Bash) hook for the CoS and Leads (.claude/settings.json): pane
# placement and agent spawning go through scripts/ only (spawn-worker.sh,
# spawn-lead.sh, grid-workers.sh, cleanup-worker.sh, cleanup-lead.sh), so the
# harness — not the agent — decides tabs, team workspaces, owners and the
# 4-worker cap.
#
# Blocked (exit 2; stderr goes back to the agent): a simple command whose
# executable resolves to herdr and whose arguments hold a guarded word pair —
# `pane split|move`, `tab create`, `workspace create|close`, `agent start`.
# The scanner below tokenizes the command first: it resolves quotes and
# escapes into words ("herdr", h\e\rdr), strips path prefixes
# (/usr/local/bin/herdr, ./herdr), skips VAR= prefixes and shell keywords,
# recurses into command substitutions ($( ), ` `) and the payloads of
# bash|sh|dash|zsh -c, eval and prefix wrappers (sudo, env, command, exec,
# nice, time, nohup, setsid, stdbuf, xargs, builtin), and skips heredoc
# bodies. `herdr ...` inside an argument string (printf '%s\n' 'example:
# herdr pane split') is an argument, not an executable, and is allowed.
#
# This is a heuristic guardrail against direct calls, not a security
# boundary: indirection it cannot resolve ($BIN variables, sourced scripts,
# interpreter payloads other than the supported wrappers) still passes. If it
# ever has to be an authority boundary, enforce it in herdr's API/permission
# layer instead.
set -euo pipefail

cmd=$(jq -r '.tool_input.command // ""')
[ -n "$cmd" ] || exit 0

block() {
  echo "blocked: spawn/move panes and workspaces only via scripts/ (spawn-worker.sh, spawn-lead.sh, grid-workers.sh, cleanup-worker.sh, cleanup-lead.sh) — rules/_all/fanout-agent-grid.md" >&2
  exit 2
}

# ---------------------------------------------------------------------------
# Tokenizer. tok <text> appends one element per simple command to the global
# GW_SEGS array, words TAB-separated. Quotes and escapes are resolved into
# the word they belong to; $( ) and ` ` bodies are tokenized recursively
# (they run commands of their own); redirect targets and heredoc bodies are
# dropped. _gw_* helpers see tok's locals through bash dynamic scope.
# ---------------------------------------------------------------------------
declare -ga GW_SEGS=()

GW_SI='' GW_SE=0
_gw_subst() {  # _gw_subst: s[i] is '`' or '$' with s[i+1]='('.
               # Sets GW_SI = inner text, GW_SE = index just past the closer.
  local j inner='' c q='' depth=1
  if [[ ${s:i:1} == '`' ]]; then
    j=$((i + 1))
    while (( j < n )) && [[ ${s:j:1} != '`' ]]; do inner+=${s:j:1}; j=$((j+1)); done
    GW_SI=$inner; GW_SE=$j
    if (( GW_SE < n )); then GW_SE=$((GW_SE+1)); fi
    return
  fi
  j=$((i + 2))
  while (( j < n )); do
    c=${s:j:1}
    if [[ $q == "'" ]]; then
      if [[ $c == "'" ]]; then q=''; fi
      inner+=$c; j=$((j+1)); continue
    fi
    if [[ $q == '"' ]]; then
      case $c in
        '"') q='' ;;
        '\') inner+=$c; j=$((j+1)); if (( j < n )); then inner+=${s:j:1}; fi ;;
      esac
      if [[ $c != '\' ]]; then inner+=$c; fi
      j=$((j+1)); continue
    fi
    case $c in
      "'") q="'" ;;
      '"') q='"' ;;
      '\') inner+=$c; j=$((j+1)); if (( j < n )); then inner+=${s:j:1}; fi; j=$((j+1)); continue ;;
      '(') depth=$((depth+1)) ;;
      ')') depth=$((depth-1)); if (( depth == 0 )); then j=$((j+1)); break; fi ;;
    esac
    inner+=$c; j=$((j+1))
  done
  GW_SI=$inner; GW_SE=$j
}

_gw_fw() {  # emit the current word unless it is empty or a redirect target
  if (( have )); then
    if (( want_hdoc )); then
      want_hdoc=0
      # element = strip-tabs marker (N or T) + delimiter
      hdocs+=("$hdoc_strip$word")
    elif (( skip )); then
      skip=0
    else
      line+="${line:+$'\t'}$word"
    fi
  fi
  word=''; have=0
}

_gw_fs() { _gw_fw; if [[ -n $line ]]; then GW_SEGS+=("$line"); fi; line=''; skip=0; want_hdoc=0; }

_gw_redir() {  # s[i] is the first < or > of a redirect operator; consume the
               # operator and leave i on its last char. Sets skip (drop the
               # next word — the detached or attached target) or want_hdoc.
  local lts=0
  if [[ ${s:i:1} == '<' ]]; then lts=1; fi   # s[i] itself is the first op char
  while (( i + 1 < n )) && [[ ${s:i+1:1} == '>' || ${s:i+1:1} == '<' ]]; do
    if [[ ${s:i+1:1} == '<' ]]; then lts=$((lts+1)); fi
    i=$((i+1))
  done
  if [[ ${s:i+1:1} == '&' ]]; then
    # >&fd / <&fd — the target is attached to the operator; nothing to skip
    i=$((i+1))
    while (( i + 1 < n )) && [[ ${s:i+1:1} =~ [0-9a-zA-Z_.~-] ]]; do i=$((i+1)); done
    return
  fi
  if (( lts == 2 )); then
    # <<DELIM / <<-DELIM heredoc: the next word is the delimiter, the body
    # (skipped at the next newline) is literal text, not commands
    want_hdoc=1
    hdoc_strip=N
    if [[ ${s:i+1:1} == '-' ]]; then i=$((i+1)); hdoc_strip=T; fi
    return
  fi
  skip=1
}

tok() {  # tok <text>
  local s=$1 i=0 c word='' line='' have=0 quote='' skip=0
  local n=${#s}
  local want_hdoc=0 hdoc_strip=N inner
  local -a hdocs=()
  while (( i < n )); do
    c=${s:i:1}
    if [[ $quote == "'" ]]; then
      if [[ $c == "'" ]]; then quote=''; else word+=$c; fi
      i=$((i+1)); continue
    fi
    if [[ $quote == '"' ]]; then
      case $c in
        '"') quote='' ;;
        '\')
          i=$((i+1))
          if (( i < n )) && [[ ${s:i:1} != $'\n' ]]; then word+=${s:i:1}; fi ;;
        '`')
          _gw_subst; inner=$GW_SI; i=$GW_SE
          if [[ -n $inner ]]; then tok "$inner"; fi
          continue ;;
        '$')
          if [[ ${s:i+1:1} == '(' ]]; then
            _gw_subst; inner=$GW_SI; i=$GW_SE
            if [[ -n $inner ]]; then tok "$inner"; fi
            continue
          fi
          word+=$c ;;
        *) word+=$c ;;
      esac
      i=$((i+1)); continue
    fi
    case $c in
      "'") quote="'"; have=1 ;;
      '"') quote='"'; have=1 ;;
      '\')
        i=$((i+1))
        if (( i < n )) && [[ ${s:i:1} != $'\n' ]]; then word+=${s:i:1}; have=1; fi ;;
      '`')
        _gw_subst; inner=$GW_SI; i=$GW_SE
        if [[ -n $inner ]]; then tok "$inner"; fi
        have=1; continue ;;
      '$')
        if [[ ${s:i+1:1} == '(' ]]; then
          _gw_subst; inner=$GW_SI; i=$GW_SE
          if [[ -n $inner ]]; then tok "$inner"; fi
          have=1; continue
        fi
        word+=$c; have=1 ;;
      ';'|'|'|'('|')'|$'\n')
        _gw_fs
        if [[ $c == $'\n' ]] && (( ${#hdocs[@]} )); then
          # heredoc bodies: literal lines until each delimiter line
          local ln lns d strip
          while (( ${#hdocs[@]} )) && (( i + 1 < n )); do
            d=${hdocs[0]}; strip=''
            if [[ ${d:0:1} == T ]]; then strip=T; fi
            d=${d:1}
            hdocs=("${hdocs[@]:1}")
            while (( i + 1 < n )); do
              i=$((i+1)); ln=''
              while (( i < n )) && [[ ${s:i:1} != $'\n' ]]; do ln+=${s:i:1}; i=$((i+1)); done
              lns=$ln
              if [[ -n $strip ]]; then lns=${ln#"${ln%%[!$'\t']*}"}; fi
              if [[ $lns == "$d" ]]; then break; fi
            done
          done
          hdocs=()
        fi ;;
      '&')
        if [[ ${s:i+1:1} == '>' ]]; then
          _gw_fw; i=$((i+1)); _gw_redir
        else _gw_fs; fi ;;
      '>'|'<')
        # a bare fd number before the operator belongs to it (2>file)
        if (( have )) && [[ $word =~ ^[0-9]+$ ]]; then word=''; have=0; else _gw_fw; fi
        _gw_redir ;;
      '#')
        if (( ! have )); then
          while (( i < n )) && [[ ${s:i:1} != $'\n' ]]; do i=$((i+1)); done
          continue
        fi
        word+=$c ;;
      ' '|$'\t') _gw_fw ;;
      *) word+=$c; have=1 ;;
    esac
    i=$((i+1))
  done
  _gw_fs
}

# ---------------------------------------------------------------------------
# Checker. The executable is the first word of a segment that is not a VAR=
# assignment or a compound-shell keyword. A herdr executable with a guarded
# consecutive argument pair is blocked; supported wrappers are unwrapped and
# rechecked.
# ---------------------------------------------------------------------------
_gw_keyword() {
  case $1 in
    if|then|else|elif|fi|for|in|while|until|do|done|case|esac|select|function|'{'|'}'|'!'|coproc) return 0 ;;
    # note: 'time' is deliberately NOT here — it is a prefix wrapper handled
    # below, so 'time -p herdr pane split' still resolves herdr
    *) return 1 ;;
  esac
}

check_text() {  # check_text <shell-source>
  GW_SEGS=(); tok "$1"
  # snapshot: check_words may recurse into check_text for -c/eval payloads,
  # which resets GW_SEGS
  local -a segs=("${GW_SEGS[@]}") sw=()
  local seg si
  for (( si = 0; si < ${#segs[@]}; si++ )); do
    seg=${segs[si]}
    IFS=$'\t' read -r -a sw <<<"$seg"
    if (( ${#sw[@]} )); then check_words "${sw[@]}"; fi
  done
}

check_words() {  # check_words <word>...
  local -a w=("$@")
  local i=0 k
  while (( i < ${#w[@]} )); do
    if _gw_keyword "${w[i]}" || [[ ${w[i]} =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then
      i=$((i+1)); continue
    fi
    break
  done
  if (( i >= ${#w[@]} )); then return 0; fi
  local base=${w[i]##*/}   # executable word: quotes stripped, path removed
  local -a a=()
  if (( i + 1 < ${#w[@]} )); then a=("${w[@]:i+1}"); fi
  case $base in
    herdr)
      for (( k = 0; k + 1 < ${#a[@]}; k++ )); do
        case "${a[k]} ${a[k+1]}" in
          'pane split'|'pane move'|'tab create'|'workspace create'|'workspace close'|'agent start') block ;;
        esac
      done ;;
    bash|sh|dash|zsh)
      for (( k = 0; k < ${#a[@]}; k++ )); do
        if [[ ${a[k]} =~ ^-[a-z]*c$ ]] && (( k + 1 < ${#a[@]} )); then
          check_text "${a[k+1]}"   # the -c payload; trailing args are $0/$1
          return 0
        fi
      done ;;
    eval)
      if (( ${#a[@]} )); then check_text "${a[*]}"; fi ;;
    sudo|env|command|exec|nice|time|nohup|setsid|stdbuf|xargs|builtin|chrt|ionice|taskset)
      # prefix wrapper: the real command follows its options and VAR=
      # prefixes. Value-taking flags differ per wrapper; for the rest a '-*'
      # word is only dropped, never allowed to swallow the next word.
      local vf=' ' k
      case $base in
        sudo) vf=' u g h p C T U R t D ' ;;
        env)  vf=' u C S ' ;;
        nice) vf=' n ' ;;
      esac
      local -a rest=()
      for (( k = 0; k < ${#a[@]}; k++ )); do
        if [[ ${a[k]} =~ ^-([a-zA-Z])$ ]] && [[ $vf == *" ${BASH_REMATCH[1]} "* ]]; then
          k=$((k+1))   # value-taking flag: consume its value too
        elif [[ ${a[k]} == -* ]]; then
          :            # boolean flags
        elif ! [[ ${a[k]} =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then
          rest=("${a[@]:k}"); break
        fi
      done
      if (( ${#rest[@]} )); then check_words "${rest[@]}"; fi ;;
  esac
  return 0
}

check_text "$cmd"
exit 0
