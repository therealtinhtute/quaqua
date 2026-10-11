# Fanout agent grid — MANDATORY

Every spawn/fanout of task workers MUST end with the workers arranged in the agent grid before any `herdr agent prompt`:
- `scripts/spawn-worker.sh` / `scripts/cleanup-worker.sh` run the grid themselves; run `scripts/grid-workers.sh --tab <repo>-workers` by hand only to repair a layout.
- 1 worker → split beside its orchestrator. 2+ workers → a new tab `<repo>-workers` (e.g. `monotinhtute-workers`) in a grid (2 = left|right, 3 = A|B + C below, 4 = 2x2); the orchestrator tab keeps only the orchestrator.
- One project per workers tab; never mix projects or put 2+ workers in the CoS tab.
- `GRID_SKIP=1` is for `pipeline.sh` (grids its own workers) and tests (`QUAQUA_TEST=1`) only; anywhere else it is ignored with a warning — never use it to dodge the grid.
- Label each pane with the worker name. Never move a pane while its agent is working or during `--wait`.

Why: anh iu, 2026-10-02 — workers left scattered had to be moved by hand (tieude-pillfix + tieude-ccresearch → w4:t3 "coucou-workers"). 2026-10-11 — CoS spawned probes with `GRID_SKIP=1`, piling 3 panes into the CoS tab.
