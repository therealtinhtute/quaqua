# Herdr Rust Plugin Migration Research

## Goal

Move the `herdr-grid` workspace plugin from Bash/jq to a Rust CLI-driven plugin without changing the Herdr host contract.

## Constraints

- Herdr v1 plugins expose no native SDK; every integration goes through manifest commands plus `HERDR_BIN_PATH`.
- Primary use of Herdr is terminal/workspace, tab, pane, and agent lifecycle.
- Layout actions must remain reversible and safe for foreign panes.
- Config changes should be data-driven.

## Phase plan

### M0 Research and schema
- Document worker rules in TOML.
- Replace hard-coded plan/jq rules with typed planner.
- Add differential tests using fixture files.

### M1 Rust planner crate
- Port `plan.jq` into pure functions.
- Test convergence for 0, 1, 2, 3, 4 and 5 workers.

### M2 Shadow runner
- Rust command emits plan only, no `pane.move`.
- Validate against 5 real spawn sessions before live mode.

### M3 Config-driven team layout
- Externalize role count, group mapping, strategy, deferred statuses.
- Add config validation in Rust before execution.

### M4 Host commands and channels
- Call `herdr api snapshot` and `herdr pane move`.
- Keep failure modes (closed panes, zoom, foreign panes, backoff).

### M5 Rust hook live
- Replace event collection and locking.
- Preserve pending-counter lock logic.
- Keep immediate fallback to Bash.

### M6 Packaging
- Cargo build manifest commands.
- Target platforms linux/macos.
- Publish clean GitHub path.
- Ensure `--ref main` install works and native permissions stay intact.

## Criteria

- 176 commits history stays documented.
- No corporate/private metadata in public export.
- Group and role config change without binary rebuild.
- Test gates pass before any cutover.
