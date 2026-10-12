# Fanout agent grid — MANDATORY

Every spawn/fanout of task workers MUST end with the workers arranged in the agent grid before any `herdr agent prompt`:
- `scripts/worker/spawn-worker.sh` / `scripts/worker/cleanup-worker.sh` run the grid themselves; run `scripts/worker/grid-workers.sh --tab <repo>-workers` by hand only to repair a layout.
- The harness decides placement, not the agent: `spawn-worker.sh` records `runs/<name>.owner` (`QUAQUA_OWNER`) and `runs/<name>.grid` (`GRID_TAB`, default `<repo>-workers`); the grid only moves the caller's own workers, grouped by project.
- An orchestrator with exactly 1 worker in total → split beside it. Otherwise each project gets its own `<repo>-workers` tab (e.g. `monotinhtute-workers`) in a grid (1 = full, 2 = left|right, 3 = A|B + C below, 4 = 2x2); the orchestrator tab keeps only the orchestrator.
- Each SLP team gets its own herdr workspace `<slug>` (`spawn-lead.sh`): the Lead and its Peers' tabs live there; the CoS (the one Supervisor) stays in its own workspace and reaches Leads by agent name. `cleanup-lead.sh <slug>` closes it when the project is done.
- Enforced: `.claude/settings.json` runs `scripts/hooks/herdr-gate.sh`, which blocks a direct `herdr pane split|move`, `herdr tab create`, `herdr workspace create|close` or `herdr agent start` from the CoS/Leads.
- One project per workers tab; never mix projects or owners, or put 2+ workers in the CoS tab.
- `GRID_SKIP=1` is for `pipeline.sh` (grids its own workers) and tests (`QUAQUA_TEST=1`) only; anywhere else it is ignored with a warning — never use it to dodge the grid.
- Label each pane with the worker name. Never move a pane while its agent is working or during `--wait`.

Why: 2026-10-11 — the Lead split into the CoS tab put its Peer there too (3 panes in one tab), and a CoS worker was stranded alone in a project tab. anh iu, 2026-10-02 — workers left scattered had to be moved by hand (tieude-pillfix + tieude-ccresearch → w4:t3 "coucou-workers"). 2026-10-11 — CoS spawned probes with `GRID_SKIP=1`, piling 3 panes into the CoS tab.
