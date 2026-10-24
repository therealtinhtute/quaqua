# Farm layouts — plan set

**Goal.** One CoS for the whole workspace drives N teams (one `<slug>-lead` per slug), each team on
its own layout (slp, lead-peer, advisor, dual-lens, council, user-defined), with
`can_spawn` / `can_prompt` / depth enforced at the verbs quaqua mediates.

**Status: ready.** `quaqua-tidy` done and PR #113 merged (2026-10-23); P0 brief for a fresh
session: [`p0-kickoff.md`](p0-kickoff.md). Phases P0–P8 ship independently, in order: P0 (identity + depth) → P1
(`layout/` model, read-only `show`/`why`) → P2 (bus foundation) → P3 (gate in shadow) → P4 (gate
enforced) → P5 (mail) → P6 (consumers) → P7 (freeze + `config migrate`) → P8 (user layouts,
`council`). Model: 1 CoS per workspace, not per project; anh iu talks only to the CoS.

## Decisions (anh iu, 2026-10-23)

- **D1 — freeze topology only**, per slug: `runs/<slug>.layout.toml`. Kinds and models stay live, so
  the "Claude out" profile keeps working.
- **D3 — CoS identity is a positive pane token**, set by `quaqua cos tag`. No token → `Unknown`,
  never `cos` (the gate fails closed rather than guessing).
- **D4 — mail and `farm_events` live in `runs/quaqua.db`**, in their own tables, not in the engine's
  `events` table (that one is the replay journal).

D2 and D5–D9 take the design's recommendation unless anh iu says otherwise at kickoff.

## Reading order

1. [`plan.md`](plan.md) — decisions, goal, and the P0 addendum (CoS marker + protection, tried by hand on herdr 0.9.5).
2. [`design.md`](design.md) — the design: §A layout manifest, §B runtime gate, §C mail, §D events, sequences, P0–P8 rollout, D1–D9.
3. [`research-piggery.md`](research-piggery.md) — what piggery does and what quaqua should and should not borrow (read this second if you question the design).
4. [`event-monitor.md`](event-monitor.md) → [`metrics.md`](metrics.md) — the two earlier plans folded into this set; read them for the history of P2/P6, not as live plans.

## Files

| File | One line |
| --- | --- |
| [`roadmap.md`](roadmap.md) | Delivery roadmap: token key decision, lanes, Leads, checkpoints, per-phase gate. |
| [`p0-kickoff.md`](p0-kickoff.md) | P0 brief for a fresh session: identity + depth, `cos tag`, CoS pane protection, evidence, budgets. |
| [`plan.md`](plan.md) | The plan: goal, D1/D3/D4, the P0 CoS-marker addendum (`quaqua cos tag`, sidebar chip, no silent herdr config edits). |
| [`design.md`](design.md) | The design in full: `[layout]` in `team.toml` v3, one pure `layout::check` at every verb, `mail`/`farm_events` tables in `runs/quaqua.db`, per-verb events, P0–P8 with one proving test each. |
| [`research-piggery.md`](research-piggery.md) | piggery vs quaqua source comparison: manifest schema, gate, mail, events; ranked borrow list, skips with reasons, migration path. Carries a CoS correction on the worker cap. |
| [`event-monitor.md`](event-monitor.md) | Earlier plan (2026-10-17, issue #47): event-driven `quaqua watch` + `quaqua status` with alert flags. Absorbed by design §D / P2+P6. |
| [`metrics.md`](metrics.md) | Earlier plan (2026-10-21, #47 P3/#48): one cost/time row per worker and the offline spend readers per harness. Its row now lands in `farm_events`. |
