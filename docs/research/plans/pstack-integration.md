# pstack integration — PR specs

Source: brainstorm 2026-10-21 (pstack @ 190d287, plugin 0.15.29, vs quaqua main d24be71). Revised 2026-10-21 against main 309f698 (H1 + H2 merged; H3a + H4 in flight). v3 adds the router, figure-it-out, session lifecycle, and the orchestrate rules found on a second read.
Status: **draft spec** (chưa giao worker) · spec only, nothing implemented. [F] = verified in repo/CLI or the pstack source, [A] = assumption the PR must confirm first.

Scope: only ideas ported from [pstack](https://github.com/cursor/plugins/tree/main/pstack). Non-pstack work (drift, read-only reviewers, pi, `harness/`, Codex) lives in `harness-hygiene.md` + `pi-harness.md`.

Locked decisions: port ideas, not the pstack plugin · router = playbook data + `quaqua playbook list|show` + a `/poteto-mode` skill · Playbook header + Orchestrator/Worker split, exit 6 · port all 14 playbooks before eval · verdict = journal event, no TSV · brief gains Scope, Acceptance, Verify, Context in the same PR as the Playbook header, each warning for one cycle · no orch.ts/store.ts, no standing-orders file, no swarm.

| PR | From pstack |
|---|---|
| P1 Router + brief contract | poteto-mode (route the goal to a playbook, steps as todos, `done` / `skip: <reason>`), orchestrate brief template (SCOPE, CONTEXT, ACCEPTANCE, VERIFY), orchestrate coordinator rules, `docs/harness/claude-code.md` (one mapping per harness, `Skipped in <harness>` line) |
| P2 Port 14 playbooks | poteto-mode playbooks + figure-it-out |
| P3 Verdict in journal | orch verdict ledger keyed by SHA, worker self-report vs verifier override, prove-it-works levels |
| P5 CoS queue | orchestrate: completions are queue events, drain in batches, gates with a default |

P4 is folded into P1: both change the brief headers in `brief_headers_ok` (`worker/prompt/mod.rs`), so two PRs would conflict on one function.

## Order and lanes

Prerequisites: H1 (one handback format) [F merged #95], H2 (harness registry) [F merged #96], H4 (review baseline SHA) before P3.

| Lane | PR | Starts from | Waits for |
|---|---|---|---|
| A | P1 | main | nothing |
| B | P3 | main | H4 merged |
| C | P5 | main | nothing |
| D | P2a/b/c | main | P1 merged (the parser loads the playbooks) |

A, B and C can run in parallel. Shared files: A and B both touch `worker/prompt/`; B and C both touch `status.rs` and the journal. The second to merge rebases.

Risk to test first: the exit-6 check assumes workers reliably write `## Playbook` with every step id. Before the check applies to the whole team, run P1 on one real pi worker (`opencode-go/deepseek-v4.1-flash`) and one claude worker. A model that keeps dropping the section turns exit 6 into churn. This is pstack's "pilot one unit through the whole path before fan-out" applied to P1 itself.

---

## P1 — Router + brief contract

Goal: the CoS routes every task through one front door, picks a playbook, and writes a brief a stranger could execute; the worker follows the steps; the engine checks the report.
Scope: `worker/prompt/mod.rs` (`brief_headers_ok`, :41; submit path), `worker/prompt/watchdog.rs` (`validate_report_content`, :57-75) and its callers (`workflow.rs:467`, `mod.rs:956`, `cost.rs:483,488,511`), `rules/_all/brief.md`, `templates/workspace/AGENTS.md`, new `templates/workspace/playbooks/` (fixture playbook only; real ones in P2), new `templates/workspace/.claude/skills/poteto-mode/SKILL.md`, new `playbook` subcommand. Line numbers are from d24be71: re-locate them after H2.

Design, router (anh iu: data + CLI, plus a `poteto-mode` skill that narrows what the agent may call):
- Each playbook file starts with frontmatter `name:` + `when:` (one sentence, the routing trigger). The index is generated from these files, never hand-written, so it cannot drift from `playbooks/`. pstack writes its index by hand in `poteto-mode/SKILL.md` § Playbooks [F]; this is the one place quaqua departs from it.
- `quaqua playbook list` prints `<name> — <when>` per file. `quaqua playbook show <name>` prints the `## Orchestrator` section for the CoS. Reuses the P1 parser.
- No playbook fits, or the work is large and cross-cutting → `figure-it-out` (ported in P2a), never a blank. pstack routes there for "no bundled playbook fits" [F]. `Playbook: none, <reason>` stays only for trivial one-command units.
- Skill `templates/workspace/.claude/skills/poteto-mode/SKILL.md`, the only routing entry:
  - `/poteto-mode <goal>` from anh iu, or the CoS on any new task. Steps: run `quaqua playbook list`, pick one name, run `quaqua playbook show <name>`, write the brief (all headers below), spawn per `models.md`.
  - `allowed-tools` limits the skill to `Bash(quaqua playbook *)`, `Bash(quaqua worker *)`, `Read`, so a routing turn cannot wander into edits or other skills [A: confirm the `allowed-tools` frontmatter key in the installed Claude Code].
  - Never invent a playbook name; never route inside a worker. A worker that loads it stops: its playbook is already fixed in `runs/.<name>.playbook` and it follows only the `## Worker` steps in its prompt.
  - pstack's other skills (how, architect, swarm, interrogate…) are not bundled; each playbook names the quaqua equivalent (P2 rules).
- `templates/workspace/AGENTS.md`: one line, "Before every brief, route through `/poteto-mode`."

Design, playbook contract:
- Brief header `Playbook: <name>`. `--check-brief` rejects unknown names; a missing header warns for one cycle, as does `none` without a reason.
- Playbook file has `## Orchestrator` (CoS/Lead) and `## Worker`; Worker steps are a numbered list, the number is the step id.
- Injection point is **`worker prompt`, not spawn**: spawn writes `runs/<name>.rules.md` before any brief exists (`spawn.rs:321-354`; `SpawnOpts` has no brief, `spawn.rs:43-49`) and each harness reads that file once at launch (`launch.rs:105,118-119,145-146`) [F]. `worker prompt` appends the `## Worker` section to the brief it submits. Never via `rules/_all` (`playbooks/` stays outside it; spawn's concat isn't recursive [F]).
- `worker prompt` persists the playbook name in `runs/.<name>.playbook` (like `.budget`), so `--resume` validates without the brief.
- Builder report (not reviewer: `expect_verdict = false`) must contain `## Playbook` with one line per step id: `<n>: done` or `<n>: skip: <reason>`. Brief has a Playbook and the section is missing, a step id is missing, or `skip:` has no reason → exit 6 (resumable, like today's Lessons check).
- `validate_report_content` gains the expected step ids (`None` = no playbook = today's behavior); all callers pass it.

Design, harness skip mapping (from pstack `docs/harness/claude-code.md`: one mapping file per harness, and an unsupported step becomes one `Skipped in Claude Code: <step>. <reason>.` line, then the next step, no stop, no ask) [F]:
- A playbook step the worker's harness cannot do is reported as `<n>: skip: unsupported on <harness>, <reason>`; it is a valid skip, not exit 6.
- One short table per harness in `templates/workspace/playbooks/HARNESS.md`: step concepts (read-only check, run targeted test, open PR, headless UI) → the command or flag for claude / omp / opencode / pi. Row keys come from the H2 registry's harness names, so a new harness without a row shows up in a test.
- `worker prompt` appends only the worker's own harness rows next to the `## Worker` steps.

Design, brief headers (pstack's 9-field brief: GOAL, SCOPE, CONTEXT, ACCEPTANCE, VERIFY, TIMEBOX, FORBIDDEN, REPORT, STANDING; quaqua already has Goal, Hard constraints ≈ FORBIDDEN, Evidence ≈ REPORT, `.budget` ≈ TIMEBOX) [F]:
- New headers, each warning (not failing) for one cycle; a later PR makes them required:
  - `Scope:` owned / forbidden paths.
  - `Acceptance:` checkable lines, outcomes not a solution, so it keeps the no-pre-solve rule.
  - `Verify:` the exact command(s), plus known gotchas.
  - `Context:` file / PR pointers; when this unit depends on another, that unit's report pasted in full. Workers cannot read sibling reports, so a dependency is a context relay, not just ordering.
- Size the brief to the unit: a one-command unit collapses to one paragraph that still names goal, scope, the verify command and the report shape. A full template around a two-line edit costs more to write and obey than the edit.
- Later (not this PR): `herdr-gate.sh` blocks writes outside Scope.

Prose (same PR), coordinator rules in `templates/workspace/AGENTS.md`, from pstack `orchestrate.md` [F]:
- Retry by mode: cap-hit / oom → respawn with smaller scope; network drop → retry as-is; tool error → retry on a different model; unknown → retry once. Two retries, then abandon the unit and replan.
- Stop line: when more spawning would produce garbage tree-wide (bad upstream output, broken acceptance, dead infra), write a stop line, let in-flight work finish, fix the cause, clear it.
- Budget: at about 70% of the wall-clock or cost budget, stop spawning and land what is verified.
- Pilot: on a new unit shape, push one unit through brief → worker → verify → merge before any fan-out; fix the brief from that evidence.
- Liveness: never prompt or resume a worker just to check on it; read `quaqua status`, the journal, git and `runs/` instead.
- Rollup: at each wave boundary, account for every spawned worker (arrived, respawned, or scope absorbed); never silently redo a missing worker's job.
- Fresh over resumed: a fix round or follow-up gets a fresh worker with consolidated scope (original brief + later directives + prior report), unless it needs state only the old worker has (uncommitted changes, a running process).

Acceptance: tests for unknown playbook rejected, Worker-only text in the submitted prompt (no Orchestrator text), only the worker's own harness rows appended, missing section → 6, missing step → 6, `skip:` without reason → 6, `skip: unsupported on <harness>` accepted, `--resume` keeps the check, report without Playbook brief unchanged, brief missing Playbook / Scope / Acceptance / Verify / Context warns but passes, `playbook list` built from the fixture frontmatter, `playbook show` prints only the Orchestrator section, every H2 harness has a row in `HARNESS.md`
Verify: `cargo test --locked -p quaqua prompt watchdog playbook`
Forbidden: auto-classifying the playbook in the engine (the CoS picks; the engine only lists and checks)

## P2 — Port 14 playbooks

Goal: 14 playbooks in `templates/workspace/playbooks/`, all landed before eval is used:
- 11 of pstack's 23 files in `skills/poteto-mode/playbooks/` [F]: bug-fix, feature, refactoring, perf-issue, investigation, prototype, hillclimb, runtime-forensics, trace-forensics, visual-parity, eval.
- figure-it-out, from `skills/figure-it-out/SKILL.md` [F]: the router's target when nothing fits. Frame done as a falsifiable predicate, pick a rigor level, split into independently landable units riskiest-first, build the check before the work, run each unit as hypothesis → smallest change → measure → keep or revert, verdict `VERIFIED | NOT VERIFIED | INCONCLUSIVE` (inconclusive is not a pass).
- session-pickup, from `playbooks/session-pickup.md` [F]: resume a prior worker or CoS session from its report, branch and journal; the prior trail is authoritative, do not redo completed work; then route the rest to the matching playbook.
- pause-safely, from `playbooks/pause-safely.md` [F]: stop at a safe boundary, no irreversible action, one `wip:` commit, a resume note off-context (`runs/<name>.resume.md`). Triggered on an explicit pause or before a CoS compaction, never on "keep going".

pstack playbooks have no Orchestrator/Worker split: one `### Title`, a lead paragraph, numbered steps that mix lead and subagent work (`bug-fix.md`: "You own this task… Delegate investigation and the fix to subagents") [F]. Each port is an authored split, not a copy. session-pickup and pause-safely may be Orchestrator-only (empty `## Worker`) when the CoS is the one resuming or pausing. Local source for the brief: a pstack checkout at 190d287 (`git clone https://github.com/cursor/plugins` → `pstack/skills/`).
Rules for each port:
- drop Cursor, Task-tool, `/loop` and mono-harness skill references
- arena → N workers of different kinds + judge; architect → taskforce `dual-lens`; interrogate → reviewer on a different model
- visual-parity runs headless only (`no-gui-driving.md`) [F]
- eval reads `runs/` reports + telemetry, not `~/.cursor/projects/`; it also reports the share of briefs routed to `none` and `figure-it-out`, so a router the CoS skips shows up as a number
- each port carries frontmatter `name:` + `when:`; the `when:` line is adapted from the playbook's one-line entry in pstack's `poteto-mode/SKILL.md` § Playbooks
Split into 3 PRs, so each review stays small:
- P2a — code + fallback: bug-fix, feature, refactoring, perf-issue, figure-it-out
- P2b — explore + lifecycle: investigation, prototype, hillclimb, session-pickup, pause-safely
- P2c — forensics + eval: runtime-forensics, trace-forensics, visual-parity, eval
P2a, P2b and P2c touch disjoint files and can run in parallel once P1 is merged.
Acceptance: each file passes the P1 parser; `quaqua playbook list` shows all 14; `grep -riE "cursor|Task tool|/loop|brainstorm|to-plan"` in playbooks/ returns nothing
Verify: P1 test fixture loads every playbook in the directory

## P3 — Verdict in the journal

Goal: separate agent-done → verified → ship-ready, with verified tied to a SHA.
Depends on H4: H4 records the worktree HEAD + `git status --porcelain` at review spawn and voids a verdict whose tree changed. P3 reuses that recorded SHA as the verdict's `sha`; it does not compute its own. One place decides "stale".
Scope: `engine/journal_sqlite.rs` (new event kind, no schema change: `events.kind` + `payload_json`) [F], `pipeline/mod.rs` (Verdict parse, :2662 at 0fc0ad8) [F], `status.rs`
Design (pstack orchestrate § Verification) [F]:
- event `verdict {branch, sha, level, evidence, verifier, source: self|verifier}`; levels `unit-test-verified | type-check-only | review-clean | inconclusive | verifier-blocked | verifier-failed`
- a builder may self-report; a reviewer verdict on the same branch + SHA overrides it. `quaqua status` shows the strongest source for the current SHA.
- `inconclusive` and `verifier-blocked` are never a pass; `verifier-failed` calls for a fix unit, not a re-review
- CI green is evidence inside a verdict, not a verdict
- report line `Verified: <level>  <sha>`
- `quaqua status` shows the per-branch level; stale if head SHA ≠ verdict SHA (turns "new commit voids review" from prose into a check)
- ship-ready stays anh iu's approval; the engine only refuses to call a stale SHA "verified"
Acceptance: tests for record/read verdict, stale on new commit, status output, reviewer overrides self-report on the same SHA, `inconclusive` not shown as verified, a verdict H4 voided is never recorded as verified
Verify: `cargo test -p quaqua journal status pipeline`

## P5 — CoS queue via `quaqua watch`

Goal: worker completions/failures/blockers land in one queue; the CoS drains it at boundaries instead of waiting on each agent, and asks anh iu only through batched gates.
Already on main [F]: `quaqua status` with `DONE-NOT-CLEANED` / `IDLE-UNPROMPTED` / `ORPHAN` (`status.rs:50-52`) and `--metrics` (#93); the per-worker watchdog blocks on `herdr agent wait` (`herdr.rs` `agent_wait`, #90). Missing [F]: no `watch` subcommand exists.
herdr contract (from the evtprobe run, `~/quaqua/raw/2026-10-17-tieude-evtprobe.md`, workspace-private, 8/8 transitions delivered, latency ≤1.6s):
- subscriptions are per pane, no wildcard, no snapshot on connect, one subscribe per connection → pin the pane set, read state once at connect, reconnect and re-sync when topology changes (`pane.agent_detected`)
- `idle` at the end of a turn with a background shell is a real idle → a status event is never proof of completion; `runs/<name>.md` stays the source of truth
Design:
- `quaqua watch <names…> [--timeout]` reuses `Herdr::agent_wait` per name (one thread each) [A: `herdr agent wait` takes one name]. On `idle | done | blocked` or pane exit it checks `runs/<name>.md`, appends a journal event `worker_signal {name, status, report: present|missing}`, prints one line and exits. The CoS runs it in the background and is woken instead of sleep-polling.
- Drain discipline (pstack orchestrate § Queue and drain) [F]: a signal is recorded and the CoS returns to what it was doing; it never reviews a diff inside a drain (a review is its own reviewer unit). The CoS drains at four points: after a critical section (writing a brief, a merge decision), at a wave boundary, on a `watch` wake, and before reporting to anh iu.
- `quaqua status --drain` lists undrained signals, marks them drained, and ends with three lines: counts per state, what changed since the last drain, open gates.
- Gates: a question for anh iu is a journal event `gate {question, options, default}`, shown in the drain's last line and batched into the next report, never asked one by one. The CoS routes work around an open gate. Irreversible actions (merge to main, push, branch delete, `herdr integration install`) are always gates with no default.
Acceptance: replayed event fixture → correct one-line output and journal event; idle with no report → `report: missing`, not done; socket drop → reconnect + re-sync; `--drain` output ends with the three lines; a gate without `default` is rejected unless it is on the irreversible list
Verify: `cargo test -p quaqua watch status`; one live run with 2 workers
Forbidden: a daemon or scheduler beyond `watch`
