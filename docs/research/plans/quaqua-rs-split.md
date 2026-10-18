# quaqua-rs split — LOCK (2026-10-16)

Status: locked, awaiting approval → to-plan

## Goal
The Rust engine lives in its own repo `~/Lab/quaqua-rs`. `~/quaqua` holds only the workspace and uses the installed `quaqua` binary. The workspace is the md files, memory, runs, rules, team.toml and the hooks.

## Decisions (anh iu, 2026-10-16)
- Repo: `~/Lab/quaqua-rs`
- History: keep it with `git subtree split` (blame and PR #42 stay traceable)
- Grid: port `grid-workers.sh` to Rust before the split
- Order: lock the plan and run to-plan for both phases first, then build

## Phase 1: decouple in place (no behaviour change, ships alone)
- Move every hardcoded workspace path into one `workspace_root()`. The default stays `~/quaqua`, and one override (env or team.toml) can change it:
  - `crates/quaqua/src/worker/spawn.rs:257` (rules)
  - `crates/quaqua/src/worker/cleanup.rs:12` (runs)
  - `crates/quaqua/src/config.rs:262` and `crates/quaqua/src/main.rs:183` (team.toml)
- Port `grid-workers.sh` to `quaqua grid`. The binary then stops searching for the script (`cleanup.rs:37-80`), and pipeline/spawn/cleanup call the Rust grid.
- Done when:
  - `rg 'join\("quaqua/' crates/` finds only the default in `workspace_root()`
  - no Rust code shells out to `grid-workers.sh`
  - cargo tests pass, and the grid tests and the targeted bash suites pass

## Phase 2: split the repo
- Move `crates/`, `Cargo.*` and the engine tests to `~/Lab/quaqua-rs` with `git subtree split`, keeping history.
- Install from the new repo with `cargo install --path ~/Lab/quaqua-rs/crates/quaqua`.
- In `~/quaqua`:
  - `.claude/settings.json` calls `quaqua gate` directly
  - `scripts/*` become thin shims or are removed
  - `crates/` is removed in a reviewed commit
- Split tests by owner: engine tests go with the code, workspace and docs tests stay in `~/quaqua`.
- Register `quaqua-rs` in `projects/INDEX.md` with path `~/Lab/quaqua-rs`.
- Done when a fresh `cargo install` from `~/Lab/quaqua-rs` plus `~/quaqua` with no `crates/` passes:
  - a worker spawn → prompt → cleanup smoke run
  - the gate hook

## Steps (to-plan 2026-10-16)
Driver: the Lead `quaqua-rs-lead` (`quaqua lead spawn quaqua-rs`) runs the pipeline in Peer worktrees from `quaqua/quaqua-rs-integ`. The CoS verifies, and anh iu approves every push and every merge to main.

### P1: decouple in place
1. **workspace_root**
   - Builder: a Peer (tieude).
   - Change: add one resolver. It reads `QUAQUA_HOME` first, then `$HOME/quaqua`.
   - Route 4 call sites through it: `spawn.rs:257`, `cleanup.rs:12`, `config.rs:262`, `main.rs:183`.
   - Verify:
     - `rg -n 'join\("quaqua/' crates/quaqua/src` finds only the resolver
     - a new unit test covers the env override and the default
     - `cargo test --locked` passes
2. **grid → Rust**
   - Builder: a Peer (tieude).
   - Change: give `quaqua grid reconcile` the full behaviour of `grid-workers.sh`, including `--only`, `--tab` and `--dry-run`. Then point every caller of `find_grid_script`/`run_grid_hook` at it: spawn, cleanup and pipeline (`pipeline/mod.rs:667-1230`).
   - Verify:
     - `rg -n 'grid-workers.sh' crates/quaqua/src` finds nothing
     - the 25 grid fixtures pass the differential test
     - `bash tests/test_pipeline_grid.sh` and `tests/test_spawn_worker.sh` pass
     - one live spawn→cleanup run on herdr arranges the panes correctly
3. **shim**
   - Change: `scripts/worker/grid-workers.sh` becomes a thin `exec quaqua grid reconcile "$@"`.
   - Verify: `bash tests/run.sh --slow` gives 25 pass / 0 fail (3 skip allowed).
4. **gate P1**
   - Review: suhuynh reviews the integ branch.
   - Then: PR integ→main, anh iu approves the merge, reinstall the binary.
   - P1 is done here. The system works even if P2 never lands.

### P2: split the repo
5. **subtree**
   - Run `git subtree split --prefix=crates -b quaqua-rs-export` on main.
   - Create `~/Lab/quaqua-rs`: root `Cargo.toml` (workspace), `crates/quaqua` from the export, `.github/workflows/ci.yml`, the engine shell tests.
   - Verify:
     - `git log --follow` on one file shows history from before PR #42
     - `cargo test --locked` passes in the new repo
6. **remote**
   - Create a GitHub repo for quaqua-rs and push. This step is outward-facing: ask anh iu for the name, private/public, and approval to push.
7. **consume**
   - `cargo install --path ~/Lab/quaqua-rs/crates/quaqua --locked`.
   - In `~/quaqua`: `.claude/settings.json` calls `quaqua gate` directly.
   - `scripts/*` stay as shims onto the binary.
   - Verify: the gate hook still blocks a direct `herdr pane split` and allows `quaqua worker spawn`.
8. **cut**
   - Remove `crates/`, `Cargo.*`, and the engine tests moved in step 5 from `~/quaqua`. Use a reviewed PR, delete with `git rm`, and give anh iu the list first.
   - Verify:
     - `~/quaqua` contains no `.rs` files
     - `bash tests/run.sh` (the workspace suites) is green
     - a spawn → prompt → cleanup smoke run of a small worker succeeds
9. **register**
   - `projects/INDEX.md`: point quaqua-rs at path `~/Lab/quaqua-rs`.
   - Update `AGENTS.md`, `rules/_all/fanout-agent-grid.md` and `rules/_all/hooks.md`: say the binary comes from `~/Lab/quaqua-rs`, and drop the `scripts/*.sh` names where nothing uses them any more.
   - Rollback: `cargo install` from a `~/quaqua` commit taken before step 8.

## Fragile assumption
Every workspace path the binary uses goes through `workspace_root()`. If one is missed, the binary silently reads or writes `~/quaqua`. The Phase 1 rg check is the guard.

## Out of scope
- New features
- Publishing to crates.io
- Changing CI beyond what `tieude-ci` adds
