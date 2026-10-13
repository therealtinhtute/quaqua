# Task-contract helpers (rules/_all/brief.md), sourced by
# scripts/worker/prompt-worker.sh and scripts/lead/pipeline.sh.
# A contract brief = the 3 core headers + 'Owned scope:' + at least one
# acceptance line '- A<n>: `<command>` → <expected>'.

# brief_contract_gaps <text> — one missing field per line; empty = a contract
brief_contract_gaps() {
  local b=$1
  grep -qF 'Goal:' <<<"$b" || echo 'Goal:'
  grep -qF 'Hard constraints:' <<<"$b" || echo 'Hard constraints:'
  grep -qE 'Current design(\s*\(open to challenge\))?:' <<<"$b" \
    || echo 'Current design (open to challenge):'
  grep -qE '^Owned scope:[[:space:]]*[^[:space:]]' <<<"$b" || echo 'Owned scope:'
  grep -qE '^[[:space:]]*-[[:space:]]*A[0-9]+:[[:space:]]*`[^`]+`' <<<"$b" \
    || echo 'Acceptance: - A1: `<command>` → <expected>'
  return 0
}

# brief_acceptance <text> — 'A<n><TAB><command>' per acceptance line
brief_acceptance() {
  sed -nE 's/^[[:space:]]*-[[:space:]]*(A[0-9]+):[[:space:]]*`([^`]+)`.*/\1\t\2/p' <<<"$1"
}

# brief_scope <text> — the Owned scope patterns, one per line (the part
# before ' · Read-only:'; comma or space separated)
brief_scope() {
  sed -nE 's/^Owned scope:[[:space:]]*//p' <<<"$1" | head -1 \
    | sed -E 's/[[:space:]]*·[[:space:]]*Read-only:.*//' \
    | tr ', ' '\n\n' | sed '/^$/d'
}

# path_in_scope <path> <pattern...> — a pattern is a glob ('src/*.rs'), a
# directory ('src/' or 'src'), or an exact file
path_in_scope() {
  local p=$1 s; shift
  for s in "$@"; do
    # shellcheck disable=SC2053  # glob match on purpose
    [[ $p == $s || $p == "${s%/}"/* ]] && return 0
  done
  return 1
}
