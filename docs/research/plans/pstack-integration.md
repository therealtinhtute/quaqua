# pstack integration — PR specs

Source: brainstorm 2026-10-21 (pstack @ 190d287 vs quaqua main d24be71).
Status: **draft spec** (chưa giao worker) · 2026-10-21 · spec only, nothing implemented. [F] = verified in repo/CLI, [A] = assumption the PR must confirm first.

Locked decisions: port ideas, not the pstack plugin · Playbook header + Orchestrator/Worker split, exit 6 · port all 11 playbooks before eval · verdict = journal event, no TSV · Codex now · reviewer read-only right after W1 · brief gains Scope + Acceptance · keep suhuynh as reviewer (accepted risk) · no orch.ts/store.ts, no standing-orders file, no swarm.

Order: PR1 → PR2 → PR3 → PR4 → PR5 → PR6 → PR7 → PR8 → PR9. Each one works on its own after merge.

---

## PR1 — Fix contract drift

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

## PR2 — Read-only reviewers on every harness

Goal: a review worker cannot edit the worktree on any harness.
Current: only claude gets `deny Edit(worktree)` + `dontAsk` (`launch.rs:98-150`) [F]. opencode/omp reviewers can write.
Scope: `crates/quaqua/src/worker/launch.rs`, `team.toml` comment ("claude read-only flags" is stale) [F]
Design:
- opencode: add a permission deny for edit/write to `OPENCODE_CONFIG_CONTENT` when role=review [A: confirm opencode config key]
- omp: find its read-only/tool-deny flag [A]. If none exists, refuse omp as a reviewer and error clearly, never silently allow writes.
Acceptance: unit test per harness asserting the review launch args contain the deny; spawn a review on opencode in a scratch worktree, the edit attempt is refused
Verify: `cargo test -p quaqua launch`
Unknown: omp capability → may shrink to "opencode + refuse omp"

## PR3 — Codex harness

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
Risk: adds a 5th branch to every `match harness`, which PR7 moves into `harness/` later (accepted)

## PR4 — Playbook contract

Goal: the CoS picks a playbook, the worker follows its steps, the engine checks it.
Scope: `worker/prompt/mod.rs` (`brief_headers_ok`, :41), `worker/spawn.rs:318-355` (rules concat), `watchdog.rs`, new `templates/workspace/playbooks/`
Design:
- Brief header `Playbook: <name>`, optional. `--check-brief` rejects unknown names.
- Playbook file has `## Orchestrator` (CoS/Lead) and `## Worker`. Spawn appends only `## Worker` to `runs/<name>.rules.md`.
- `playbooks/` is a separate dir because spawn's concat isn't recursive [F]. Read it explicitly, never via `rules/_all`.
- Report must contain `## Playbook` with `<step>: done` or `<step>: skip: <reason>`. Brief has a Playbook but the report lacks the section → exit 6.
Acceptance: tests for unknown name rejected, Worker-only injection, missing section → 6, `skip:` without reason → 6
Verify: `cargo test -p quaqua prompt watchdog spawn`
Forbidden: auto-classifying the playbook in the engine

## PR5 — Port 11 playbooks

Goal: bug-fix, feature, refactoring, perf-issue, investigation, prototype, hillclimb, runtime-forensics, trace-forensics, visual-parity, eval.
Rules for each port:
- drop Cursor, Task-tool, `/loop` and mono-harness skill references
- arena → N workers of different kinds + judge; architect → taskforce `dual-lens`; interrogate → reviewer on a different model
- visual-parity runs headless only (`no-gui-driving.md`) [F]
- eval reads `runs/` reports + telemetry, not `~/.cursor/projects/`
Acceptance: each file passes the PR4 parser; `grep -riE "cursor|Task tool|/loop|brainstorm|to-plan"` in playbooks/ returns nothing
Verify: PR4 test fixture loads all 11
Can split: 2–3 PRs per playbook group

## PR6 — Verdict in the journal

Goal: separate agent-done → verified → ship-ready, with verified tied to a SHA.
Scope: `engine/journal_sqlite.rs` (new event kind, no schema change: `events.kind` + `payload_json`) [F], `pipeline/mod.rs` (Verdict parse :2662), `status.rs`
Design:
- event `verdict {branch, sha, level, evidence, verifier}`; levels `unit-test-verified | type-check-only | review-clean | verifier-blocked | verifier-failed`
- report line `Verified: <level> @ <sha>`
- `quaqua status` shows the per-branch level; stale if head SHA ≠ verdict SHA (turns "new commit voids review" from prose into a check)
- ship-ready stays ní's approval; the engine only refuses to call a stale SHA "verified"
Acceptance: tests for record/read verdict, stale on new commit, status output
Verify: `cargo test -p quaqua journal status pipeline`

## PR7 — `harness/` module

Goal: one place per harness: launch args, rules channel, read-only mode, effort/variants, cost reader, quirks (claude trust dialog `spawn.rs:434`).
Scope: new `crates/quaqua/src/harness/{mod,claude,opencode,omp,codex}.rs`; remove the `match harness` sites in launch/config/auditor/usage/spawn
Design: a `Harness` trait or enum dispatch. Pure move, no behavior change.
Acceptance: the existing launch-arg snapshot tests are byte-identical before and after; `rg "match .*harness"` outside harness/ → 0
Verify: `cargo test -p quaqua`
Check `engine/OWNERS.md` for file ownership before moving code.

## PR8 — Brief: Scope + Acceptance

Goal: the brief states owned paths and checkable outcomes.
Scope: `rules/_all/brief.md`, `worker/prompt/mod.rs`
Design: add `Scope:` (owned / forbidden paths) and `Acceptance:` (checkable lines). `brief_headers_ok` warns, doesn't fail, for one cycle. Acceptance describes outcomes, not a solution, so it keeps the no-pre-solve rule.
Later (not this PR): `herdr-gate.sh` blocks writes outside Scope.
Verify: `cargo test -p quaqua prompt`

## PR9 — CoS queue via `quaqua watch`

Goal: worker completions/failures/blockers land in one queue; the CoS drains it at boundaries instead of waiting on each agent.
Base: `docs/plans/active/team-event-driven-monitor.md` (herdr 0.9.5 event bus, evtprobe 8/8) [F]
Design: `quaqua watch` subscribes and writes journal events; `quaqua status` / a drain command reads them. The report file stays the source of truth; idle ≠ done.
Forbidden: a daemon or scheduler beyond `watch`
Verify: replay recorded events in a test; one live run with 2 workers

---

Retry-by-mode table, stop-line and batched gates: prose edits to `templates/workspace/AGENTS.md`, fold into PR1 or PR4.
