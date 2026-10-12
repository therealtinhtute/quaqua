#!/usr/bin/env bash
# Test runner: executes every tests/test_*.sh sequentially, prints a
# PASS/FAIL count, and exits non-zero if any test failed.
# Requires Bash >= 4.4 (macOS system Bash 3.2 is insufficient), git, jq,
# Perl, python3, and ripgrep. Each test builds its own sandbox (temp HOME,
# stubs on PATH); no network, real agents, or services are touched.
# Usage: tests/run.sh   (exit 0 = all pass)
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
T=$(mktemp -d "${TMPDIR:-/tmp}/quaqua-tests.XXXXXX")
trap 'rm -rf "$T"' EXIT

pass=0 fail=0 failed=()

for t in "$here"/test_*.sh; do
  [[ -e $t ]] || { echo "no tests found in $here" >&2; exit 1; }
  name=$(basename "$t")
  if bash "$t" >"$T/$name.log" 2>&1; then
    printf 'PASS %s\n' "$name"
    pass=$((pass + 1))
  else
    printf 'FAIL %s\n' "$name"
    fail=$((fail + 1))
    failed+=("$name")
  fi
done

# failing tests: surface their output (the tail is where the assertion died)
for name in "${failed[@]}"; do
  printf '\n--- %s ---\n' "$name"
  tail -n 30 "$T/$name.log"
done

printf '\n%s passed, %s failed (of %s)\n' "$pass" "$fail" "$((pass + fail))"
(( fail == 0 ))
