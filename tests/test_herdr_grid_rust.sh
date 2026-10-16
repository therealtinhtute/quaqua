#!/usr/bin/env bash
# tests/test_herdr_grid_rust.sh — verification for quaqua grid {plan,reconcile,hook}.
# Tests differential equality between `quaqua grid plan` and `plan.jq` across all
# fixtures in tests/fixtures/herdr-grid/*.json (including the 9 edge cases),
# plus --only + custom --tab scoping (dec5a30 bugfix) and 2-worker landing in GRID_TAB.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$here/.." && pwd)"
fixdir="$repo_root/tests/fixtures/herdr-grid"
plan_jq="$repo_root/plugins/herdr-grid/plan.jq"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# Locate or build quaqua binary
QUAQUA_BIN="${QUAQUA_BIN:-}"
if [ -z "$QUAQUA_BIN" ]; then
  (cd "$repo_root" && cargo build --locked --release --quiet)
  QUAQUA_BIN="$repo_root/target/release/quaqua"
fi

[[ -x "$QUAQUA_BIN" ]] || fail "cannot find executable quaqua binary at $QUAQUA_BIN"
echo "Using binary: $QUAQUA_BIN"

# ── 1. Differential test: quaqua grid plan vs plan.jq across all fixtures ────
echo "--- 1. Differential test against plan.jq ---"
if ! command -v jq >/dev/null 2>&1; then
  echo "SKIP: jq not found on PATH; skipping differential comparison against plan.jq"
else
  total=0
  passed=0

  for fix in "$fixdir"/*.json; do
    [ -f "$fix" ] || continue
    name="$(basename "$fix")"
    total=$((total + 1))

    # Run plan.jq
    jq_err=0
    jq_out=$(jq -c -f "$plan_jq" "$fix" 2>&1) || jq_err=$?

    # Run quaqua grid plan
    rust_err=0
    rust_out=$("$QUAQUA_BIN" grid plan "$fix" 2>&1) || rust_err=$?

    if [ "$jq_err" -ne 0 ]; then
      # Expect rust to fail as well (e.g. 5-workers.json >4 workers rejected)
      if [ "$rust_err" -ne 0 ]; then
        if [[ "$jq_out" == *"too many workers in group"* && "$rust_out" == *"too many workers in group"* ]]; then
          echo "ok: $name (both rejected >4 workers)"
          passed=$((passed + 1))
        else
          fail "fixture $name: error message mismatch:\n  jq:   $jq_out\n  rust: $rust_out"
        fi
      else
        fail "fixture $name: plan.jq failed (rc=$jq_err) but quaqua grid plan succeeded:\n  $rust_out"
      fi
    else
      if [ "$rust_err" -ne 0 ]; then
        fail "fixture $name: plan.jq succeeded but quaqua grid plan failed (rc=$rust_err):\n  $rust_out"
      fi

      # Normalize both outputs with jq for comparison
      norm_jq=$(printf '%s' "$jq_out" | jq -c -S .)
      norm_rust=$(printf '%s' "$rust_out" | jq -c -S .)

      if [ "$norm_jq" = "$norm_rust" ]; then
        echo "ok: $name ($norm_rust)"
        passed=$((passed + 1))
      else
        fail "fixture $name: output mismatch!\n  want (jq):   $norm_jq\n  got (rust):  $norm_rust"
      fi
    fi
  done

  echo "Differential result: $passed/$total fixtures passed"
  [ "$passed" -eq "$total" ] || fail "some fixtures failed differential comparison"
  [ "$total" -ge 23 ] || fail "expected at least 23 fixtures, found $total"
fi

# ── 2. dec5a30 bugfix test: --only + custom --tab ────────────────────────────
echo "--- 2. dec5a30 bugfix test: --only + custom --tab ---"
# only-custom-tab.json has workers with grid_group="quaqua".
# When --tab mono-workers is specified, ops whose group is not mono-workers
# are ignored, so remaining ops is empty and it converges immediately (exit 0).
out=""
rc=0
out=$(HERDR_STUB_SNAPSHOT="$fixdir/only-custom-tab.json" QUAQUA_OWNER=cos "$QUAQUA_BIN" grid reconcile \
  --check \
  --tab mono-workers \
  --only sude-isl,quansu-islr1 \
  --dry-run \
  --team-file "$repo_root/team.toml" 2>&1) || rc=$?
if [ "$rc" -ne 0 ]; then
  fail "--check --only with custom --tab mono-workers expected exit 0, got $rc: $out"
fi
[[ "$out" == *"[]"* ]] || fail "expected empty ops '[]' for foreign tab mono-workers, got: $out"
echo "ok: --check --only with custom --tab mono-workers converges immediately (exit 0)"

# Conversely, when --tab quaqua-workers matches the worker group, it reports pending ops
err=""
if err=$(HERDR_STUB_SNAPSHOT="$fixdir/only-custom-tab.json" QUAQUA_OWNER=cos "$QUAQUA_BIN" grid reconcile \
  --check \
  --tab quaqua-workers \
  --only sude-isl,quansu-islr1 \
  --dry-run \
  --team-file "$repo_root/team.toml" 2>&1); then
  fail "--check --only with matching --tab quaqua-workers expected error (pending ops), got success: $err"
fi
[[ "$err" == *"pending ops"* ]] || fail "expected pending ops error, got: $err"
echo "ok: --check --only with matching --tab quaqua-workers reports pending ops"

# ── 3. 2-worker set lands in GRID_TAB, never orchestrator tab ────────────────
echo "--- 3. 2-worker set lands in GRID_TAB, never orchestrator tab ---"
ops=$("$QUAQUA_BIN" grid plan "$fixdir/2-workers.json")
# Op 0 target must be new_tab with label quaqua-workers (GRID_TAB)
op0_kind=$(printf '%s' "$ops" | jq -r '.[0].target.kind')
op0_label=$(printf '%s' "$ops" | jq -r '.[0].target.label')
# Op 1 target must be split on @workers
op1_kind=$(printf '%s' "$ops" | jq -r '.[1].target.kind')
op1_tab=$(printf '%s' "$ops" | jq -r '.[1].target.tab')

[[ "$op0_kind" == "new_tab" ]] || fail "op 0 kind expected new_tab, got $op0_kind"
[[ "$op0_label" == "quaqua-workers" ]] || fail "op 0 label expected quaqua-workers, got $op0_label"
[[ "$op1_kind" == "split" ]] || fail "op 1 kind expected split, got $op1_kind"
[[ "$op1_tab" == "@workers" ]] || fail "op 1 tab expected @workers, got $op1_tab"

# Assert neither op targets the orchestrator's tab (w1:t1)
has_w1_t1=$(printf '%s' "$ops" | jq '[.[] | select(.target.tab == "w1:t1")] | length')
[[ "$has_w1_t1" -eq 0 ]] || fail "2-worker set must never target orchestrator tab w1:t1"

echo "ok: 2-worker set lands in GRID_TAB (quaqua-workers), never orchestrator tab"

# ── 4. Verify fake driver in Rust tests ──────────────────────────────────────
echo "--- 4. Verify fake driver in Rust tests ---"
(cd "$repo_root" && cargo test --locked --test grid -- test_fake_herdr_driver_applies_ops_cleanly --quiet)
echo "ok: fake herdr driver applied ops without real herdr calls"

# ── 5. Reject --check and --repair together ─────────────────────────────────
echo "--- 5. Reject --check and --repair together ---"
err=""
if err=$("$QUAQUA_BIN" grid reconcile --check --repair 2>&1); then
  fail "expected failure when both --check and --repair are passed, got success: $err"
fi
[[ "$err" == *"cannot specify both --check and --repair"* ]] || fail "wrong error message for --check --repair: $err"
echo "ok: --check and --repair together rejected with clear error"

echo "PASS: all test_herdr_grid_rust.sh cases passed"
