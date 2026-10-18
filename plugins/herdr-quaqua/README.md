# herdr-quaqua

Automatic agent grid layout reconciler for Herdr.

## Overview

`herdr-quaqua` is an event-driven Herdr plugin that arranges worker panes into an organized grid layout according to the grid formula:
- 1 worker total per owner: split beside owner pane (2/5 owner, 3/5 worker).
- Multiple workers: dedicated `<group>-workers` tab per project/group:
  - 1 worker: full tab
  - 2 workers: left | right (50/50 split)
  - 3 workers: top split (A | B) + bottom pane (C)
  - 4 workers: 2x2 grid
  - >4 workers: rejected

## Architecture

- **`herdr-plugin.toml`**: Plugin manifest declaring subscriptions for lifecycle events:
  - `pane.created`, `pane.agent_detected`, `pane.agent_status_changed`, `pane.moved`, `tab.closed`, `workspace.closed`.
  - Never subscribes to `layout.updated` to avoid recursive trigger loops.
- **`plan.jq`**: Pure jq planner transforming `herdr api snapshot` into an ordered list of `pane.move` operations. Retained as the differential oracle for the Rust planner.
- **`quaqua grid hook`** (was `hook.sh`): event hook handler, the manifest's `command = ["quaqua", "grid", "hook"]`. Same behaviour as the retired bash body:
  - Serialized via a lock directory with dead-PID reclaim and a pending event counter.
  - Detects closed panes via snapshot diff against the last observed snapshot (handles cascaded pane closures from tab and workspace closing where `pane.closed` is not emitted).
  - Early-exits on `pane.agent_status_changed` events when the status change does not affect layout execution (e.g. non-worker panes or transitions between non-working states `idle`, `done`, `unknown`).
  - Runs in **shadow mode** by default: calculates layout ops using the Rust planner and logs them without modifying panes.
  - When shadow mode is disabled (`HERDR_QUAQUA_SHADOW=0`), applies non-deferred moves itself (was `apply.sh`).

## Configuration

- `HERDR_QUAQUA_SHADOW`: Set to `1` (default) for dry-run observation or `0` for live execution. Can also be set in `$STATE_DIR/shadow`.
- `HERDR_GRID_SHADOW`: legacy name still honoured as a fallback when `HERDR_QUAQUA_SHADOW` is unset.
- State directory defaults to `$HERDR_PLUGIN_STATE_DIR` or `~/.local/state/herdr/plugins/herdr-quaqua`. The previous name was `herdr-grid`; old state (`~/.local/state/herdr/plugins/herdr-grid`) is not migrated automatically.
