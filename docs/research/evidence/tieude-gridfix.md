# tieude-gridfix — make `grid reconcile` converge right after a spawn

Branch: `quaqua/tieude-gridfix` (from `quaqua/quaqua-rs-integ`), commit `096b30a`.
No push, no merge.

## Root causes (two cases, both reproduced first)

**Case 1 — single worker already beside the anchor timed out.**
`plan`/`plan.jq` (the `--check` path) decided "1 worker is in place" by comparing
the anchor's and worker's rectangles against the 2/5|3/5 cells (ratio 0.4). The
repair path (`reconcile.rs` `beside`) instead accepts the worker as soon as it
shares the anchor's tab. A herdr `pane split` (what `worker spawn` uses, no ratio)
lands **50/50**, so the check kept emitting the `ratio 0.4` op that the repair
path would never perform → `grid check timed out after 60s`.
Reproduced from a live capture (`herdr api snapshot`, saved `/tmp/realsnap.json`,
2026-10-16): `w9:t1A` area 206x78, `w9:p10` rect 103 wide, `w9:p32` rect 103 wide
(50/50) — `plan` emitted
`{"pane":"w9:p32","target":{"kind":"split","tab":"w9:t1A","anchor":"w9:p10","ratio":0.4}}`,
and with the worker idle `--check` exited 1 with the exact observed error.

**Case 2 — a split that depends on a deferred anchor counted as pending.**
An op's `deferred` flag was taken only from *its own* pane's status. When worker 0
is working, its `new_tab` op is deferred, but the dependent split anchored on it
(and any split into `@workers`) stayed `deferred:false`. `filter_ops_for_check`
drops deferred ops only, so the pending dependent failed the check while its
anchor was still working. `apply_ops` and the hook already propagate deferral —
the planner did not. Reproduced: `plan` of
`deferred-anchor-dependent-split` gave
`p5 new_tab deferred=true` + `p6 split @workers anchor p5 deferred=false`, and
`--check --only sude-1` exited 1 with `pending ops`.

## Fix

Single source of truth: the planner now matches the runtime.

- `crates/quaqua/src/grid/plan.rs`
  - single-worker branch: `in_place = anchor_tab.is_some() && w.tab_id == anchor_tab`
    (was `rect_near` on 2/5|3/5). Identical to `reconcile.rs` `beside`, so check and
    repair can no longer disagree; a `pane split` is never re-split.
  - new `propagate_deferrals(&mut ops)` at the end of `plan_with_deferred`: a split
    is deferred if its anchor is deferred, or if it targets `@workers` while the
    group's new tab is deferred — the same rule as `apply_ops`/`reconcile_step`.
- `plugins/herdr-grid/plan.jq` (differential oracle): mirrored both changes
  (`propagate_deferrals` def + `( … ) | propagate_deferrals`; same-tab `in_place`).
- `tests/test_herdr_grid_plan.sh` test 10 updated: working-deferred now marks the
  dependents deferred too, with the idle case (`1-worker.json`) kept as the
  "idle = not working" check.

## New fixtures + tests

- `tests/fixtures/herdr-grid/single-worker-in-anchor-tab.json` — real 206x78
  103|103 capture (worker set idle, the pre-prompt state).
- `tests/fixtures/herdr-grid/deferred-anchor-dependent-split.json` — working
  anchor + idle dependent, no workers tab.
- `crates/quaqua/tests/grid.rs`: `test_reconcile_converges_when_single_worker_already_beside_anchor`
  and `test_check_converges_when_dependent_split_follows_deferred_anchor`
  (both fail on the old code: non-empty ops / non-deferred op → timeout).
- New fixtures also flow through the differential test vs `plan.jq` (25/25).

## Evidence

- `cargo test --locked` → **84 passed, 0 failed** (targets: 6, 1, 11, 8, 35, 5, 18;
  `--test grid` = 35 passed, includes the differential-vs-`plan.jq` over 25 fixtures).
- `cargo build --locked --release` → `Finished release profile`.
- `bash tests/test_herdr_grid_plan.sh` → `ALL TESTS PASSED` (12 cases).
- `bash tests/test_herdr_grid_rust.sh` → `Differential result: 25/25 fixtures passed`,
  `PASS: all test_herdr_grid_rust.sh cases passed`.
- Cases 1 & 2 before/after via CLI: before `rc=1` (timeout, same op as reported),
  after `rc=0`.
- Live end-to-end, read-only, on the real session that used to time out:
  `cd ~/quaqua && GRID_CHECK_TIMEOUT_SEC=15 quaqua grid reconcile --check --tab quaqua-workers --only tieude-gridfix`
  → `rc=0` in 0.006s (was a 60s timeout).
- Existing grid fixtures + 9 edge fixtures: all pass; `tests/test_grid_workers.sh`
  still correctly SKIPs (shim replaced it).

## Risks / scope

- A 1-worker pane is no longer reshaped to 40/60. It was never reshaped by the
  repair path either, so this only stops `plan`/`plan.jq` from demanding a move
  that never happened; live layout is unchanged.
- Case-2 fix is status-driven through the same rule the applier uses, so the
  check never waits on a working peer (matches the design note).

## Lessons

- A "converged?" check and the repair it guards must share one acceptance
  predicate; a stricter check times out forever on a layout the repair accepts —
  2026-10-16, runs/tieude-gridfix.md
- Deferral is a property of the plan, not just of each pane: dependents of a
  deferred anchor must inherit it, or checks count unrunnable ops as pending —
  2026-10-16, runs/tieude-gridfix.md
