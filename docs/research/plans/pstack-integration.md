# pstack integration — PR specs

Source: brainstorm 2026-10-21 (pstack @ 190d287, plugin 0.15.29, vs quaqua main d24be71). Revised 2026-10-21 against main 309f698 (H1 + H2 merged; H3a + H4 in flight). v3 adds the router, figure-it-out, session lifecycle, and the orchestrate rules found on a second read. v3.1 (2026-10-21, main dd01e68, H3a + H4 merged): P1 split into P1a/P1b, coordinator-never-edits-code rule, merge warns without a verdict at head, fixed Verify lines, and an evidence audit (§ Audit).
Status: **draft spec** (chưa giao worker) · spec only, nothing implemented. [F] = verified in repo/CLI or the pstack source, [A] = assumption the PR must confirm first.

Scope: only ideas ported from [pstack](https://github.com/cursor/plugins/tree/main/pstack). Non-pstack work (drift, read-only reviewers, pi, `harness/`, Codex) lives in `harness-hygiene.md` + `pi-harness.md`.

Locked decisions: workflow-first, not agent-first: the playbook is picked before the agent (P1b skill steps), and the harness is an adapter (`HARNESS.md`) · port ideas, not the pstack plugin · router = playbook data + `quaqua playbook list|show` + a `/poteto-mode` skill that is only the front door · Playbook header + Orchestrator/Worker split, exit 6 · port all 14 playbooks before eval · verdict = journal event, no TSV · brief gains Scope, Acceptance, Verify, Context in the same PR as the Playbook header; they warn until P2a/b/c are all merged, then a follow-up PR makes them required · no orch.ts/store.ts, no standing-orders file, no swarm.

| PR | From pstack |
|---|---|
| P1a Router engine + brief contract | poteto-mode (playbook index, steps as todos, `done` / `skip: <reason>`), orchestrate brief template (SCOPE, CONTEXT, ACCEPTANCE, VERIFY), `docs/harness/claude-code.md` (one mapping per harness, `Skipped in <harness>` line) |
| P1b Router skill + coordinator rules | poteto-mode as the one front door, orchestrate coordinator rules |
| P2 Port 14 playbooks | poteto-mode playbooks + figure-it-out |
| P3 Verdict in journal | orch verdict ledger keyed by SHA, worker self-report vs verifier override, prove-it-works levels |
| P5 CoS queue | orchestrate: completions are queue events, drain in batches, gates with a default |

P4 is folded into P1a: both change the brief headers in `brief_headers_ok` (`worker/prompt/mod.rs`), so two PRs would conflict on one function.

## Order and lanes

Prerequisites: H1 (one handback format) [F merged #95], H2 (harness registry) [F merged #96], H4 (review baseline: worktree HEAD + porcelain recorded at review spawn, `worker/prompt/readonly.rs:1-3`) [F merged #99].

| Lane | PR | Starts from | Waits for |
|---|---|---|---|
| A | P1a | main | nothing |
| A | P1b | main | P1a merged (the skill calls `quaqua playbook`) |
| B | P3 | main | nothing |
| C | P5 | main | nothing |
| D | P2a/b/c | main | P1a merged (the parser loads the playbooks); not P1b |

A, B and C can run in parallel. Shared files: A and B both touch `worker/prompt/`; B and C both touch `status.rs` and the journal. The second to merge rebases. P1b touches templates only.

Risk to test first: the exit-6 check assumes workers reliably write `## Playbook` with every step id. Before the check applies to the whole team, run P1a on one real pi worker (`opencode-go/deepseek-v4.1-flash`) and one claude worker. A model that keeps dropping the section turns exit 6 into churn. Report parsing has already produced false blocks twice: notes under `## Blockers` were read as real blockers (CoS log 2026-10-16 truonglao-intake, which cost a respawn; 2026-10-18 T1 bashclean). This is pstack's "pilot one unit through the whole path before fan-out" applied to P1a itself.

---

## P1a — Router engine + brief contract

Goal: the engine lists and shows playbooks, appends the worker's steps to the brief it submits, and checks the report against them; briefs gain the headers a stranger needs to execute them.
Scope: `worker/prompt/mod.rs` (`brief_headers_ok`, :41; submit path), `worker/prompt/watchdog.rs` (`validate_report_content`, :57-75) and its callers (`workflow.rs:467`, `mod.rs:956`, `cost.rs:483,488,511`), `rules/_all/brief.md`, new `templates/workspace/playbooks/` (fixture playbook + `HARNESS.md`; real playbooks in P2), new `playbook` subcommand. Line numbers are from d24be71: re-locate them on current main.

Design, router (anh iu: data + CLI; the skill is P1b):
- Each playbook file starts with frontmatter `name:` + `when:` (one sentence, the routing trigger). The index is generated from these files, never hand-written, so it cannot drift from `playbooks/`. pstack writes its index by hand in `poteto-mode/SKILL.md` § Playbooks [F]; this is the one place quaqua departs from it.
- `quaqua playbook list` prints `<name> — <when>` per file. `quaqua playbook show <name>` prints the `## Orchestrator` section for the CoS. Reuses the P1a parser.
- No playbook fits, or the work is large and cross-cutting → `figure-it-out` (ported in P2a), never a blank. pstack routes there for "no bundled playbook fits" [F]. `Playbook: none, <reason>` stays only for trivial one-command units.

Design, playbook contract:
- Brief header `Playbook: <name>`. `--check-brief` rejects unknown names; a missing header warns until P2a/b/c are all merged, as does `none` without a reason.
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
- New headers, each warning (not failing) until P2a/b/c are all merged; then a follow-up PR makes them required:
  - `Scope:` owned / forbidden paths.
  - `Acceptance:` checkable lines, outcomes not a solution, so it keeps the no-pre-solve rule.
  - `Verify:` the exact command(s), plus known gotchas. `cargo test` takes one test filter per run (`runs/ni-h3a.md`), so a Verify line with several filters is one run per filter.
  - `Context:` file / PR pointers; when this unit depends on another, that unit's report pasted in full. Reviewers could not read the brief path before H4 (`runs/suhuynh-r0r1.md`, `runs/tieude-r0.fix1.md`), and pasted text reaches every harness, so a dependency is a context relay, not just ordering.
- Size the brief to the unit: a one-command unit collapses to one paragraph that still names goal, scope, the verify command and the report shape. A full template around a two-line edit costs more to write and obey than the edit. The context relay is exempt: an upstream report is pasted in full even into a one-paragraph brief.
- Later (not this PR): `herdr-gate.sh` blocks writes outside Scope.

Acceptance: tests for unknown playbook rejected, Worker-only text in the submitted prompt (no Orchestrator text), only the worker's own harness rows appended, missing section → 6, missing step → 6, `skip:` without reason → 6, `skip: unsupported on <harness>` accepted, `--resume` keeps the check, report without Playbook brief unchanged, brief missing Playbook / Scope / Acceptance / Verify / Context warns but passes, `playbook list` built from the fixture frontmatter, `playbook show` prints only the Orchestrator section, every H2 harness has a row in `HARNESS.md`
Verify: `for f in prompt watchdog playbook; do cargo test --locked -p quaqua "$f"; done`
Forbidden: auto-classifying the playbook in the engine (the CoS picks; the engine only lists and checks)

## P1b — Router skill + coordinator rules (templates only)

Goal: one front door that routes every new task to a playbook before an agent is picked, and the coordinator rules the CoS follows; no engine change.
Scope: new `templates/workspace/.claude/skills/poteto-mode/SKILL.md`, `templates/workspace/AGENTS.md`, `crates/quaqua/tests/init.rs` (the skill lands in a fresh workspace).

Design, skill `templates/workspace/.claude/skills/poteto-mode/SKILL.md`, the only routing entry:
- `/poteto-mode <goal>` from anh iu, or the CoS on any new task. Steps: run `quaqua playbook list`, pick one name, run `quaqua playbook show <name>`, write the brief (all P1a headers), spawn per `models.md`.
- It is a front door, not a fence. Claude Code's `allowed-tools` pre-approves tools and does not restrict them, and the CoS runs with bypassed permissions, so nothing stops a routing turn from editing. Whether the CoS routes is measured, not enforced: the P2c eval reports how briefs spread across playbooks.
- Never invent a playbook name; never route inside a worker. A worker that loads it stops: its playbook is already fixed in `runs/.<name>.playbook` and it follows only the `## Worker` steps in its prompt.
- pstack's other skills (how, architect, swarm, interrogate…) are not bundled; each playbook names the quaqua equivalent (P2 rules).
- `templates/workspace/AGENTS.md`: one line, "Before every brief, route through `/poteto-mode`."

Coordinator rules in `templates/workspace/AGENTS.md`, from pstack `orchestrate.md` [F]:
- Coordinator never edits code: the CoS writes briefs, reads reports, verifies and merges. A conflicted merge or rebase is a worker unit, which `quaqua merge` already hands back as rework (exit 4, `merge/mod.rs:288-294`) [F]. Evidence: the CoS resolved the W3 merge conflict itself, which left 2 suites failing and leaked a real herdr spawn (CoS log 2026-10-18); a stray `git checkout origin/main~4` in the engine checkout (CoS log 2026-10-20). Later (not this PR): `herdr-gate.sh` denies CoS edits under `crates/` and `tests/`.
- Retry by mode: cap-hit / oom → respawn with smaller scope; network drop → retry as-is; tool error → retry on a different model; unknown → retry once. Two retries, then abandon the unit and replan.
- Stop line: when more spawning would produce garbage tree-wide (bad upstream output, broken acceptance, dead infra), write a stop line, let in-flight work finish, fix the cause, clear it.
- Budget: at about 70% of the wall-clock or cost budget, stop spawning and land what is verified.
- Pilot and liveness already live in `rules/_all/worker-timebox.md` (:11 pilots get 30 min, :9 never blind re-prompt, read the pane) [F]. AGENTS.md links there and adds only "on a new unit shape, pilot one unit through brief → worker → verify → merge before any fan-out"; it never restates a rule, because a restated rule drifts (`runs/tieude-h1drift.md`).
- Rollup: at each wave boundary, account for every spawned worker (arrived, respawned, or scope absorbed); never silently redo a missing worker's job.
- Fresh over resumed: a fix round or follow-up gets a fresh worker with consolidated scope (original brief + later directives + prior report), unless it needs state only the old worker has (uncommitted changes, a running process). This names the existing `.fixN` practice.

Acceptance: a fresh `quaqua init` workspace has `.claude/skills/poteto-mode/SKILL.md`; AGENTS.md has the route line and the coordinator rules; pilot and liveness appear as a link to `worker-timebox.md`, not as copied text
Verify: `cargo test --locked -p quaqua --test init`

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
- eval reads `runs/` reports + telemetry, not `~/.cursor/projects/`; it reports how briefs spread across playbooks (every name, plus `none` and `figure-it-out`), so a router the CoS skips, or one playbook that takes everything, shows up as a number
- the last `## Worker` step of every playbook (except Orchestrator-only ones) is "check every Acceptance line, then write `Verified: <level> <sha>`" with the P3 levels; before P3 lands it is plain report text
- each port carries frontmatter `name:` + `when:`; the `when:` line is adapted from the playbook's one-line entry in pstack's `poteto-mode/SKILL.md` § Playbooks
Split into 3 PRs, so each review stays small:
- P2a — code + fallback: bug-fix, feature, refactoring, perf-issue, figure-it-out
- P2b — explore + lifecycle: investigation, prototype, hillclimb, session-pickup, pause-safely
- P2c — forensics + eval: runtime-forensics, trace-forensics, visual-parity, eval
P2a, P2b and P2c touch disjoint files and can run in parallel once P1a is merged.
Acceptance: each file passes the P1a parser; `quaqua playbook list` shows all 14; every non-empty `## Worker` ends with the `Verified:` step; `grep -riE "cursor|Task tool|/loop|brainstorm|to-plan"` in playbooks/ returns nothing
Verify: P1a test fixture loads every playbook in the directory

## P3 — Verdict in the journal

Goal: separate agent-done → verified → ship-ready, with verified tied to a SHA.
Builds on H4 [F merged #99]: H4 records the worktree HEAD + `git status --porcelain` at review spawn and voids a verdict whose tree changed (`worker/prompt/readonly.rs:1-3`). P3 reuses that recorded SHA as the verdict's `sha`; it does not compute its own. One place decides "stale".
Scope: `engine/journal_sqlite.rs` (new event kind, no schema change: `events.kind` + `payload_json`) [F], `pipeline/mod.rs` (Verdict parse, :2662 at 0fc0ad8) [F], `status.rs`, `merge/mod.rs`
Design (pstack orchestrate § Verification) [F]:
- event `verdict {branch, sha, level, evidence, verifier, source: self|verifier}`; levels `unit-test-verified | type-check-only | review-clean | inconclusive | verifier-blocked | verifier-failed`
- a builder may self-report; a reviewer verdict on the same branch + SHA overrides it. `quaqua status` shows the strongest source for the current SHA.
- `inconclusive` and `verifier-blocked` are never a pass; `verifier-failed` calls for a fix unit, not a re-review
- CI green is evidence inside a verdict, not a verdict
- report line `Verified: <level>  <sha>`
- `quaqua status` shows the per-branch level; stale if head SHA ≠ verdict SHA (turns "new commit voids review" from prose into a check)
- ship-ready stays anh iu's approval; the engine only refuses to call a stale SHA "verified"
- `quaqua merge` prints the branch head's strongest verdict and warns when no verdict matches the head SHA. Today its check passes when no check command is set (`merge/mod.rs:157-165`) [F]. Evidence: PR #42 merged at `b12f74f` while the last CoS test run was at `a7de35b` (its handoff comment says so).
Acceptance: tests for record/read verdict, stale on new commit, status output, reviewer overrides self-report on the same SHA, `inconclusive` not shown as verified, a verdict H4 voided is never recorded as verified, `quaqua merge` warns on a head SHA with no verdict
Verify: `for f in journal status pipeline merge; do cargo test --locked -p quaqua "$f"; done`

## P5 — CoS queue via `quaqua watch`

Goal: worker completions/failures/blockers land in one queue; the CoS drains it at boundaries instead of waiting on each agent, and asks anh iu only through batched gates.
Already on main [F]: `quaqua status` with `DONE-NOT-CLEANED` / `IDLE-UNPROMPTED` / `ORPHAN` (`status.rs:50-52`) and `--metrics` (#93); the per-worker watchdog blocks on `herdr agent wait` (`herdr.rs` `agent_wait`, #90). Missing [F]: no `watch` subcommand exists.
One design, not two: `quaqua watch` is P1 of `team-event-driven-monitor.md` (:19, :26; #47, draft) [F]. P5 builds it once and adds only pstack's drain and gates; the same PR marks that plan's P1 as superseded by P5.
herdr contract (from the evtprobe run, `~/quaqua/raw/2026-10-17-tieude-evtprobe.md`, workspace-private, 8/8 transitions delivered, latency ≤1.6s):
- subscriptions are per pane, no wildcard, no snapshot on connect, one subscribe per connection → pin the pane set, read state once at connect, reconnect and re-sync when topology changes (`pane.agent_detected`)
- `idle` at the end of a turn with a background shell is a real idle → a status event is never proof of completion; `runs/<name>.md` stays the source of truth
Design:
- `quaqua watch <names…> [--timeout]` reuses `Herdr::agent_wait` per name (one thread each) [A: `herdr agent wait` takes one name]. On `idle | done | blocked` or pane exit it checks `runs/<name>.md`, appends a journal event `worker_signal {name, status, report: present|missing}`, prints one line and exits. The CoS runs it in the background and is woken instead of sleep-polling.
- Drain discipline (pstack orchestrate § Queue and drain) [F]: a signal is recorded and the CoS returns to what it was doing; it never reviews a diff inside a drain (a review is its own reviewer unit). The CoS drains at four points: after a critical section (writing a brief, a merge decision), at a wave boundary, on a `watch` wake, and before reporting to anh iu.
- `quaqua status --drain` lists undrained signals, marks them drained, and ends with three lines: counts per state, what changed since the last drain, open gates.
- Gates: a question for anh iu is a journal event `gate {question, options, default}`, shown in the drain's last line and batched into the next report, never asked one by one. The CoS routes work around an open gate. Irreversible actions (merge to main, push, branch delete, `herdr integration install`) are always gates with no default.
Acceptance: replayed event fixture → correct one-line output and journal event; idle with no report → `report: missing`, not done; socket drop → reconnect + re-sync; `--drain` output ends with the three lines; a gate without `default` is rejected unless it is on the irreversible list
Verify: `for f in watch status; do cargo test --locked -p quaqua "$f"; done`; one live run with 2 workers
Forbidden: a daemon or scheduler beyond `watch`

## Audit — does quaqua have the failure each concept fixes?

Method (2026-10-21): pstack @ 190d287 read from source; quaqua evidence from 152 worker reports (247 `## Lessons` lines), 79 fix commits out of 193 non-merge commits, the CoS daily logs 2026-10-1x/2x, `rules/_all/`, and the code. Counts are incidents, not cost. Not validated: the blind check by a non-Claude reviewer was stopped, so these verdicts are one Claude pass.

| Concept | quaqua incidents | Verdict | Where it lands / evidence |
|---|---|---|---|
| Router skill | 0 routing failures | Keep, as front door only | P1b; measured by the P2c eval, not enforced |
| Steps as todos, `skip:` | ~10 false done | Keep (locked) | P1a exit 6; the parse risk is under Risk |
| figure-it-out | 0 | Keep as fallback | P2a; probes already work this way (`watchdog-event-driven.md` wdprobe) |
| Principles (attack-the-premise, encode lessons in structure) | ~12 two-copies drift, ~20 grid fix commits | Not ported | see Counterfactual |
| 9-field brief, refuse to spawn | ~7 unclear briefs | Adapt | P1a: 4 headers; `brief.md` + `--check-brief` already cover the rest |
| Context relay | 2 | Keep | P1a `Context:` (`runs/suhuynh-r0r1.md`, `runs/tieude-r0.fix1.md`) |
| Standing orders | handled | Exists | spawn concatenates `rules/_all` into every `runs/<name>.rules.md` (`worker/spawn.rs:330-345`) |
| Coordinator never edits code | 2 | Keep, new | P1b (CoS logs 2026-10-18, 2026-10-20) |
| Queue and drain | 5 "is anything running?" prompts in one CoS session | Keep | P5 (`team-event-driven-monitor.md:7`) |
| SHA verdict, verifier overrides | ~10 false done + 3 stale | Keep, new | P3 + merge warning (PR #42; "merge-tree clean ≠ compiles") |
| Verifier on another harness | handled | Exists | config rejects builder = reviewer harness (`tests/fixtures/invalid/same-harness.toml`) |
| Liveness | 13 false exits (fixed) | Exists | `worker-timebox.md:9`; event watchdog (f65c479) |
| Retry by mode, stop line | ~4 (78-min stall, lead cap hit twice, hermpin cancelled by anh iu) | Keep | P1b prose; pipeline already caps rounds (`pipeline/mod.rs:2082`) |
| Pilot, 70% budget | 1 (srt pilot) | Exists + 70% line | `worker-timebox.md:11`; P1b links it |
| Fresh over resumed | handled | Exists | `.fixN` workers; P1b names it |
| Gates, irreversible pause | 1 (Lead prompt lost while the CoS waited) | Keep | P5; `lead-inbox.md` |
| Session pickup, pause safely | 0 | Keep (locked, 14 playbooks) | P2b; `handoff.md` already covers handoff |
| Harness adapter | ~15 harness quirks | Keep | P1a `HARNESS.md`, built from the existing per-harness rules (`opencode-no-bg-wait.md`, `herdr-plugin-smoke.md`) |
| Model per role | ~5 fallback fix commits (fixed) | Exists | `launch.rs:19-71` fails closed per kind |
| Durable orch store | handled | Exists | event journal `runs/quaqua.db` |

Counterfactual, the costliest failures with P1a–P5 in place:
- Grid layout (~20 fix commits, the self-anchor loop 3–4 times in one day): no P-step catches it. pstack's answer is principle-level (attack the premise; one predicate per fact), which this plan does not port.
- W2 cutover (7 review findings, a second review round): no P-step catches it; it was caught by review, which quaqua already has.
- PR #42 merged without tests at its head SHA: P3's merge warning catches it.
- CoS resolves a conflict and breaks main: P1b's rule names it; only the later gate hook would stop it.
- `## Blockers` false block → respawn: no P-step; P1a adds parse surface of the same kind.

Pre-mortem: in six months exit 6 and the four new headers have made more false blocks and resumes than the false-done reports they caught. Watch the ratio of exit-6 resumes to reports where a missing step hid real unfinished work; if resumes win, make `## Playbook` a warning.
