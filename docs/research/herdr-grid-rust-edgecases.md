# Herdr Grid Rust Migration Edge Cases

## Observed signal

- Worker `tieude-pubfix` (`w9:p2R`) was tagged but sat in tab `w9:t1A` with `quaqua-cos`, not the worker tab.
- A shell glob to `.../herdr-grid/*.log` failed because no exact files matched.
- Agent session paths and branch names can drift from pane/tab state.

The useful diagnosis is **layout/identity drift**, not necessarily a failed op.

## Required spec/plan additions

1. Stale worker inside non-worker tab
- Detect any worker-token pane that is not in its canonical group target tab.
- Move only verified workers.
- Do not use pane `cwd`, branch label, or session JSON path as routing truth; use tokens and policy.

2. Wrong or missing tab
- Refresh tab list every reconcile.
- If canonical group tab is unavailable, create or reuse a non-CoS existing tab.
- No hidden dependency on a literal tab label besides config.

3. Foreign / untagged panes
- Keep skip conditions.
- Add `grid_ready` drift test.
- Avoid moving a pane with unresolved owner/group.

4. Zoomed or archived restore
- Save snapshot after every apply.
- Validate tab zoom state before first pane move.
- Add restore fixture in tests.

5. Agent name collision
- Agent names are live only and server-scoped.
- Rust planner ignores names; worker routing uses tokens.
- Startup smoke tests use unique random names.

6. Event log instead of glob
- Remove reliance on `*.log` glob.
- Use deterministic `events.jsonl` and `shadow.log` in `HERDR_PLUGIN_STATE_DIR`.
- Include old/new pane IDs, snapshot seq, deferred state, apply status.

7. No workers
- Return `ops []`.
- Do not error; emit status screen.
- A clear action may be shown as configured, but no required action by default.

8. Provider/owner/group drift
- Verify owner and group before apply.
- Block cross-project moves.
- Retain previous tab ID through remap.

9. Build/maintenance
- Track `min_herdr_version`.
- Preserve executable bits for binaries.
- Manifest platform overrides only when necessary.

## Test cases to add before cutover

- `worker_in_wrong_tab.json`
- `worker_with_foreign_pane.json`
- `zoomed_anchor_tab.json`
- `missing_workers_tab.json`
- `stale_session_path.json`
- `event_storm_after_move.json`
- `divide_initial_empty_workspace.json`
- `failed_apply_then_backoff.json`
- `cross_checker_owner_group.json`

## Migration plan notes

- Run Rust planner differential tests against all fixture categories.
- Shadow mode must log convergence for 2-4 worker tab sets.
- Live apply only after logs are empty `[]`.
- Keep Bash fallback until option 1 telemetry is clean.
- Add config validation and migration defaults for schema changes.
