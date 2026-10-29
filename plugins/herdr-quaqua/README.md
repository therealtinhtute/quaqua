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
- **`quaqua identity heal`**: lifecycle hook handler triggered on `[[startup]]` and `pane.agent_detected`. Restores lost identity tokens (`grid_role`, `grid_group`, `grid_owner`, `grid_tab`, and `grid_farm` for Leads and workers) across herdr server restarts for Leads, workers, and CoS.

## Farm scoping

A plugin is registered per herdr **daemon**, so `quaqua events hook` (`pane.agent_status_changed`, `pane.exited`) receives events from every workspace on the machine, while `runs/quaqua.db` belongs to exactly one workspace — a *farm*, whose id is the last component of the canonicalized root (`~/quaqua` → `quaqua`). The hook attributes each event before writing.

- **Ingest rule** (`bus::events::attribute_pane`, evaluated in order): token `grid_farm` equal to the local farm → local; token present and different → foreign; `cwd` canonical-prefixed by the farm root → local; a `cwd` ancestor carrying `runs/<ancestor>.owner` → local (worker worktrees under `.wt/<name>`, subdirectories included); `pane get` answering `pane_not_found` → foreign; any other failure or unparseable output (herdr down, timeout, empty stdout) → local with one warning on stderr — fail-open, so an unhealthy herdr never silences the notification channel.
- A **foreign** event is dropped before `FarmBus::record` and before `herdr notification show`: another workspace's blocked pane neither enters this farm's inbox nor pops a desktop notification here.
- `pane.exited` skips the ladder — the pane is already gone — and is recorded as local with `owner`/`project` read from `runs/<agent>.owner` / `runs/<agent>.repo`.
- A local event is stamped with the farm plus `owner` = token `grid_owner` (falling back to `runs/<name>.owner`) and `project` = token `grid_group` (falling back to the basename of the worktree's `runs/<name>.repo`). Readers filter on that stamp: the CoS sees every local incident, a Lead only those whose `owner` is its own name; mail is scoped the same way.
- `grid_farm` is written by `worker spawn`, `lead spawn` / `lead tag` / `lead repair`, and re-applied by `quaqua identity heal` (Leads through `lead tag`, plus worker panes) after a herdr restart. The CoS pane (`quaqua cos tag`) carries no `grid_farm` and is attributed by its `cwd`.
- Cost: one `herdr pane get` per status-change event, inside the same `QUAQUA_HOOK_TIMEOUT_MS` (default 1500 ms) deadline as the existing notification call.

## Configuration

- `HERDR_QUAQUA_SHADOW`: Set to `1` (default) for dry-run observation or `0` for live execution. Can also be set in `$STATE_DIR/shadow`.
- `HERDR_GRID_SHADOW`: legacy name still honoured as a fallback when `HERDR_QUAQUA_SHADOW` is unset.
- State directory defaults to `$HERDR_PLUGIN_STATE_DIR` or `~/.local/state/herdr/plugins/herdr-quaqua`. The previous name was `herdr-grid`; old state (`~/.local/state/herdr/plugins/herdr-grid`) is not migrated automatically.
