#!/usr/bin/env bash
# R1 gate: `quaqua config validate` / `get` on the repo-root team.toml, every
# invalid fixture rejected with its own error, and the plan's Rule -> key table
# covers every source rule (33 rows, one per rule).
# Targeted: builds the crate, runs cases against the real binary; nothing is
# written into the repo. Usage: tests/test_config.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root=$(cd "$here/.." && pwd)
fail() { echo "FAIL: $*" >&2; exit 1; }

# Build once (--locked keeps Cargo.lock authoritative, same as tests/run.sh).
(cd "$root" && cargo build --locked --quiet) || fail "cargo build failed"
bin="$root/target/debug/quaqua"

# ── valid config + get ───────────────────────────────────────────────────────
[[ "$("$bin" config validate --file "$root/team.toml")" == "ok" ]] \
  || fail "repo team.toml did not validate"
echo "ok: repo team.toml validates"

rev=$("$bin" config get strategy.default_reviewer --file "$root/team.toml")
[[ "$rev" == $'quansu\nsuhuynh' ]] || fail "default_reviewer = '$rev'"
echo "ok: config get strategy.default_reviewer = quansu,suhuynh"

cap=$("$bin" config get limits.max_workers_per_orchestrator --file "$root/team.toml")
[[ "$cap" == "4" ]] || fail "cap = '$cap'"
zcap=$("$bin" config get limits.max_workers_per_orchestrator --file "$root/team.toml" --project zimage)
[[ "$zcap" == "3" ]] || fail "zimage cap = '$zcap'"
echo "ok: cap 4, zimage override 3"

[[ "$("$bin" config get kinds.quansu.can_review --file "$root/team.toml")" == "true" ]] \
  || fail "quansu.can_review"
[[ "$("$bin" config get kinds.tieude.can_review --file "$root/team.toml")" == "false" ]] \
  || fail "tieude.can_review"
echo "ok: kinds.<k>.can_review"

# ── invalid fixtures: rejected, each with its own message ─────────────────────
invalid="$root/crates/quaqua/tests/fixtures/invalid"
check_reject() { # <fixture> <needle> [--models]
  local f=$1 needle=$2; shift 2
  local err rc=0
  err=$("$bin" config validate --file "$invalid/$f" "$@" 2>&1 >/dev/null) || rc=$?
  [[ $rc -ne 0 ]] || fail "$f: expected rejection, exited 0"
  [[ "$err" == *"$needle"* ]] || fail "$f: '$err' lacks '$needle'"
  printf 'ok: %-28s -> %s\n' "$f" "$err"
}

check_reject cap-over.toml                "over the cap"
check_reject bad-weights.toml             "exceeds cap"
check_reject project-cap-over.toml        "exceeds global cap"
check_reject unknown-kind.toml            "unknown kind 'ghost'"
check_reject unknown-class.toml           "unknown role-class 'ghostrole'"
check_reject unknown-role.toml            "unknown role 'ghost'"
check_reject same-harness.toml            "share harness"
check_reject reviewer-cannot-review.toml  "can_review = false"
check_reject model-drift.toml             "drift" --models "$root/models.md"
check_reject bad-schema.toml              "unsupported schema_version"
check_reject integ-branch.toml            "{slug}"
check_reject differ-policy.toml           "unknown differ policy"

# ── Rule -> key table covers every source rule ────────────────────────────────
plan="$root/docs/research/herdr-grid-rust-migration-plan.md"
rows=$(sed -n '/rule-key-table:start/,/rule-key-table:end/p' "$plan" | grep -c '^| ')
data=$(( rows - 1 ))   # drop the header row (the |---| separator is not "^| ")
[[ $data -eq 33 ]] || fail "rule->key table has $data data rows, expected 33"

count() { sed -n '/rule-key-table:start/,/rule-key-table:end/p' "$plan" | grep -c "^| $1"; }
[[ "$(count 'AGENTS\.md')" -eq 13 ]]       || fail "AGENTS.md rows != 13 (7 Rules + 6 Protocol)"
[[ "$(count '`worker-cap\.sh`')" -eq 4 ]]  || fail "worker-cap.sh rows != 4"
[[ "$(count '`worker-timebox\.md`')" -eq 9 ]] || fail "worker-timebox.md rows != 9"
[[ "$(count '`models\.md`')" -eq 7 ]]      || fail "models.md rows != 7"

# every data row names a key (backticked) or explicitly says prose, none dropped
unmapped=$(sed -n '/rule-key-table:start/,/rule-key-table:end/p' "$plan" \
  | grep '^| ' | tail -n +2 | grep -v '`' | grep -vc 'prose' || true)
[[ "$unmapped" -eq 0 ]] || fail "$unmapped rule->key row(s) map to neither a key nor prose"
echo "ok: rule->key table = 33 rows (AGENTS 13, cap 4, timebox 9, models 7), all mapped"

echo "PASS test_config.sh"
