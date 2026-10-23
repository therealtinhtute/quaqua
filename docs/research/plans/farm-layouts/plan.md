---
status: ready (tidy done, #113 merged 2026-10-23; P0 brief: p0-kickoff.md)
lane: high-risk
---
# Farm layouts — layout manifest, runtime gate, mail, events

Goal: one CoS for the whole workspace drives N teams (one `<slug>-lead` per slug), each team on its own layout (slp, lead-peer, advisor, dual-lens, council, user-defined), with `can_spawn`/`can_prompt`/depth enforced at the verbs quaqua mediates.

Related: [`design.md`](design.md) (phases P0–P8, open decisions D1–D9) · [`research-piggery.md`](research-piggery.md) (piggery evidence) · [`event-monitor.md`](event-monitor.md) (P2+P6 input) · [`metrics.md`](metrics.md) (`farm_events` input) · index [`README.md`](README.md)

## Decisions (anh iu, 2026-10-23)
- D1: freeze topology only per slug (`runs/<slug>.layout.toml`); kinds/models stay live.
- D3: CoS identity = positive pane token, set by `quaqua cos tag`; no token → `Unknown`, never `cos`.
- D4: mail + `farm_events` in `runs/quaqua.db` (own tables, not the engine `events` table).
- D2, D5–D9: take the design's recommendation unless anh iu says otherwise at kickoff.
- Model: 1 CoS per workspace (not per project); anh iu talks only to the CoS.

## P0 addendum — CoS marker + protection (anh iu, 2026-10-23)
Tried by hand on pane `w1:p1X`, works on herdr 0.9.5:
- `herdr pane report-metadata <pane> --source herdr-quaqua --token role=cos --display-agent "🍑 CoS Quả Quả"` (pane id first; `--opt=value` form is rejected).
- `herdr pane rename <pane> "🍑 CoS"`.
- Sidebar chip in `~/.config/herdr/config.toml` (backup `config.toml.bak-2026-10-23`), then `herdr server reload-config`:
  ```toml
  [ui.sidebar.agents]
  row_gap = 0
  rows = [
    [{ token = "$role", fg = "#888888", rules = [{ equals = "cos", fg = "#ff5fa2", bold = true }] }, "state_icon", "machine", "workspace", "tab"],
    ["agent"],
  ]
  ```
P0 must:
1. `quaqua cos tag` sets the token, label and display name idempotently; re-run on CoS session start (tokens may not survive a herdr restart).
2. Decide the token name: hand test used `role=cos` because `grid reconcile` reads `grid_role` — check what grid does with an unknown `grid_role=cos` before reusing that key (D3 said `grid_role=cos`).
3. Protection quaqua can enforce: `worker cleanup`, `grid reconcile` and the PreToolUse `gate/` refuse to close/move a pane carrying the CoS token. herdr has no pin/lock: a manual UI close cannot be blocked (upstream feature request if wanted); recovery = `claude --resume`.
4. Ship the sidebar snippet as an opt-in template, never edit the user's herdr config silently.
