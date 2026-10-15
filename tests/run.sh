#!/usr/bin/env bash
# Test runner: executes every tests/test_*.sh sequentially, prints a
# PASS/FAIL count, and exits non-zero if any test failed.
# Requires Bash >= 4.4 (macOS system Bash 3.2 is insufficient), git, jq,
# Perl, python3, ripgrep, and a Rust toolchain (cargo/rustc — the crate's tests
# run below; a cold cargo registry cache needs network on the first run). Each
# test builds its own sandbox (temp HOME, stubs on PATH); no network, real
# agents, or services are touched by the sandboxed tests.
# Usage: tests/run.sh   (exit 0 = all pass)
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
T=$(mktemp -d "${TMPDIR:-/tmp}/quaqua-tests.XXXXXX")
trap 'rm -rf "$T"' EXIT

pass=0 fail=0 failed=()

# Rust crate gate: the `quaqua` binary takes over from the shell engine, so its
# crate tests are part of this runner. Booked under cargo_test so the failure
# tail below picks up "$T/cargo_test.log" like any other case. --locked keeps the
# committed Cargo.lock authoritative: a manifest/lock drift fails here instead of
# being silently repaired by cargo.
if (cd "$here/.." && cargo test --locked) >"$T/cargo_test.log" 2>&1; then
  printf 'PASS %s\n' cargo_test
  pass=$((pass + 1))
else
  printf 'FAIL %s\n' cargo_test
  fail=$((fail + 1))
  failed+=(cargo_test)
fi

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
