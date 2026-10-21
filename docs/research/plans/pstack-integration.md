# pstack integration — PR specs

Source: brainstorm 2026-10-21 (pstack @ 190d287 vs quaqua main d24be71).
Status: **draft spec** (chưa giao worker) · spec only, nothing implemented. [F] = verified in repo/CLI, [A] = assumption the PR must confirm first.

Scope: only ideas ported from [pstack](https://github.com/cursor/plugins/tree/main/pstack). Non-pstack work (drift, read-only reviewers, Codex, `harness/`) lives in `harness-hygiene.md`.

Locked decisions: port ideas, not the pstack plugin · Playbook header + Orchestrator/Worker split, exit 6 · port all 11 playbooks before eval · verdict = journal event, no TSV · brief gains Scope + Acceptance · no orch.ts/store.ts, no standing-orders file, no swarm.

| PR | From pstack |
|---|---|
| P1 Playbook contract | poteto-mode: pick a playbook, steps as todos, `done` / `skip: <reason>` |
| P2 Port 11 playbooks | poteto-mode playbooks |
| P3 Verdict in journal | orch verdict ledger keyed by SHA, prove-it-works levels |
| P4 Brief Scope + Acceptance | orchestrate brief template (SCOPE, ACCEPTANCE) |
| P5 CoS queue | orchestrate: completions are queue events, not interrupts |

Order: P1 → P2 → P3 → P4 → P5. Dependency: `harness-hygiene.md` H1 (one handback format) merges before P1, because P1 adds `## Playbook` to that report.

---

## P1 — Playbook contract

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

## P2 — Port 11 playbooks

Goal: bug-fix, feature, refactoring, perf-issue, investigation, prototype, hillclimb, runtime-forensics, trace-forensics, visual-parity, eval.
Rules for each port:
- drop Cursor, Task-tool, `/loop` and mono-harness skill references
- arena → N workers of different kinds + judge; architect → taskforce `dual-lens`; interrogate → reviewer on a different model
- visual-parity runs headless only (`no-gui-driving.md`) [F]
- eval reads `runs/` reports + telemetry, not `~/.cursor/projects/`
Acceptance: each file passes the P1 parser; `grep -riE "cursor|Task tool|/loop|brainstorm|to-plan"` in playbooks/ returns nothing
Verify: P1 test fixture loads all 11
Can split: 2–3 PRs per playbook group

## P3 — Verdict in the journal

Goal: separate agent-done → verified → ship-ready, with verified tied to a SHA.
Scope: `engine/journal_sqlite.rs` (new event kind, no schema change: `events.kind` + `payload_json`) [F], `pipeline/mod.rs` (Verdict parse :2662), `status.rs`
Design:
- event `verdict {branch, sha, level, evidence, verifier}`; levels `unit-test-verified | type-check-only | review-clean | verifier-blocked | verifier-failed`
- report line `Verified: <level>  <sha>`
- `quaqua status` shows the per-branch level; stale if head SHA ≠ verdict SHA (turns "new commit voids review" from prose into a check)
- ship-ready stays ní's approval; the engine only refuses to call a stale SHA "verified"
Acceptance: tests for record/read verdict, stale on new commit, status output
Verify: `cargo test -p quaqua journal status pipeline`

## P4 — Brief: Scope + Acceptance

Goal: the brief states owned paths and checkable outcomes.
Scope: `rules/_all/brief.md`, `worker/prompt/mod.rs`
Design: add `Scope:` (owned / forbidden paths) and `Acceptance:` (checkable lines). `brief_headers_ok` warns, doesn't fail, for one cycle. Acceptance describes outcomes, not a solution, so it keeps the no-pre-solve rule.
Later (not this PR): `herdr-gate.sh` blocks writes outside Scope.
Verify: `cargo test -p quaqua prompt`

## P5 — CoS queue via `quaqua watch`

Goal: worker completions/failures/blockers land in one queue; the CoS drains it at boundaries instead of waiting on each agent.
Base: `docs/plans/active/team-event-driven-monitor.md` (herdr 0.9.5 event bus, evtprobe 8/8) [F]
Design: `quaqua watch` subscribes and writes journal events; `quaqua status` / a drain command reads them. The report file stays the source of truth; idle ≠ done.
Forbidden: a daemon or scheduler beyond `watch`
Verify: replay recorded events in a test; one live run with 2 workers

---

Retry-by-mode table, stop-line and batched gates: prose edits to `templates/workspace/AGENTS.md`, fold into P1.
