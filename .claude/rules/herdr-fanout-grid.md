# herdr fanout grid — MANDATORY

Applies to every Claude Code session in ~/quaqua that spawns or prompts herdr agents (CoS, project orchestrators, EA).

1. After every `scripts/spawn-worker.sh` (and after `scripts/cleanup-worker.sh`), run `scripts/grid-workers.sh` BEFORE any `herdr agent prompt`.
2. 1 worker → split beside its orchestrator; 2–4 → the project's own workers tab in a grid (2 = left|right, 3 = A|B + C below, 4 = 2x2). Never mix projects or put workers in the CoS tab.
3. Close the pane as soon as the run is verified: `scripts/cleanup-worker.sh` right after `runs/<name>.md` is read and its test re-run passes (branch kept; merge still needs approval).
4. Label panes with the worker name; never move a pane while its agent works or during `--wait`.

Details + why: `rules/_all/fanout-agent-grid.md`.
