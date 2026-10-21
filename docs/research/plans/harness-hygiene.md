# Harness & hygiene — PR specs

Source: brainstorm 2026-10-21 (pstack @ 190d287 vs quaqua main d24be71).
Status: **draft spec** (chưa giao worker) · spec only, nothing implemented. [F] = verified in repo/CLI, [A] = assumption the PR must confirm first.

Not from pstack: findings from the 2026-10-21 workspace audit (drift, reviewer sandbox) and the six-concept brief (Codex, harness adapter). Runs in parallel with `pstack-integration.md`, except H1 merges before its P1.

Locked decisions: Codex now · reviewer read-only right after H1 · `harness/` module is a pure move · keep suhuynh as reviewer (accepted risk).

Order: H1 → H2 → H3 → H4. Each one works on its own after merge.

---

## H1 — Fix contract drift

Goal: the rules a worker reads match what the engine enforces.
Scope:
- `templates/workspace/.claude/skills/herdr-orchestration/SKILL.md` ("Max 4" → follow `team.toml` `max_workers_per_orchestrator = 8`) [F]
- `templates/workspace/PEER.md` + `templates/workspace/rules/_all/brief.md`: merge the 6-field handback (Outcome/Snapshot/Scope/Verification/Unknown/Ownership) into the `runs/<name>.md` report format, one contract [F]
- `brief.md` says numbered headings fail; `watchdog.rs:22-49` accepts `## 5. Lessons` [F]. Fix the doc so it matches the code.
Acceptance:
- One handback format, referenced from both PEER.md and brief.md
- `grep -rn "Max 4"` under templates/ returns nothing
Verify: `cargo test -p quaqua watchdog`; `quaqua worker prompt --check-brief` on a sample brief
Forbidden: changing watchdog parsing logic

## H2 — Read-only reviewers on every harness

Goal: a review worker cannot edit the worktree on any harness.
Current: only claude gets `deny Edit(worktree)` + `dontAsk` (`launch.rs:98-150`) [F]. opencode/omp reviewers can write.
Scope: `crates/quaqua/src/worker/launch.rs`, `team.toml` comment ("claude read-only flags" is stale) [F]
Design:
- opencode: add a permission deny for edit/write to `OPENCODE_CONFIG_CONTENT` when role=review [A: confirm opencode config key]
- omp: find its read-only/tool-deny flag [A]. If none exists, refuse omp as a reviewer and error clearly, never silently allow writes.
Acceptance: unit test per harness asserting the review launch args contain the deny; spawn a review on opencode in a scratch worktree, the edit attempt is refused
Verify: `cargo test -p quaqua launch`
Unknown: omp capability → may shrink to "opencode + refuse omp"

## H3 — Codex harness

Goal: `kind = codex` spawns, gets watched and is reviewed like the other harnesses.
Facts:
- codex-cli 0.163.0 installed [F]; flags `-m <model>`, `-c key=value`, `-s/--sandbox <mode>` [F]
- herdr integration for codex exists but is **outdated (v8 < v10)** [F] → `herdr integration install codex` must run first (needs ní's OK, it writes to `~/.codex`)
Scope: `config.rs:209` (allowlist, `effort_allowed`), `launch.rs`, `auditor.rs:236`, `usage.rs:344`, `models.md`/`team.toml` example entry
Design:
- launch: `codex -m <model> -c model_reasoning_effort=<effort> -s workspace-write` (build) / `-s read-only` (review)
- rules channel: [A] how to inject `runs/<name>.rules.md` (`-c` instructions key vs `AGENTS.md` in the worktree). Settle by test before coding.
- cost reader: [A] location of Codex session logs (`~/.codex/sessions`?). Without one, usage shows "n/a", not 0.
Acceptance: `herdr agent wait` sees idle/done for a codex pane; watchdog exit codes hold; review runs with `-s read-only`
Verify: `cargo test -p quaqua`, targeted to config/launch; one live spawn of a 1-line task in a scratch worktree
Risk: adds a 5th branch to every `match harness`, which H4 moves into `harness/` later (accepted)

## H4 — `harness/` module

Goal: one place per harness: launch args, rules channel, read-only mode, effort/variants, cost reader, quirks (claude trust dialog `spawn.rs:434`).
Scope: new `crates/quaqua/src/harness/{mod,claude,opencode,omp,codex}.rs`; remove the `match harness` sites in launch/config/auditor/usage/spawn
Design: a `Harness` trait or enum dispatch. Pure move, no behavior change.
Acceptance: the existing launch-arg snapshot tests are byte-identical before and after; `rg "match .*harness"` outside harness/ → 0
Verify: `cargo test -p quaqua`
Check `engine/OWNERS.md` for file ownership before moving code.
