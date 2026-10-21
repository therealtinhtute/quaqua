# pstack integration — PR specs

Source: brainstorm 2026-10-21 (pstack @ 190d287 vs quaqua main d24be71). Revised 2026-10-21 against main c35511b (H1 + H2 merged; H3a + H4 in flight).
Status: **draft spec** (chưa giao worker) · spec only, nothing implemented. [F] = verified in repo/CLI, [A] = assumption the PR must confirm first.

Scope: only ideas ported from [pstack](https://github.com/cursor/plugins/tree/main/pstack). Non-pstack work (drift, read-only reviewers, pi, `harness/`, Codex) lives in `harness-hygiene.md` + `pi-harness.md`.

Locked decisions: port ideas, not the pstack plugin · Playbook header + Orchestrator/Worker split, exit 6 · port all 11 playbooks before eval · verdict = journal event, no TSV · brief gains Scope + Acceptance, in the same PR as the Playbook header · no orch.ts/store.ts, no standing-orders file, no swarm.

| PR | From pstack |
|---|---|
| P1 Brief contract: Playbook + Scope + Acceptance | poteto-mode (pick a playbook, steps as todos, `done` / `skip: <reason>`) + orchestrate brief template (SCOPE, ACCEPTANCE) |
| P2 Port 11 playbooks | poteto-mode playbooks |
| P3 Verdict in journal | orch verdict ledger keyed by SHA, prove-it-works levels |
| P5 CoS queue | orchestrate: completions are queue events, not interrupts |

P4 is folded into P1: both change the brief headers in `brief_headers_ok` (`worker/prompt/mod.rs`), so two PRs would conflict on one function.

## Order and lanes

Prerequisites: H1 (one handback format) [F merged #95], H2 (harness registry) [F merged #96], H4 (review baseline SHA) before P3.

| Lane | PR | Starts from | Waits for |
|---|---|---|---|
| A | P1 | main | nothing |
| B | P3 | main | H4 merged |
| C | P5 | main | nothing |
| D | P2 | main | P1 merged (the parser loads the playbooks) |

A, B and C can run in parallel. Shared files: A and B both touch `worker/prompt/`; B and C both touch `status.rs` and the journal. The second to merge rebases.

Risk to test first: the exit-6 check assumes workers reliably write `## Playbook` with every step id. Before the check applies to the whole team, run P1 on one real pi worker (`opencode-go/deepseek-v4.1-flash`) and one claude worker. A model that keeps dropping the section turns exit 6 into churn.

---

## P1 — Brief contract: Playbook + Scope + Acceptance

Goal: the CoS picks a playbook and states owned paths and checkable outcomes; the worker follows the steps; the engine checks the report.
Scope: `worker/prompt/mod.rs` (`brief_headers_ok`, :41; submit path), `worker/prompt/watchdog.rs` (`validate_report_content`, :57-75) and its callers (`workflow.rs:467`, `mod.rs:956`, `cost.rs:483,488,511`), `rules/_all/brief.md`, new `templates/workspace/playbooks/` (fixture playbook only; real ones in P2). Line numbers are from d24be71: re-locate them after H2.
Design, playbook:
- Brief header `Playbook: <name>`, optional. `--check-brief` rejects unknown names.
- Playbook file has `## Orchestrator` (CoS/Lead) and `## Worker`; Worker steps are a numbered list, the number is the step id.
- Injection point is **`worker prompt`, not spawn**: spawn writes `runs/<name>.rules.md` before any brief exists (`spawn.rs:321-354`; `SpawnOpts` has no brief, `spawn.rs:43-49`) and each harness reads that file once at launch (`launch.rs:105,118-119,145-146`) [F]. `worker prompt` appends the `## Worker` section to the brief it submits. Never via `rules/_all` (`playbooks/` stays outside it; spawn's concat isn't recursive [F]).
- `worker prompt` persists the playbook name in `runs/.<name>.playbook` (like `.budget`), so `--resume` validates without the brief.
- Builder report (not reviewer: `expect_verdict = false`) must contain `## Playbook` with one line per step id: `<n>: done` or `<n>: skip: <reason>`. Brief has a Playbook and the section is missing, a step id is missing, or `skip:` has no reason → exit 6 (resumable, like today's Lessons check).
- `validate_report_content` gains the expected step ids (`None` = no playbook = today's behavior); all callers pass it.
Design, Scope + Acceptance:
- New headers `Scope:` (owned / forbidden paths) and `Acceptance:` (checkable lines). `brief_headers_ok` warns, doesn't fail, for one cycle; a later PR makes them required.
- Acceptance describes outcomes, not a solution, so it keeps the no-pre-solve rule.
- Later (not this PR): `herdr-gate.sh` blocks writes outside Scope.
Prose (same PR): retry-by-mode table, stop-line and batched gates in `templates/workspace/AGENTS.md`.
Acceptance: tests for unknown playbook rejected, Worker-only text in the submitted prompt (no Orchestrator text), missing section → 6, missing step → 6, `skip:` without reason → 6, `--resume` keeps the check, report without Playbook brief unchanged, brief without Scope/Acceptance warns but passes
Verify: `cargo test --locked -p quaqua prompt watchdog`
Forbidden: auto-classifying the playbook in the engine

## P2 — Port 11 playbooks

Goal: bug-fix, feature, refactoring, perf-issue, investigation, prototype, hillclimb, runtime-forensics, trace-forensics, visual-parity, eval (11 of pstack's 23 files in `skills/poteto-mode/playbooks/`) [F]. All 11 land before eval is used.
pstack playbooks have no Orchestrator/Worker split: one `### Title`, a lead paragraph, numbered steps that mix lead and subagent work (`bug-fix.md`: "You own this task… Delegate investigation and the fix to subagents") [F]. Each port is an authored split, not a copy. Local source for the brief: a pstack checkout at 190d287 (`git clone https://github.com/cursor/plugins` → `pstack/skills/poteto-mode/playbooks/`).
Rules for each port:
- drop Cursor, Task-tool, `/loop` and mono-harness skill references
- arena → N workers of different kinds + judge; architect → taskforce `dual-lens`; interrogate → reviewer on a different model
- visual-parity runs headless only (`no-gui-driving.md`) [F]
- eval reads `runs/` reports + telemetry, not `~/.cursor/projects/`
Split into 3 PRs, so each review stays small:
- P2a — code: bug-fix, feature, refactoring, perf-issue
- P2b — explore: investigation, prototype, hillclimb
- P2c — forensics + eval: runtime-forensics, trace-forensics, visual-parity, eval
P2a, P2b and P2c touch disjoint files and can run in parallel once P1 is merged.
Acceptance: each file passes the P1 parser; `grep -riE "cursor|Task tool|/loop|brainstorm|to-plan"` in playbooks/ returns nothing
Verify: P1 test fixture loads every playbook in the directory

## P3 — Verdict in the journal

Goal: separate agent-done → verified → ship-ready, with verified tied to a SHA.
Depends on H4: H4 records the worktree HEAD + `git status --porcelain` at review spawn and voids a verdict whose tree changed. P3 reuses that recorded SHA as the verdict's `sha`; it does not compute its own. One place decides "stale".
Scope: `engine/journal_sqlite.rs` (new event kind, no schema change: `events.kind` + `payload_json`) [F], `pipeline/mod.rs` (Verdict parse, :2662 at 0fc0ad8) [F], `status.rs`
Design:
- event `verdict {branch, sha, level, evidence, verifier}`; levels `unit-test-verified | type-check-only | review-clean | verifier-blocked | verifier-failed`
- report line `Verified: <level>  <sha>`
- `quaqua status` shows the per-branch level; stale if head SHA ≠ verdict SHA (turns "new commit voids review" from prose into a check)
- ship-ready stays anh iu's approval; the engine only refuses to call a stale SHA "verified"
Acceptance: tests for record/read verdict, stale on new commit, status output, a verdict H4 voided is never recorded as verified
Verify: `cargo test -p quaqua journal status pipeline`

## P5 — CoS queue via `quaqua watch`

Goal: worker completions/failures/blockers land in one queue; the CoS drains it at boundaries instead of waiting on each agent.
Already on main [F]: `quaqua status` with `DONE-NOT-CLEANED` / `IDLE-UNPROMPTED` / `ORPHAN` (`status.rs:50-52`) and `--metrics` (#93); the per-worker watchdog blocks on `herdr agent wait` (`herdr.rs` `agent_wait`, #90). Missing [F]: no `watch` subcommand exists.
herdr contract (from the evtprobe run, `~/quaqua/raw/2026-10-17-tieude-evtprobe.md`, workspace-private, 8/8 transitions delivered, latency ≤1.6s):
- subscriptions are per pane, no wildcard, no snapshot on connect, one subscribe per connection → pin the pane set, read state once at connect, reconnect and re-sync when topology changes (`pane.agent_detected`)
- `idle` at the end of a turn with a background shell is a real idle → a status event is never proof of completion; `runs/<name>.md` stays the source of truth
Design:
- `quaqua watch <names…> [--timeout]` reuses `Herdr::agent_wait` per name (one thread each) [A: `herdr agent wait` takes one name]. On `idle | done | blocked` or pane exit it checks `runs/<name>.md`, appends a journal event `worker_signal {name, status, report: present|missing}`, prints one line and exits. The CoS runs it in the background and is woken instead of sleep-polling.
- `quaqua status` shows journal signals not yet drained; the CoS drains them at boundaries. The report file stays the source of truth; idle ≠ done.
Acceptance: replayed event fixture → correct one-line output and journal event; idle with no report → `report: missing`, not done; socket drop → reconnect + re-sync
Verify: `cargo test -p quaqua watch status`; one live run with 2 workers
Forbidden: a daemon or scheduler beyond `watch`
