# quaqua on Rust — one binary rewrite

Status: locked 2026-10-15 (brainstorm + think, approved by anh iu). Supersedes the herdr-grid-only plan (d1d0069, 9471b16) and `docs/research/team-config-schema.md`; folds in piggery ideas (github.com/sting8k/piggery `main`: manifests/, manifests/rules/general-policy.md, manifests/prompts/).

## Goal
One stateless Rust binary `quaqua` replaces `scripts/` and `plugins/herdr-grid/` (~3.9k lines of bash/jq). It reads one `team.toml` that declares roles, who may spawn or prompt whom, limits, timers, kinds, taskforces and grid groups. Cutover happens in one PR.

Goal is **correctness and testability, not a security boundary.** Herdr v1 has no SDK and no mail daemon, so the fence is still the PreToolUse hook, now `quaqua gate`. If an agent calls herdr indirectly, the hook misses it, the same as today.

## Decisions (2026-10-15)
| Decision | Choice | Rejected |
|---|---|---|
| Shape | One binary, big-bang cutover | Layered strangler with Bash fallback (safer, but anh iu wants one release) |
| Fence | Stateless CLI + hook | piggery-style daemon owning mail, gate and SQLite (too much scope; `runs/*.md` already cover what mail does) |
| Config format | TOML (`team.toml`) | YAML as piggery uses (weaker typing, footguns) |
| Policy rules | Ship in the cutover PR | Ship now as md-only |
| Cutover gate | All tests + edge fixtures + 1 real SLP run | Tests only |
| Rollback | `git revert` the cutover PR (bash stays on `main` until then) | Keep both engines live |

## Non-goals
- No daemon, mailbox or SQLite. Reports stay `runs/<name>.md` (atomic tmp → mv).
- No peer-to-peer routing: Peers report only to their orchestrator.
- Out of the binary: `scripts/auditor/*` (cron, read-only) and `scripts/hooks/pre-commit`.
- Brief format unchanged (`rules/_all/brief.md` already matches piggery's: outcome, limits, unknowns, check, no pre-solve).

## Command map
Every old script becomes a one-line shim `exec quaqua <cmd> "$@"`. That way `tests/test_*.sh`, AGENTS.md paths and muscle memory keep working unchanged.

| Old | New | Notes |
|---|---|---|
| `scripts/worker/spawn-worker.sh` | `quaqua worker spawn` | enforces `roles.<caller>.can_spawn`, cap, depth |
| `scripts/worker/prompt-worker.sh` (+watchdog) | `quaqua worker prompt` | exit codes 0/2–6, `--check-brief`, `--resume`, `--extend` unchanged; time via the fake-clock `now_sec` hook |
| `scripts/worker/cleanup-worker.sh` | `quaqua worker cleanup` | |
| `scripts/worker/worker-cap.sh` | `quaqua cap` | pipeline and dual-lens weigh 2 |
| `scripts/worker/grid-workers.sh` + `plugins/herdr-grid/{hook.sh,plan.jq,apply.sh}` | `quaqua grid {reconcile,hook,plan}` | plugin manifest `command = ["quaqua","grid","hook"]`; `plan` is pure (snapshot → ops JSON) |
| `scripts/lead/spawn-lead.sh`, `cleanup-lead.sh` | `quaqua lead {spawn,cleanup}` | still injects `rules/_all/*.md` as the system prompt |
| `scripts/lead/pipeline.sh` | `quaqua pipeline` | |
| `scripts/hooks/herdr-gate.sh` | `quaqua gate` | same tokenizer semantics, same `test_herdr_gate.sh` cases |
| `scripts/workspace/init-workspace.sh` | `quaqua init` | |
| `scripts/lib/common.sh` | internal module | |
| — (new) | `quaqua config {validate,get}` | bash-free config access; validation runs at every command start |
| — (new, piggery refinery) | `quaqua merge <slug> <branch>…` | Lead merge queue (below) |
| — (new, piggery taskforce) | `quaqua taskforce {advisor,dual-lens}` | read-only ephemeral workers (below) |

## `team.toml` (single source of truth)
```toml
schema_version = 2

[limits]
depth = 2                         # CoS → Lead → Peer
max_workers_per_orchestrator = 4  # CoS and each Lead; Leads don't count
weights = { pipeline = 2, "dual-lens" = 2 }

[timers]                          # watchdog, rules/_all/worker-timebox.md
budget_default = "45m"            # brief `Time budget:` wins
hard_cap = "120m"
stall = "10m"
idle = "2m"
nudge_at = 0.8
grace = 0.25
pilot_budget = "30m"

[roles.cos]
can_spawn  = ["lead", "peer", "taskforce"]
can_prompt = ["lead", "peer"]
[roles.lead]
can_spawn  = ["peer", "taskforce"]
can_prompt = ["peer"]
reports_to = "cos"
[roles.peer]
can_spawn  = []
can_prompt = []
reports_to = "owner"

[strategy]
default_kind = "tieude"           # anh iu, 2026-10-15
default_reviewer = ["quansu", "suhuynh"]  # first available wins (anh iu, 2026-10-15)
builder_reviewer_differ = "harness"

[kinds.tieude]                    # harness·model·effort move here from models.md; prose stays there
harness = "opencode"
model = "opencode-go/deepseek-v4.1-flash"
can_review = false                # true for bacdau, quansu (codex profile) and ni, suhuynh, truonglao (claude read-only flags)
# … detu, sude, ni, suhuynh, truonglao, bacdau, quansu — values copied verbatim, never changed

[taskforce.advisor]
members = ["quansu"]
read_only = true
[taskforce.dual-lens]
members = ["truonglao", "quansu"]
read_only = true

[merge]
integ_branch = "quaqua/{slug}-integ"

[grid]                            # was M3: groups, tab names, deferred statuses
workers_tab = "workers"
deferred_statuses = ["working"]

[projects.zimage]                 # optional per-project override, merged over defaults
max_workers_per_orchestrator = 3
```
Resolution: project override → defaults → reject. Validation rejects: workers over the cap, builder and reviewer on the same harness, a reviewer whose kind has `can_review = false`, unknown kind or role, a kind whose model differs from `models.md` at startup (model drift).

Rule → key coverage is a gate: every rule in AGENTS.md `## Rules`/`## Protocol`, `worker-cap.sh`, `worker-timebox.md` and `models.md` must map to a key or stay prose by name. Nothing is dropped silently.

## New behaviour from piggery
**Merge queue (`quaqua merge`):** one branch at a time, in order. Note the integ commit, merge, run the project check. If it passes, record the new commit. If it conflicts or fails, undo (`merge --abort` / `reset --hard <noted>`), return rework to the Peer, and never fix it yourself. If the check fails without the branch too, ask the CoS and hold the queue. Never rewrite integ history except to undo its own last merge. Never push.

**Taskforce (`quaqua taskforce`):** short-lived and read-only, it reports to the caller, and the caller cleans it up after reading `runs/*.md`. `advisor` returns: recommendation + confidence, evidence, alternatives, strongest downside, and the smallest check that would prove it wrong (marked not run). `dual-lens` gives both members the same neutral brief; the caller chairs: points both agree on are settled, and each conflict goes to the other lens once. Counts toward the cap per `weights`.

**Silence:** a Peer that goes quiet gets its pane read first, then `--resume`, and is never replaced. That's piggery's rule. The existing kill rule (same error ≥3 iterations → fresh worker) stays as is; it covers a different failure.

**Policy rules (shipped in the cutover PR, ≤25 lines each, injected into every prompt):**
- `rules/_all/change-policy.md`: 5 questions before fixing a mechanism (outcome needed · what it served · real vs implementation constraints · keeping wrong behaviour? · patch, replace or remove). Tests: one per invariant or acceptance criterion; a bug fix starts with a repro that fails on old code; no tests for wording, format or layout; extend before adding; report test lines added and removed. Docs: one home per fact, deleting stale text. Scope: project repos; quaqua `runs/`, `raw/` and handoffs are evidence and exempt.
- `rules/_all/merge-queue.md`: Leads merge only via `quaqua merge`.
- `rules/_all/taskforce.md`: when to call advisor or dual-lens instead of a builder.

## Build phases (all on one branch; `main` untouched until R6)
| # | Phase | Done when |
|---|---|---|
| R0 | Crate scaffold `crates/quaqua/` (clap, serde, toml), `cargo test` runs from `tests/run.sh` | `cargo build --release` on macOS + Linux |
| R1 | `team.toml` + `config validate/get` + rule → key table | table complete; invalid fixtures rejected |
| R2 | `grid` (pure planner + apply + hook) | existing `tests/fixtures/herdr-grid` + 9 edge fixtures pass; planner output equals `plan.jq` on every fixture (differential) |
| R3 | `worker spawn/prompt/cleanup`, `cap` | `test_spawn_worker`, `test_prompt_worker*`, `test_cleanup_worker`, `test_worker_cap` pass against shims; `--review` works for every `can_review` kind on each harness (codex, claude) |
| R4 | `lead`, `pipeline`, `merge`, `taskforce` | `test_spawn_lead`, `test_cleanup_lead`, `test_pipeline` pass; new tests: merge-queue undo on failing check, taskforce weight 2, pipeline builder + reviewer land in `GRID_TAB` (2 workers → workers tab, never the Lead tab; bash bug 2026-10-15) |
| R5 | `gate`, `init` | `test_herdr_gate`, `test_init_workspace` pass |
| R6 | Cutover PR: shims, plugin manifest, `.claude/settings.json` hook → `quaqua gate`, 3 policy rules, AGENTS.md/models.md/`rules/_all/*` path updates, `trash` the replaced bash bodies | gate below |

**Drift guard:** while R0–R5 run, any change to `scripts/` on `main` is ported to the binary in the same week, or `scripts/` is frozen. Choose per change, and log it in the project card.

## Cutover gate (R6)
1. `tests/run.sh` all green against the shims.
2. All 9 edge fixtures pass (`docs/research/herdr-grid-rust-edgecases.md`): wrong tab, foreign pane, zoomed anchor, missing workers tab, stale session path, event storm, empty workspace, failed apply + backoff, cross-owner group.
3. One real SLP run on the binary: CoS → Lead → 2 Peers → `quaqua merge` into integ, with the grid converged and `runs/*.md` read.
4. anh iu approves the merge to `main`.

Rollback: `git revert` the cutover commit. The bash bodies come back intact.

## Packaging
- Platforms linux/macos; plugin keeps `min_herdr_version`; executable bits preserved.
- Shims find the binary on `PATH`, else `$QUAQUA_BIN`; `quaqua init` prints the install line (`cargo install --path crates/quaqua`).
- Public export: no corporate or private metadata; commit history stays documented.

## Execution (to-plan 2026-10-15)
Who: R0–R5 = Lead `quaqua-rs-lead` (SLP, pipeline per phase, Peers `--base quaqua/quaqua-rs-integ`). R6 touches `scripts/`, `rules/`, AGENTS.md, which are CoS-only per `spawn-lead.sh:138`, so the CoS runs R6 itself with one worker, no Lead.
Kinds: builder `sude` (heavy: R2, R3, R4) or `tieude` (R0, R1, R5); reviewer `suhuynh` (quansu unavailable, anh iu 2026-10-15), on a different harness (omp/opencode vs claude).

### S0 Prerequisites (CoS)
1. Commit this plan + `models.md` on `docs/rust-herdr-plugin-brainstorm`. Verify: `git status` clean for those paths.
2. **anh iu approves** merging that branch into `main`, because `spawn-lead.sh` cuts `quaqua/quaqua-rs-integ` from `main` and Peers must see this plan. Verify: `git log main -1 -- docs/research/herdr-grid-rust-migration-plan.md`.
3. Register project `quaqua-rs`: INDEX row (repo `~/quaqua`, owner `quaqua-rs-lead`) + card `projects/quaqua-rs.md` (goal, stack Rust + bash, drift guard, gate). Verify: `spawn-lead.sh` dry checks pass (slug known, card exists).
4. `scripts/lead/spawn-lead.sh quaqua-rs` → `scripts/worker/grid-workers.sh`; Lead brief = this file, § Build phases + § Cutover gate.

### S1 Lead order (dependencies → parallelism; cap 4, pipeline = 2 → ≤2 pipelines at once)
| Step | Phase | Depends on | Parallel with | Check the Lead re-runs |
|---|---|---|---|---|
| 1 | R0 scaffold `crates/quaqua/`, `tests/run.sh` also runs `cargo test` | S0 | – | `cargo build --release && cargo test`; `tests/run.sh` diff adds the `cargo test` call (full suite waits for step 7) |
| 2 | R1 `team.toml` + `config validate/get` + rule → key table (in this file) | R0 | – | `cargo test config`; every row of the table → key or "prose" |
| 3 | R2 `grid` planner/apply/hook | R1 | R3 | differential: `quaqua grid plan` vs `plan.jq` on all 13 `tests/fixtures/herdr-grid/*.json` + 9 edge fixtures |
| 4 | R3 `worker spawn/prompt/cleanup`, `cap` (includes the omp `sude` branch and the claude `--review` mode) | R1 | R2 | `test_spawn_worker`, `test_prompt_worker*`, `test_cleanup_worker`, `test_worker_cap` against shims in the Peer worktree |
| 5 | R4 `lead`, `pipeline`, `merge`, `taskforce` | R3 | R5 | `test_spawn_lead`, `test_cleanup_lead`, `test_pipeline` + new merge-undo + taskforce-weight + pipeline-grid-tab tests |
| 6 | R5 `gate`, `init` | R1 | R4 | `test_herdr_gate`, `test_init_workspace` |
| 7 | Lead handback: integ branch, commit, all checks with real output | R2–R5 | – | CoS re-runs `cargo test && bash tests/run.sh` (shims) on `quaqua/quaqua-rs-integ` |

Each step = `pipeline.sh <phase> ~/quaqua --base quaqua/quaqua-rs-integ --brief <file>`; the builder plans first, the Lead approves, then it builds. The Lead merges accepted branches via the merge-queue discipline (§ New behaviour), since `quaqua merge` doesn't exist yet.

### S2 Drift guard (CoS, during S1)
- Any `scripts/` change landing on `main` (e.g. the 2026-10-15 `sude` → omp change) → message the Lead the same day: port it into the phase that owns it. Log it in `projects/quaqua-rs.md`.

### S3 Cutover R6 (CoS + one `sude` worker on a branch from integ)
1. Worker: shims for every row of § Command map, plugin manifest → `["quaqua","grid","hook"]`, `.claude/settings.json` hook → `quaqua gate`, `trash` the old bash bodies. Verify: `bash tests/run.sh` all PASS.
2. CoS: write the 3 policy rules, update AGENTS.md/models.md/`rules/_all/*` paths + `[kinds]` move. Verify: `bash tests/test_docs.sh` PASS; `quaqua config validate` OK.
3. CoS: real SLP run on the binary (CoS → Lead → 2 Peers → `quaqua merge`), with grid converged and `runs/*.md` read.
4. **anh iu approves** the cutover PR → merge to `main`. Rollback = `git revert`.
5. `cleanup-lead.sh quaqua-rs`; promote 0–1 lesson; card `## Now` updated.

## Open
none
