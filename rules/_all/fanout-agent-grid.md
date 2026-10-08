# Fanout agent grid — MANDATORY

Every spawn/fanout of task workers MUST end with the workers arranged in the agent grid before any `herdr agent prompt`:
- Run `scripts/grid-workers.sh` right after `scripts/spawn-worker.sh` (and again after cleanup) — 1 worker splits beside its orchestrator; 2–4 go to a workers tab in a grid (2 = left|right, 3 = A|B + C below, 4 = 2x2).
- A project's workers live in that project's own tab (e.g. `coucou-workers`), never mixed with another project's workers or the CoS tab.
- Label each pane with the worker name. Never move a pane while its agent is working or during `--wait`.

Why: anh iu, 2026-10-02 — workers left scattered had to be moved by hand (tieude-pillfix + tieude-ccresearch → w4:t3 "coucou-workers").
