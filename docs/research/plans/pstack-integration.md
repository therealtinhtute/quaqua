# pstack integration — PR specs

Source: review session 2026-10-21 (pstack @ 190d287, plugin 0.15.29, vs quaqua main d24be71). Revised 2026-10-21 against main 309f698 (H1 + H2 merged; H3a + H4 in flight). v3 adds the router, figure-it-out, session lifecycle, and the orchestrate rules found on a second read. v3.1 (2026-10-21, main dd01e68, H3a + H4 merged): P1 split into P1a/P1b, coordinator-never-edits-code rule, merge warns without a verdict at head, fixed Verify lines, and an evidence audit (§ Audit). v3.2 (2026-10-22): non-Claude second pass on § Audit (`runs/suhuynh-pstackfit.md`), counts redone with recorded filters, exit 6 starts as a warning, 24 principles as cited leaves, 18 playbooks in P2a–d, the SLP team mapping, P3 first. v3.3 (2026-10-22, main dd01e68): applies the v3.2 review (`~/quaqua/docs/plans/pstack-integration-review.md`, workspace-private): an MVP of steps S0–S8, success metrics measured before building, idempotency rules, deterministic Phase 2 triggers; the full port starts right after S6. v3.4 (2026-10-23): pilot slimmed to ≥5 units/lane (anh iu).
Status: **in progress** (2026-10-23) · S1–S7 + P2 (18 playbooks, 24 principles, `/poteto-mode`) merged on main (#101 merged); S8 pilot running; P5 `watch`/drain still deferred to its trigger. [F] = verified in repo/CLI or the pstack source, [A] = assumption the step must confirm first.

Scope: only ideas ported from [pstack](https://github.com/cursor/plugins/tree/main/pstack): its skills, principles and playbooks. Nothing from other skill sets. Non-pstack work (drift, read-only reviewers, pi, `harness/`, Codex) lives in `harness-hygiene.md` + `pi-harness.md`.

Locked decisions: workflow-first, not agent-first: the playbook is picked before the agent, and the harness is an adapter · port ideas, not the pstack plugin · router = playbook data + `quaqua playbook list|show` + a `/poteto-mode` skill that is only the front door (S6b) · Playbook header + Orchestrator/Worker split · **everything warns in the MVP**: no new exit code, deny or hard gate; each becomes a gate only on its § Phase 2 trigger · MVP = 3 playbooks (bug-fix, feature, figure-it-out) + 6 leaves; **the other 15 playbooks and 18 leaves are ported right after S6, in parallel with the pilot**, not on a trigger · a Worker section cites at most 3 leaves besides `principle-prove-it-works` · verdict = journal event, no TSV · brief gains only `Playbook:` and `Context:`; Scope, Acceptance and Verify fold into the existing Hard constraints and Evidence headers · verifier model family: record kinds + a `same-harness` flag, no family table yet · P5 `watch`/drain deferred to its trigger · playbooks never point at skills outside pstack, so figure-it-out is ported in full · coordinator = CoS, or the Lead inside an SLP team (§ SLP team) · no orch.ts/store.ts, no standing-orders file, no swarm.

| Step | From pstack |
|---|---|
| S1–S2 Verdict in journal | orch verdict ledger keyed by SHA, worker self-report vs verifier override, prove-it-works levels |
| S3–S5 Router engine + brief contract | poteto-mode (playbook index, steps as todos, `done` / `skip: <reason>`, cited principles), orchestrate brief CONTEXT |
| S6 3 playbooks + coordinator rules | bug-fix, feature, figure-it-out, `principle-*`, orchestrate coordinator rules |
| S6b Router skill | poteto-mode as the one front door |
| P2 Full port | the other 15 playbooks + 18 leaves |

## Success metrics

Deterministic counts from the journal and `runs/`, measured in S0 before any build, re-measured after the pilot:
- **G1**: merges to main or `<slug>-integ` with no verdict at the head SHA. Target 0 unacknowledged. Baseline example: PR #42 merged at `b12f74f` while the last CoS test run was at `a7de35b`.
- **G2** (guardrail): false-block rate = exit-6 resumes / accepted reports. Must not rise above the S0 baseline. [A] whether the journal's `workflow_failed` payload carries the exit code; if not, count from watchdog stderr / reports.
- **G3**: `playbook_gap` events that the coordinator confirms hid real unfinished work (checked against the diff). This is why S5 exists; if G3 stays 0 after the pilot, S5's check is not paying off.

Deterministic counts miss nuance an LLM eval would catch, but they are reproducible and cost no tokens.

## Order and lanes

Prerequisites: H1 (one handback format) [F merged #95], H2 (harness registry) [F merged #96], H4 (review baseline: worktree HEAD + porcelain recorded at review spawn, `worker/prompt/readonly.rs:1-3`) [F merged #99].

| Step | Content | Files | Verify | Depends on |
|---|---|---|---|---|
| S0 | Baseline G1/G2, read-only | `raw/<date>-pstack-baseline.md` (workspace) | numbers + the commands that produced them | — |
| S1 | Record `verdict` (§ S1) | `worker/prompt/` (acceptance), `engine/journal_sqlite.rs` | `cargo test --locked -p quaqua verdict` | — |
| S2 | `status` per-branch verdict + stale + `same-harness`; `merge` prints and warns | `status.rs`, `merge/mod.rs` | `cargo test --locked -p quaqua status`, then `… merge` | S1 |
| S3 | `playbook` module + `playbook list|show` + 1 fixture (§ S3) | new `playbook/`, `main.rs` | `cargo test --locked -p quaqua playbook` | — |
| S4 | `--check-brief` + injection + `.playbook` (§ S4) | `worker/prompt/mod.rs`, `rules/_all/brief.md` | `cargo test --locked -p quaqua prompt` | S2, S3 |
| S5 | Warn-only report check + `playbook_gap` (§ S5) | `worker/prompt/` (one call site) | `cargo test --locked -p quaqua playbook_check` | S4 |
| S6 | 3 playbooks + 6 leaves + coordinator rules (§ S6) | `templates/workspace/playbooks/`, `principles/`, `AGENTS.md` | S3 fixture loads every file; `cargo test --locked -p quaqua --test init` | S3 |
| S6b | `/poteto-mode` skill (§ S6b) | `templates/workspace/.claude/skills/poteto-mode/`, `tests/init.rs` | `cargo test --locked -p quaqua --test init` | S6 |
| S7 | `status --metrics` counts verdict / `playbook_gap` / merge-without-verdict | `status.rs` | `cargo test --locked -p quaqua metrics` | S2, S5 |
| S8 | Pilot (§ Pilot) | — | § Phase 2 table filled with real numbers | S6b, S7 |
| P2 | Full port, 4 content PRs (§ P2) | `templates/workspace/playbooks/`, `principles/` | S3 fixture loads every file | S6; runs in parallel with S7–S8 |

Lanes, one owner per file area (AGENTS.md):
- A: S1 → S2 → S4 → S5, the only owner of `worker/prompt/`.
- B: S3 → S6 → S6b, then P2a → P2b → P2c → P2d, the only owner of `playbooks/` and `principles/`. P2 batches run one after another because they share those directories.
- S7 after A and B (it shares `status.rs` with S2, which has merged by then). S8 after S6b and S7.

Each step is one PR with ≤6 acceptance lines and one `cargo test` filter per run (`runs/ni-h3a.md`).

Risk to test first: report parsing has already produced false blocks: notes under `## Blockers` were read as real blockers (CoS log 2026-10-16 truonglao-intake, which cost a respawn; 2026-10-18 T1 bashclean [A: the 10-18 log is not on this machine]), and the 2026-10-22 blind-check advisor was flagged `malformed-report` for a missing `Verdict:` line although its report was complete. So every new check in the MVP warns, and the pilot (S8) decides each gate from counts.

## SLP team

Every rule below that says "coordinator" applies to the CoS on no-SLP work and to the Lead (`<slug>-lead`) inside an SLP team (`templates/workspace/AGENTS.md` § Roles). anh iu stays Principal and talks only to the CoS. In the MVP the CoS is the only coordinator that routes; Lead routing per Peer is Phase 2.

| Concept | No SLP (CoS → worker) | SLP team (CoS → Lead → ≤4 Peers) |
|---|---|---|
| Route `/poteto-mode` | CoS picks the playbook per unit | CoS picks once for the Lead brief (`figure-it-out` in the MVP, `multi-phase-plan` once P2a lands); Peer routing by the Lead is Phase 2 |
| Brief headers | CoS writes them | CoS writes the Lead brief; the Lead writes each Peer brief. `Context:` relay = an upstream Peer report pasted in full |
| `## Orchestrator` | CoS reads it | the Lead reads it |
| `## Worker` + `## Playbook` | worker | Peer |
| Verdict (S1–S2) | per branch, before `quaqua merge` to main | per Peer branch before the Lead merges it into `quaqua/<slug>-integ`, and on `<slug>-integ` before the Lead reports done |
| Background wait + drain points | CoS waits on workers | the Lead waits on its Peers; the CoS on the Lead and `runs/<slug>-lead.inbox.md` |
| Never edits code or config | CoS | the Lead too: a conflict on `<slug>-integ` is rework for a Peer |
| Attack the premise | CoS | the Lead, who sees a fix class repeat on `<slug>-integ` first |

---

## S1 — Verdict in the journal

Goal: separate agent-done → verified → ship-ready, with verified tied to a SHA.
Scope: `engine/journal_sqlite.rs` (new event kind, no schema change: `events.kind` + `payload_json`, `:59,133`) [F], the acceptance path in `worker/prompt/`.
Design (pstack orchestrate § Verification, § Roles; `principle-prove-it-works`) [F]:
- event `verdict {branch, sha, level, evidence, verifier, source: self|verifier, builder_kind, verifier_kind, harness}`; levels `unit-test-verified | type-check-only | review-clean | inconclusive | verifier-blocked | verifier-failed`, an enum. An unknown level is parsed as `inconclusive`.
- **Reviewer**: no new line. Reviewers already write `Verdict:` (`pipeline/mod.rs:2668`) [F]. `Verdict: clean` + `readonly::check` OK → `review-clean` at the H4 baseline SHA, `source: verifier`. A verdict H4 voided is never recorded as verified.
- **Builder**: report line `Verified: <level> <sha>`, `source: self`. At acceptance the engine reads `git rev-parse HEAD` in the worktree; if it differs from the reported `<sha>` → `inconclusive`. A self-report alone is never shown as verified.
- A reviewer verdict on the same branch + SHA overrides a self-report. `inconclusive` and `verifier-blocked` are never a pass; `verifier-failed` calls for a fix unit, not a re-review. CI green is evidence inside a verdict, not a verdict.
- Model family: config enforces only `builder_reviewer_differ = "harness"` (`config.rs:441-442`); `"kind"` passes validation but nothing enforces it, and there is no family table [F]. The event records `builder_kind`, `verifier_kind` and `harness`; S2 shows a `same-harness` flag. Reclassifying a same-family verdict as `self` waits for a family table (§ Phase 2).
- Idempotency: reads take the strongest verdict per (branch, SHA); before appending, check the key `(branch, sha, source, verifier)`, so a re-accept or `--resume` adds no duplicate.
- Journal unavailable (locked, missing, open failure) → warn and still accept the report. A verdict never turns into a block.
Acceptance: reviewer `Verdict: clean` + baseline → `review-clean` at the baseline SHA · builder SHA ≠ worktree HEAD → `inconclusive` · reviewer overrides self-report on the same SHA · double resume → one verdict per key · a locked or missing db does not fail acceptance · unknown level → `inconclusive`
Verify: `cargo test --locked -p quaqua verdict`

## S2 — Verdict in status and merge

Scope: `status.rs`, `merge/mod.rs`.
- `quaqua status` shows the strongest level per branch for its head SHA, with `stale` when head ≠ verdict SHA and `same-harness` when the verifier ran on the builder's harness. A rebase or force-update changes the SHA, so the verdict goes stale: intended.
- `quaqua merge` prints the head's strongest verdict and warns when no verdict matches the head SHA, for merges to main and Lead merges into `<slug>-integ`. Warn only: `run_check` passes when no check command is set (`merge/mod.rs:157-165`) [F], so the warning is the only signal, and merging to main stays anh iu's approval.
Acceptance: stale on a new commit · strongest per (branch, SHA) · `same-harness` flag shown · `merge` warns on a head with no verdict, exit code unchanged
Verify: `cargo test --locked -p quaqua status`, then `cargo test --locked -p quaqua merge`

## S3 — Playbook module

Scope: new `playbook/` module, `main.rs` (subcommand), one fixture playbook + one fixture leaf. Does not touch `worker/prompt/`, so it runs in parallel with S1.
- Each playbook file starts with frontmatter `name:` + `when:` (one sentence, the routing trigger). The index is generated from these files, never hand-written, so it cannot drift from `playbooks/`. pstack writes its index by hand in `poteto-mode/SKILL.md` § Playbooks [F]; this is the one place quaqua departs from it.
- A playbook has `## Orchestrator` (coordinator) and `## Worker`; Worker steps are a numbered list, the number is the step id.
- A Worker step cites a leaf as `principle-<name>`, resolved to `principles/<name>.md`. An unknown name fails the parser. A Worker section cites at most 3 leaves besides `principle-prove-it-works` (a test), which keeps `guard-the-context-window`.
- `quaqua playbook list` prints `<name> — <when>` per file; `quaqua playbook show <name>` prints only `## Orchestrator`.
- `playbooks/` and `principles/` stay outside `rules/_all` (spawn's concat isn't recursive [F]).
Acceptance: `list` built from fixture frontmatter · `show` prints only Orchestrator · step ids parsed · unknown `principle-<name>` fails · a 4th cited leaf fails
Verify: `cargo test --locked -p quaqua playbook`
Forbidden: auto-classifying the playbook in the engine (the coordinator picks; the engine only lists and checks)

## S4 — Brief contract + injection

Scope: `worker/prompt/mod.rs` (`brief_headers_ok`, submit path), `rules/_all/brief.md`. Line numbers in v3.2 were from d24be71: re-locate them on current main.
- Brief headers: add only `Playbook: <name>` and `Context:`. `--check-brief` warns (exit 0) when either is missing, or on `none` without a reason, and rejects an unknown playbook name. Briefs without them pass as today.
- `Context:` = file / PR pointers; when this unit depends on another, that unit's report pasted in full (a context relay, exempt from brief sizing). Evidence: 7 reports on an unreadable brief path (§ Audit F4).
- `brief.md` text only, no new header: Hard constraints gains "owned / forbidden paths"; Evidence gains "checkable lines + the exact verify command, one filter per run". If the pilot shows Evidence overloaded, split it (§ Phase 2).
- No playbook fits, or the work is large and cross-cutting → `figure-it-out`, never a blank. `Playbook: none, <reason>` only for trivial one-command units.
- Injection is in **`worker prompt`, not spawn**: spawn writes `runs/<name>.rules.md` before any brief exists (`spawn.rs:321-354`) and each harness reads it once at launch [F]. `worker prompt` appends the `## Worker` steps, each cited leaf once, and a pre-filled `## Playbook` block (`1: `, `2: ` …) the worker copies into its report.
- `runs/.<name>.playbook` stores the name **and the step ids**, so a playbook file edited between prompt and report still checks against the ids the worker got. A follow-up `worker prompt` overwrites it. A missing file means no check.
Acceptance: unknown playbook rejected · missing Playbook / Context warns, exit 0 · submitted prompt has Worker text, no Orchestrator text, each cited leaf once · pre-filled block present · `.playbook` holds name + ids · old brief unchanged
Verify: `cargo test --locked -p quaqua prompt`

## S5 — Report check, warn only

Scope: one new function in `worker/prompt/`, called once after acceptance. `validate_report_content` and its 5 callers stay untouched.
- Builder reports only (`expect_verdict = false`). Parses `1: done`, `1. done`, `1) done`, `- 1: done`, and the same for `skip: <reason>`, against the ids in `.playbook`.
- Missing `## Playbook`, a missing step id, or `skip:` without a reason → a warning plus `playbook_gap {name, missing, unreasoned}`. Since S8 (2026-10-23, Phase 2 trigger met) this is exit 6 in `worker prompt` — a report the worker fixes before `--resume` accepts it; `quaqua pipeline` still warns and leaves its exit untouched. A db failure only warns.
- A step the worker's harness cannot do is `<n>: skip: unsupported on <harness>, <reason>`, a valid skip.
Acceptance: four line formats parsed · missing section / step / reason → warning + event, exit 6 in `worker prompt`, exit unchanged in `pipeline` · `skip: unsupported on <harness>` accepted · report without `.playbook` behaves as today · `--resume` accepts the fixed report
Verify: `cargo test --locked -p quaqua playbook_check`

## S6 — 3 playbooks + coordinator rules

Scope: `templates/workspace/playbooks/` (bug-fix, feature, figure-it-out), `templates/workspace/principles/` (6 leaves), `templates/workspace/AGENTS.md`.

Playbooks, each an authored Orchestrator/Worker split, not a copy: pstack playbooks have one `### Title`, a lead paragraph, and numbered steps that mix lead and subagent work (`bug-fix.md`: "You own this task… Delegate investigation and the fix to subagents") [F]. Local source: a pstack checkout at 190d287 (`git clone https://github.com/cursor/plugins` → `pstack/skills/`).
- bug-fix, feature: from `skills/poteto-mode/playbooks/` [F].
- figure-it-out, from `skills/figure-it-out/SKILL.md` [F], ported in full: frame done as a falsifiable predicate, pick a rigor level, split into independently landable units riskiest-first, build the check before the work, run each unit as hypothesis → smallest change → measure → keep or revert, verdict `VERIFIED | NOT VERIFIED | INCONCLUSIVE` (inconclusive is not a pass). It is the router's fallback, so it is in the MVP, and it hands planning to no other skill.

Leaves (6), each keeping its rule, Why and When, minus TypeScript-only and Cursor references:

| Leaf | Cited by |
|---|---|
| prove-it-works | the last Worker step of every playbook |
| attack-the-premise | bug-fix, figure-it-out; coordinator rule |
| fix-root-causes | bug-fix |
| test-behavior-not-implementation | bug-fix, feature |
| make-operations-idempotent | feature |
| sequence-verifiable-units | figure-it-out |

Port rules, for S6 and P2:
- source only pstack: drop Cursor, Task-tool, `/loop` and every reference to a skill outside pstack
- the last `## Worker` step (unless Orchestrator-only) is "check every Evidence line, then write `Verified: <level> <sha>`", citing `principle-prove-it-works`
- frontmatter `name:` + `when:`, the `when:` adapted from the playbook's entry in pstack's `poteto-mode/SKILL.md` § Playbooks
- `grep -riE "cursor|Task tool|/loop|brainstorm|to-plan"` in `playbooks/` and `principles/` returns nothing

Coordinator rules in `templates/workspace/AGENTS.md`, from pstack `orchestrate.md` and the leaves [F]; each states it binds the CoS and a Lead:
- Route: before every brief, `quaqua playbook list`, then set `Playbook:` (S6b adds the skill as the front door).
- Coordinator never edits code or config: it writes briefs, reads reports, verifies and merges. Edits to `crates/`, `tests/`, `rules/`, `AGENTS.md`, `team.toml` and `models.md` are a worker unit, except a Lesson promotion at cleanup. A conflicted merge or rebase is a worker unit, which `quaqua merge` already hands back as rework (exit 4, `merge/mod.rs:288-294`) [F]. Evidence: the CoS resolved the W3 merge conflict itself (CoS log 2026-10-18 [A: not on this machine]); a stray `git checkout origin/main~4` (CoS log 2026-10-20); the CoS edited AGENTS.md/rules mid-brainstorm (`memory/2026-10-20.md`) [F]. Prose only until its § Phase 2 trigger.
- Attack the premise (`principle-attack-the-premise`): when a second fix in one class fails the same gate, stop patching, write the premise down, and open a `figure-it-out` unit that tests it first. Evidence: grid layout, 19 fix commits.
- Background wait (replaces P5 in the MVP): run `quaqua worker prompt` in the background; on wake, run `quaqua status`. `worker prompt` already blocks on `herdr agent wait` (#90) [F]. Drain at four points: after a critical section (writing a brief, a merge decision), at a wave boundary, on a wake, and before reporting to anh iu; never review a diff inside a drain (a review is its own unit).
- Budget: at about 70% of the wall-clock or cost budget, stop spawning and land what is verified (new; `worker-timebox.md` has no 70% rule).
- Pilot and liveness: link `rules/_all/worker-timebox.md` (:9, :11) [F], adding only "on a new unit shape, pilot one unit through brief → worker → verify → merge before any fan-out"; never restate a rule (`runs/tieude-h1drift.md`).
- Kept from v3.2 P1b unchanged: encode lessons in structure, retry by mode (two retries per unit, then replan), stop line, rollup at each wave boundary, resume vs fresh (resume stays the default; a fresh spawn gets consolidated scope), never block on the human for reversible calls only.
Acceptance: the S3 fixture test loads all 3 playbooks and 6 leaves · every cited leaf resolves, none over the cap · every `## Worker` ends with the `Verified:` step · the grep returns nothing · a fresh `quaqua init` has the AGENTS lines, pilot/liveness as a link
Verify: `cargo test --locked -p quaqua playbook`, then `cargo test --locked -p quaqua --test init`

## S6b — `/poteto-mode` skill

Scope: new `templates/workspace/.claude/skills/poteto-mode/SKILL.md`, `crates/quaqua/tests/init.rs`. Own PR after S6, reviewed alone. The name stays `poteto-mode`, as in pstack.
- `/poteto-mode <goal>` from anh iu, or the coordinator on any new task: run `quaqua playbook list`, pick one name, run `quaqua playbook show <name>`, write the brief, spawn per `models.md`.
- Front door, not a fence: Claude Code's `allowed-tools` pre-approves tools and does not restrict them, and the CoS runs with bypassed permissions. Whether the coordinator routes is measured by `--check-brief` warnings and S7, not enforced.
- Never invent a playbook name; never route inside a worker. A worker that loads it stops: its playbook is fixed in `runs/.<name>.playbook`.
- The AGENTS route line becomes "Before every brief, route through `/poteto-mode`."
Acceptance: a fresh `quaqua init` workspace has the skill; the AGENTS line names it
Verify: `cargo test --locked -p quaqua --test init`

## Pilot (S8)

- One pi worker (`opencode-go/deepseek-v4.1-flash`) and one claude worker, **≥5 real units each**, warn mode, on the S6 playbooks (P2 playbooks may land meanwhile; metrics are per playbook).
- Count per report: `## Playbook` present, `playbook_gap` events, gaps that hid real unfinished work (G3), verdicts recorded, merges without a verdict (G1), exit-6 resumes (G2).
- Output: § Phase 2 table filled with the real numbers, plus G1–G3 against the S0 baseline.

## P2 — Full port

Starts right after S6, in parallel with S7–S8, not on a trigger. Goal: the other 15 playbooks in `templates/workspace/playbooks/` and the other 18 leaves in `templates/workspace/principles/`, under the S6 port rules and the ≤3-leaf cap.
- From `skills/poteto-mode/playbooks/` [F]: refactoring, perf-issue, investigation, prototype, hillclimb, runtime-forensics, trace-forensics, visual-parity, eval.
- session-pickup [F]: resume from the prior report, branch and journal; the prior trail is authoritative; route the rest.
- pause-safely [F]: stop at a safe boundary, one `wip:` commit, a resume note in `runs/<name>.resume.md`; on an explicit pause or before a coordinator compaction, never on "keep going".
- multi-phase-plan [F]: the plan is a checklist an owner runs box by box; Orchestrator-only. The router's target for project-size work and the Lead brief.
- worktree-cleanup [F]: audit from `git worktree list`, classify by merge state, uncommitted work and age, remove through `quaqua worker cleanup`; drop the iOS half.
- shipping [F]: verify each PR independently, land only the contiguous verified run from the root; landing on main stays anh iu's approval.
- babysit [F]: declare a mode, clear one PR at a time, stop where the human's call begins.
- session-pickup, pause-safely and multi-phase-plan may be Orchestrator-only (empty `## Worker`).
- Leaves beyond a playbook's 3 Worker citations (e.g. feature's design leaves: redesign-from-first-principles, foundational-thinking, model-the-domain, boundary-discipline, type-system-discipline) are cited in its `## Orchestrator` section: the coordinator reads them through `playbook show`; they are never appended to a worker. The v3.2 citation table is the starting point; each batch reassigns citations under the cap.
- arena → N workers of different kinds + judge; architect → taskforce `dual-lens`; interrogate → reviewer on a different model. visual-parity runs headless only (`no-gui-driving.md`) [F]. eval reads `runs/` reports + telemetry and reports the playbook spread (every name, `none`, `figure-it-out`), `playbook_gap` events, and briefs with a missing header that ended in a failed or respawned unit.
Batches, one after another (same directories, lane B):
- P2a — refactoring, perf-issue, multi-phase-plan + their leaves
- P2b — investigation, prototype, hillclimb, session-pickup, pause-safely
- P2c — runtime-forensics, trace-forensics, visual-parity, eval
- P2d — shipping, babysit, worktree-cleanup
Acceptance: each file passes the S3 parser; after P2d `quaqua playbook list` shows all 18; every cited leaf resolves; the S6 grep returns nothing
Verify: `cargo test --locked -p quaqua playbook`

## Phase 2 — triggers

Each item is built only when its count from `quaqua status --metrics` (S7), the journal or memory reaches the trigger.

| Item | Trigger | Source |
|---|---|---|
| exit 6 for a missing `## Playbook` | ≥5 reports per kind (claude, pi), section present 5/5 on each, G2 not above baseline | metrics + pilot log |
| Required Scope / Acceptance / Verify headers | ≥2 units failed or respawned in 30 days with root cause = that missing header (coordinator-labelled) | memory + journal |
| `quaqua watch` + drain (v3.2 P5) | ≥1 WAITER-LOST / missed completion after the MVP lands, or a run with >4 concurrent workers | status |
| Edit/Write deny hook for coordinators | 1 more coordinator-edit incident after the AGENTS rule lands | memory |
| Same-family verdict → `source: self` | team.toml / models.md has a family table | config |
| `HARNESS.md` (per-harness skip mapping) | the first `skip: unsupported on <harness>` in a report | journal |
| Lead routing per Peer | the first SLP team that runs after S6b | status |

Notes for the deferred items:
- `quaqua watch`: v3.2's `status --drain` marked signals drained in `runs/quaqua.db`, which the CoS and Leads share (`lead/spawn.rs:237`), so one coordinator's drain would swallow another's signals. When built, use a stateless `status --since <seq>` or scope by owner (`WorkerMetadata::load_owner` exists). It is still P1 of `team-event-driven-monitor.md` and marks that plan's P1 superseded. herdr contract from the evtprobe run (`~/quaqua/raw/2026-10-17-tieude-evtprobe.md`): subscriptions are per pane, no wildcard, no snapshot on connect; `idle` with a background shell is a real idle, so `runs/<name>.md` stays the source of truth. Gates (`gate {question, options, default}`) follow `watch`.
- Deny hook: the PreToolUse hook is `quaqua gate` (`gate/mod.rs:1-3`, still invoked through `scripts/hooks/herdr-gate.sh`) and the template's matcher is `Bash` only (`templates/workspace/.claude/settings.json`) [F]. Denying coordinator edits under `crates/` needs a new Edit/Write matcher plus a path policy.
- `HARNESS.md`: one table per harness (read-only check, targeted test, headless UI → command per claude / omp / opencode / pi), rows keyed by H2 registry names; "open PR" is a row because workers do not open PRs.

## Out of scope

Failures the second pass found that no pstack concept here covers. Listed so they are not lost; each needs its own plan.
- Cost overruns: llmhub `$74.55` against a `$30` cap, sude `$71.3` (`memory/2026-10-20.md`); nothing enforces `Cost budget`.
- Team-level idle: llmhub idle ~14h (`memory/2026-10-20.md`); liveness is per agent. babysit (P2d) helps only while a coordinator is running.
- herdr `done` ≠ success: an API 400 ends `working → done`.
- Merge gates that run suites separately hide cross-module failures: arch W2 107/3 combined vs 31/31 pipeline alone (`memory/2026-10-20.md`).
- Lead inbox timestamps are not wall clock (`memory/2026-10-20.md` retro).

## Audit — does quaqua have the failure each concept fixes?

Kept from v3.2 as the evidence record, with the last column remapped to v3.3 steps. Method (2026-10-22): pstack @ 190d287 read from source; quaqua evidence from `~/quaqua/runs` and `memory/`, `git log --no-merges origin/docs/pstack-plan-v3` in the engine, `rules/_all/` and the code. Counts are files or commits matching the filter in § Filters, not incidents and not cost: a phrase scan misses incidents written in other words and counts files that only mention the phrase. Two passes: one Claude pass (v3.1), and a non-Claude pass (`runs/suhuynh-pstackfit.md`, opencode deepseek-v4.1-flash), which was not fully blind: a session restart showed it the table before it wrote its verdicts, so only its counts are independent. v3.1 counts that no filter reproduces are marked `unverified`. The second pass's reproduction found 273 reports where v3.1 counted 152 (`runs/suhuynh-pstackfit.md:100`), which is why § Success metrics replaces phrase counts as the measure.

| Concept | Count (filter) | Verdict | Lands in (v3.3) |
|---|---|---|---|
| Router skill | n/a: there is no router yet to fail | Keep, as front door only | S6b; measured by `--check-brief` warnings + S7 |
| Steps as todos, `skip:` | 1 false-done report (F1); v3.1's ~10 unverified | Keep, warn only | S4–S5; exit 6 on its Phase 2 trigger |
| figure-it-out | 0 | Keep as fallback, ported in full | S6 |
| Principles (24) | 19 grid fix commits (C3), 54 reports on drift / two copies (F2) | Keep, as cited leaves, ≤3 per Worker | S6 (6), P2 (18) |
| 9-field brief, refuse to spawn | 1 unclear-brief report (F3); v3.1's ~7 unverified | Adapt: `Playbook` + `Context` warn, rest folded into existing headers | S4 |
| Context relay | 7 reports on an unreadable brief path (F4) | Keep | S4 `Context:` |
| Standing orders | handled | Exists | spawn concatenates `rules/_all` (`worker/spawn.rs:322-355`) |
| Coordinator never edits code or config | 2 found of 3 cited | Keep, prose | S6; hook on its trigger |
| Queue and drain | 0 found | Deferred | S6 background-wait line; `watch` on its trigger |
| SHA verdict, verifier overrides | PR #42 | Keep, first | S1–S2 |
| Verifier on another model family | config compares harness only | Adapt: kinds + `same-harness` flag | S1–S2; `self` on its trigger |
| Liveness | 6 reports (F5) | Exists | `worker-timebox.md:9`; event watchdog (f65c479) |
| Retry by mode, stop line | ~4, unverified | Adapt, prose rule | S6 |
| Pilot | 1 (srt pilot) | Exists | `worker-timebox.md:11`; S6 links it |
| 70% budget | — | Adapt, new line | S6 |
| Fresh over resumed | spawneff resume saved $0.14 + 3h33m | Not adopted | resume rule stays |
| Gates, irreversible pause | 1 | Later | after `watch` |
| Session pickup, pause safely | 0 | Keep | P2b |
| multi-phase-plan | 35 reports mention phases (F6) | Keep | P2a |
| worktree-cleanup | 2 reports (F7) | Keep | P2d |
| shipping | PR #42 | Keep | P2d |
| babysit | 5 reports on long idle (F8) | Keep | P2d |
| autonomous-run, autopilot-full, autopilot-stack | — | Not ported | v3.2 reasons unchanged |
| opening-a-pr | workers do not open PRs | Not ported | a `HARNESS.md` row, on its trigger |
| Harness adapter | 16 harness fix commits (C4) | Keep, deferred | `HARNESS.md` on its trigger |
| Model per role | 3 fallback fix commits (C5) | Exists | `launch.rs:19-71` |
| Durable orch store | handled for the journal | Exists for the journal | `runs/quaqua.db` |

Filters (run 2026-10-22; R = `~/quaqua/runs/*.md` minus `.rules.md`, `.brief.md`, `.resume.md` = 362 reports, 274 with `## Lessons`; engine = `git log --no-merges origin/docs/pstack-plan-v3`, 194 commits, 81 matching `^(fix|hotfix|bugfix)`):
- F1 `grep -liE 'claimed done|reported done but|false done|not actually done|marked done but'` R
- F2 `grep -liE 'two copies|drift(ed)?\b|duplicated rule|stale copy'` R
- F3 `grep -liE 'brief (was )?(unclear|ambiguous)|ambiguous brief|unclear brief'` R
- F4 `grep -liE 'could not read the brief|brief path'` R
- F5 `grep -liE 'false exit|exited early|premature exit|false stall'` R
- F6 `grep -liE 'multi-phase|phase [0-9]'` R
- F7 `grep -liE 'prunable|stale worktree'` R
- F8 `grep -liE 'idle ~?[0-9]+h|idle for'` R
- C3 `git log --no-merges -iE --grep='^fix.*(grid|reconcile|anchor)'`
- C4 `git log --no-merges -iE --grep='^fix.*(opencode|omp|pi\b|claude|harness|launch)'`
- C5 `git log --no-merges -iE --grep='^fix.*fallback'`

Review decisions (2026-10-22, anh iu): D1 MVP 3 playbooks, with the full port right after S6 (not by trigger) · D2 only `Playbook` + `Context` · D3 P5 deferred to its trigger · D4 kinds + `same-harness` flag · D5 rejected: playbooks keep the `brainstorm|to-plan` ban and figure-it-out is ported in full · D6 rejected in part: the skill ships in the MVP as S6b and keeps the name `poteto-mode`.

Counterfactual, the costliest failures with S0–S8 in place:
- Grid layout (19 fix commits): the S6 attack-the-premise rule stops patching at the second failed fix in the class and opens a figure-it-out unit.
- W2 cutover (7 review findings): no step catches it; review already did.
- PR #42 merged without tests at its head SHA: S2's merge warning names it, and G1 counts it.
- Coordinator resolves a conflict and breaks main: S6's rule names it; only the Phase 2 hook would stop it.
- `## Blockers` false block → respawn: no step; S5 adds parse surface of the same kind, which is why it never exits, parses four line formats, and is pre-filled.

Pre-mortem: in six months the warnings are ignored and nothing became a gate, or a gate turned on and made more false blocks than it caught. G1 shows the first (merges without a verdict stay above 0); G2 against the S0 baseline shows the second, and a gate whose G2 rises above baseline goes back to a warning.
