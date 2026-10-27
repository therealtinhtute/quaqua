---
status: active
lane: high-risk
---
# Split the CoS workspace from the engine checkout

## Goal
- outcome: `~/quaqua` is only the CoS workspace (no `.git`, no engine files) with the workspace `AGENTS.md` that imports `@identity.md` and `@SOUL.md`; the engine is developed only from `~/Lab/quaqua`; `quaqua doctor` warns when a workspace is also an engine checkout.
- success_signal: a fresh Claude Code session in `~/quaqua` replies with "em"/"anh iu" and the 🍑 prefix, both at start and after `/compact`; `quaqua doctor` in `~/quaqua` reports no workspace-layout warning; a worker spawned from `~/Lab/quaqua` loads the engine `AGENTS.md` and no persona.
- actors: anh iu (owner); CoS Quả Quả (runs the migration); Leads and workers (spawn from the engine repo); `quaqua doctor`.
- authority: anh iu's decisions in the 2026-10-24 grill (CoS-only persona, `@import` via `AGENTS.md`, workspace persona overrides global tui/ní voice, split after gate-lead finishes, doctor warning, trash old `.git` after reconciling branches); root cause: commit 462855e moved the workspace `AGENTS.md` into `templates/workspace/`, so the engine `AGENTS.md` at the root of `~/quaqua` dropped the persona imports.
- requirements:
  - R1: The migration starts only when no Lead or worker is live and no worktree hangs off `~/quaqua` | acceptance: `quaqua status` shows no Lead/worker rows and `git -C ~/quaqua worktree list` lists only the main checkout | source: grill (split after gate-lead)
  - R2: `~/Lab/quaqua` is on the current `origin/main`, and every `quaqua/*` branch in `~/quaqua` exists on origin or in `~/Lab/quaqua` before anything is removed | acceptance: a script that diffs `git for-each-ref refs/heads/quaqua/` from `~/quaqua` against origin plus `~/Lab/quaqua` prints no missing branch | source: grill (trash after reconciling)
  - R3: `~/quaqua` keeps every workspace file (`memory/ projects/ runs/ raw/ reports/ USER.md identity.md SOUL.md PEER.md team.toml models.md playbooks/ principles/ rules/ scripts/ .claude/ .quaqua/ .harness/`); `.git` and engine-only files (`crates/ tests/ plugins/ docs/ .github/ Cargo.* target/ README.md SETUP-PROMPT.md LICENSE templates/`) go to the trash via `trash` | acceptance: a before/after listing shows each workspace path still present and each engine path gone; `runs/quaqua.db` opens with `quaqua status` | source: grill
  - R4: The root `AGENTS.md` in `~/quaqua` is the workspace `AGENTS.md` (from `quaqua init` or `templates/workspace/AGENTS.md`) and contains `@identity.md` and `@SOUL.md` | acceptance: `grep -c '^@identity.md$' ~/quaqua/AGENTS.md` prints 1; the success_signal session check passes | source: grill (`@import` mechanism)
  - R5: The persona applies to the CoS only: Leads skip `## Identity` via the Role gate, and workers in engine worktrees never load it | acceptance: a worker spawned from `~/Lab/quaqua` answers a probe without "em"/"anh iu"; Lead spawn still reads its brief | source: grill (CoS-only)
  - R6: Every path that names `~/quaqua` as the engine repo (spawn, worktree base, `doctor.rs:120` fallback, `team.toml`, `runs/*.owner`, `projects/INDEX.md`, memory notes) points to `~/Lab/quaqua` | acceptance: `rg -n '~/quaqua|/Users/tinhtute/quaqua' ~/quaqua ~/Lab/quaqua/crates` shows only workspace uses; one `quaqua worker spawn` from the CoS creates its worktree from `~/Lab/quaqua` | source: grill
  - R7: `quaqua doctor` warns when the workspace root holds `Cargo.toml` or `crates/`, or when the root `AGENTS.md` lacks `@identity.md`, and no longer assumes `~/quaqua` is the engine source | acceptance: new doctor tests are red on main and green on the branch; `cargo test --locked` passes | source: grill (doctor guard)
  - R8: Engine-dev artifacts (`.harness/`, the `PROJECT.md` gate commands) move to `~/Lab/quaqua` so the engine plans live with the engine | acceptance: `~/Lab/quaqua/.harness/PROJECT.md` exists and this plan is reachable from there | source: derived from R3 (no engine state in the workspace)
- non-goals:
  - NG1: No persona for Leads or workers; no change to the wording of `identity.md` or `SOUL.md`.
  - NG2: No change to global `~/.claude/CLAUDE.md`; it keeps tui/ní as the default voice.
  - NG3: No SessionStart persona hook.
  - NG4: No change to how consumer workspaces other than `~/quaqua` are created, beyond the doctor warning.
- open_questions:
  - Which files in `team.toml`, `runs/*.owner` and `projects/INDEX.md` hard-code `~/quaqua` as the repo path | owner: CoS while planning (R6)
- adr_candidates:
  - Trashing the old `~/quaqua/.git` is hard to reverse; R2 gates it.

## Phases and Verification
- approach: copy branches locally into `~/Lab/quaqua` first (no push needed), then a one-shot CoS migration once agents are idle; the doctor guard runs in parallel as a normal worker PR. Rejected: a staged multi-day move, since R6 found only one code fallback and 7 INDEX rows.
- inventory (R6, 2026-10-24): code has `doctor.rs:120` fallback and `$HOME/quaqua/runs` for review workers (workspace path, stays); `worker spawn` takes `<repo-path>` explicitly; `projects/INDEX.md` rows 12–20 name `~/quaqua` as repo; `rules/_all/lead-inbox.md` and `.claude/` refer to `~/quaqua` as the workspace (stay); 31 local `quaqua/*` branches; 2 clean worktrees in `~/Lab/.wt/` (farm-p0, farm-briefs) hang off `~/quaqua/.git`.
- phase_slug: `prep`
  status: planned
  - goal: R2, R8 | depends_on: none
  - steps: `git -C ~/Lab/quaqua pull --ff-only`; `git -C ~/Lab/quaqua fetch ~/quaqua 'refs/heads/quaqua/*:refs/heads/quaqua/*'`; copy `.harness/` to `~/Lab/quaqua/.harness/`
  - phase check: branch diff script prints nothing missing; `~/Lab/quaqua` HEAD == `origin/main`; `~/Lab/quaqua/.harness/PROJECT.md` exists
- phase_slug: `doctor`
  status: planned
  - goal: R7 | depends_on: prep
  - owner: one worker spawned with repo `~/Lab/quaqua`, PR to main
  - surfaces: `crates/quaqua/src/doctor.rs`, its tests | avoided: everything else
  - phase check: new doctor tests red on main, green on branch; `cargo test --locked`
- phase_slug: `migrate`
  status: planned
  - goal: R1, R3, R4, R5, R6 | depends_on: prep, gate-lead done
  - steps: confirm R1; recreate farm-p0/farm-briefs worktrees from `~/Lab/quaqua` (both clean); `trash` engine paths and `.git` in `~/quaqua`; copy `templates/workspace/AGENTS.md` from `~/Lab/quaqua` to `~/quaqua/AGENTS.md`; set INDEX repo column to `~/Lab/quaqua`; add memory note "spawn workers with repo ~/Lab/quaqua"
  - phase check: R3 before/after listing; `quaqua status` reads `runs/quaqua.db`; `grep -c '^@identity.md$' ~/quaqua/AGENTS.md` = 1; new session in `~/quaqua` answers em/anh iu before and after `/compact`; probe worker from `~/Lab/quaqua` has no persona

## Log
- 2026-10-24 prep done: Lab on origin/main, 31 quaqua/* branches copied locally, plan + PROJECT.md copied
- 2026-10-25 doctor done: PR #142 merged (1727f51), review suhuynh 2 findings fixed
- 2026-10-25 migrate done: gate PR #143 merged + gate-lead cleaned (R1); stash kept as quaqua/stash-cos-guard-wip-1017; farm worktrees re-homed to ~/Lab/quaqua; .git + engine files trashed; `quaqua init --update` adopted workspace AGENTS.md + settings.json; INDEX repo column -> ~/Lab/quaqua; `quaqua doctor` in ~/quaqua quiet

## Validation

## Current State and Next Action
- active_phase: migrate
- lifecycle_status: verifying
- blockers: none
- open_items: persona check in a fresh session before/after /compact; .quaqua/pending models.md + team.toml conflicts for anh iu
- exact_next_action: open a new CoS session in ~/quaqua and confirm em/anh iu, then move plan to done
