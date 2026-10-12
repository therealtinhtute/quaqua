# Coucou Island M13 — Plan (2026-10-07)

Lock: `spec-raw/coucou-island-m13-lock.md`. Repo: `~/Lab/coucou-island`, base `main` @b648077.
Each phase = one sude task worker (`--base main`), own branch, merge needs anh iu's approval. Each phase leaves main shippable.
Gate for every phase: `make test` all green (≥636 + new), `make pack` ok, `COUCOU_NESTED_DRY=1 scripts/coucou-nested.sh` ok.

## M13a — Agent off (decision 8) · smallest, ships alone
1. Schema: `watch-agents`, `agent-approvals` default `false` → verify: test reads defaults from compiled schema.
2. Prefs: hide Agent section → verify: prefs test/grep shows no Agent group built.
3. Card/docs note: `gsettings reset` both keys on owner machine.

## M13b — Island reducer: size classes + priority (decisions 4, 5) · pure, headless
1. States `idle | compact | minimal | expanded` in `src/island/reducer.js` → verify: unit tests per transition.
2. Priority: newest peek holds main pill for `auto-close-seconds`, then back to playing media → verify: tests with fake clock.
3. Minimal: ≥2 sources → main + max 2 secondary entries carrying state → verify: tests with 1/2/3/4 sources.
No view changes; existing view maps new states to current rendering.

## M13c — Idle clock replacing dateMenu label (decisions 1, 2, 3, 7-idle)
1. Idle pill renders `HH:MM`, honors `clock-format`, updates on minute boundary → verify: unit test of formatter + timer scheduling.
2. Lifecycle: hide dateMenu clock label on enable, restore on disable → verify: lifecycle test with fake panel (enable/disable/enable leaves label identical).
3. Idle click → open dateMenu; `idle-hide-minutes` ignored while clock mode → verify: tests.

## M13d — View: minimal dots + hover-delay expanded list + active click (decisions 4, 6, 7)
1. Render minimal dots (color/progress ring) from M13b state.
2. New key `expand-hover-delay-ms` (default 400); enter → open after delay, leave → close after 300 ms → verify: tests with fake timers (no flicker on quick in/out).
3. Expanded list ordered by priority, max N rows + scroll.
4. Active click → activate source app (MPRIS `Raise`) → verify: fixture test.

## M13e — Owner nested pass (anh iu only, no worker GUI)
Checklist in lock "Done when". On failure: CoS reads `~/coucou-nested.log`, opens a fix task.

## Order
a → b → c → d → e. a and b are independent (can run in parallel, 2 workers); c needs b's idle state; d needs b.

## Revised fanout (2026-10-07, approved) — supersedes Order
4 parallel workers from `main`, then 1 wiring worker:
- m13a: as above.
- m13b: as above (reducer only).
- m13c: standalone clock — formatter + minute timer, hide/restore dateMenu label in lifecycle, idle click → dateMenu. NOT wired to reducer states.
- m13d: standalone hover-delay (`expand-hover-delay-ms`, open 400 / close 300) + active click → MPRIS `Raise`. NOT wired to reducer states.
- m13wire (after a–d merged): wire clock/hover/minimal dots/expanded list to M13b states; `idle-hide-minutes` ignored in clock mode.
- m13e: owner nested pass.
