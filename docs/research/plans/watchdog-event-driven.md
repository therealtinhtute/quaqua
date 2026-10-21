# Event-driven worker watchdog without screen scraping (issues #47 P1, #87)

Status: **design** · truonglao-wddesign · 2026-10-21 · herdr 0.9.5
Inputs: `docs/research/probes/wdprobe/REPORT.md` (branch `quaqua/tieude-wdprobe`, PR #88),
`docs/research/probes/evtprobe/REPORT-2026-10-17.md`, live probes below.

## Verdict

Keep the engine workflow and the 0/2/3/4/5/6 contract, but make it **one blocking native
call per wake** instead of five activities every 30 s:

1. **Submit synchronously** with `herdr agent prompt <n> <body> --wait --until working --until blocked --timeout 10000`.
   This is herdr's own "prompt landed" check, and it removes the land/recover state machine.
2. **Wait natively** with `herdr agent wait <n> [--until …] --timeout <t>`. The `--until` set
   flips with the current status, and `<t>` is clamped to the next deadline.
3. **Measure progress only from glyph-free sources:** worktree diff hash plus the harness
   session-store mtime. A harness with neither source gets the time budget only.

Deleted: the event subscriber (`herdr_events.rs`), every pane read in the watchdog
(`ReadAgent`, `observe()`'s pane, ctx and last-tool parsing), the glyph filters, question
string matching, and the dead legacy `run_watchdog`. Production code shrinks by about 2.2k
lines, tests by about 0.6k. No new command: drop `quaqua watch`. The CoS can call
`herdr agent wait` itself, and `quaqua status` already exists.

## Probe evidence (this task, 2026-10-21)

Setup: one throwaway opencode worker `tieude-wdp1` (roster model
`opencode-go/deepseek-v4.1-flash`), spawned via `quaqua worker spawn tieude-wdp1 tieude <scratch>/wdd-repo`.
A raw `herdr workspace create` is blocked by `quaqua gate`, so the sanctioned path was
used. The worker was closed by `quaqua worker cleanup`, after which `herdr agent get tieude-wdp1`
returns `agent_not_found`.

| # | Question | Command | Result |
|---|---|---|---|
| U1a | Does wait return at once on an already-idle agent? | `herdr agent wait tieude-wdp1 --timeout 5000` on a fresh idle agent | **yes**: rc 0, `idle`, `state_change_seq 406`, **46 ms** |
| U1b | Does a wait right after a prompt return early? | `herdr agent prompt … 'sleep 25 …'` (no wait), then at once `herdr agent wait … --timeout 90000` | status still **`idle` 406** after the prompt returned (307 ms). Wait returned **`idle` 406 in 25 ms**, which is a false "finished". It flipped to `working` 407 a few seconds later and `done` 408 at the turn end (wait 39 s) |
| U1c | Does the native land check work? | `herdr agent prompt … --wait --until working --timeout 30000` | `working` **409** in **511 ms** |
| U2 | What happens when the pane closes mid-wait? | background `herdr agent wait tieude-wdp1 --timeout 120000`, then `quaqua worker cleanup` at +12 s | wait returned **at close**: rc 1, `{"error":{"code":"agent_not_running"}}`, dt 12.4 s |
| U2b | What does wait return for an unknown name? | `herdr agent wait nosuch-agent-xyz --timeout 2000` | rc 1 at once, `{"error":{"code":"agent_not_found"}}` |
| U3 | Can herdr see a silent tool call? | `herdr pane process-info --pane w1:p8P` during `sleep 90`, plus `ps --ppid` | process-info lists only the foreground group (`opencode --standalone --auto`). The tool shell `zsh -c sleep 90` is a child of `opencode serve --stdio`, so **herdr cannot see it** |

Answers:
- **U1:** `agent wait` matches the *current* state, so it cannot tell "just prompted" from
  "finished". `herdr agent prompt --wait` handles that: the help text says it "requires an
  observed working or blocked state within 5000ms; otherwise it returns
  agent_prompt_stalled". Submitting synchronously means the watchdog never starts from a
  stale idle. `state_change_seq` moves by exactly one per transition (406→407→408) and is a
  second check. Not verified: whether non-status reports also bump it.
- **U2:** a pane close ends the wait at once with `agent_not_running`. An unknown name gives
  `agent_not_found`. Neither needs a subscriber or `pane.exited`.
- **U3:** there is no native progress signal during a silent tool call. wdprobe measured the
  session store flat for 40 s during `sleep 40`. `pane_output_changed` bumps on idle TUI
  chrome (wdprobe §3), and process-info sees only the foreground process. Answer: time
  budget, plus a stall on "no diff and no store write for 600 s".
- **U4:** see the delete/keep list.

## Signal table

| Watchdog question | Signal (native or glyph-free) | Where | Exit |
|---|---|---|---|
| Finished? | `runs/<name>.md` stable (`report_ready` double-stat) + `validate_report_content` | ReportFile activity | 0 / 6 |
| Prompt landed? | `agent prompt --wait --until working --until blocked --timeout 10000`. On `agent_prompt_stalled`: send one `enter`, re-wait 10 s | mod.rs submit (before the workflow) | 3 "prompt not landed" |
| Waiting for input (question/permission/trust)? | `agent_status=blocked` (wdprobe: claude, opencode, omp all verified). `agent_blocked` on submit | wait result | 5 |
| Turn ended? | `done`/`idle` from the wait, **never as completion proof** (evtprobe: idle at turn end with a background shell; API 400 ends `working→done`) | wait result + idle timer | 3 after `idle_sec` without report |
| Pane/agent gone? | wait error `agent_not_running` / `agent_not_found` | wait result, re-checked once after `gone_sec` | 3 |
| Progress (all harnesses) | `GitRequest::DiffHash` (unchanged) | existing activity | resets stall; drives auto-extend (unchanged) |
| Progress (reviewers, no diff) | session-store mtime by `agent_session`: opencode `max(session_message.time_updated)` for the id; claude `~/.claude/projects/<slug>/<id>.jsonl` mtime; omp `kind=path` mtime | new `SessionMtime` activity; path logic shared with `cost.rs` | resets stall |
| Silent long tool call | **none** (U3) | — | budget (4); stall (3) after 600 s if neither diff nor store moved |
| Spend | session store (existing `cost.rs`), keyed by the session id, cwd and harness from the wait result | cost tracker per wake | warnings only |
| Lead waiting on its pipeline | `runs/pipeline-*.state` | moved **into the wait activity's result** (today `std::fs` runs inside `step`, workflow.rs:457, :578) | suppresses idle/stall |

The session store is **required, not optional**. `--review` workers write only to
`~/quaqua/runs`, so their worktree diff never moves. With diff alone, every reviewer would
stall at 600 s. If a harness has no readable store and the diff is static, the stall check
is skipped and the time budget alone applies (principal decision).

## Loop

```text
# mod.rs: submit (fresh prompt only; --resume skips this)
r = herdr agent prompt N BODY --wait --until working --until blocked --timeout 10000
match r:
  ok(working|blocked)     -> baseline_seq = r.state_change_seq
  err(agent_blocked)      -> exit 5
  err(agent_prompt_stalled|timeout) ->
       herdr agent send-keys N enter              # once (typed but unsubmitted input)
       r2 = herdr agent wait N --until working --until blocked --timeout 10000
       r2 ok -> baseline_seq = r2.seq ; else exit 3 "prompt not landed"
run SingleWorkerWorkflow(state{baseline_seq, ...})

# workflow.rs: step (deterministic; all I/O through activities)
init:  compute allowance/cap/nudge deadlines (unchanged); schedule wake(until=DEFAULT, t=0)
       # t=0: first wake is just an observation, no wait

on ActivityCompleted("wake_k", W):   # W = {status, seq, err, session, harness, cwd, lead_waiting}
  now = ctx.now_ms
  if W.err in {agent_not_running, agent_not_found}:
      gone_since ?= now; if now-gone_since >= gone_sec: exit 3 (gone)
      schedule_timer("backoff", min(gone_sec, next_deadline-now)); return
  if W.err is other (socket/io): errors += 1; schedule_timer("backoff", 30s clamp); return
                                   # never re-wait at once: no hot loop, no journal flood
  gone_since = 0; status = W.status
  schedule CheckMtime(report); DiffHash(repo); SessionMtime(W.session)   # one each per wake

on all three results for wake_k:
  report ready  -> Read -> exit 0 | 6
  now >= cap               -> exit 2
  status == blocked        -> exit 5
  progress = diff or store moved -> last_progress = now
  now >= allowance         -> grace (+25 %, once, if last_progress within 2 heartbeats)
                              autoext (diff moved; +25 % steps, ≤ +50 %), else exit 4
  status in {idle, done} and !lead_waiting:
      idle_since ?= now; now-idle_since >= idle_sec -> exit 3 (idle without report)
  status == working and has_progress_source and !lead_waiting
      and now-last_progress >= stall_sec          -> exit 3 (stall)
  nudge at nudge_pct (once, unchanged)
  until = status in {idle, done} ? [working, blocked] : DEFAULT(idle|done|blocked)
                                   # flip, or wait returns in 46 ms and spins (U1a)
  t = clamp(min(heartbeat, nudge_at, allowance, cap, idle_end, stall_end) - now, ≥1 s)
  schedule wake_{k+1} = AgentWait{N, until, t}

on TimerFired("backoff"): schedule wake_{k+1} = AgentWait{N, DEFAULT, 0}   # plain re-observe
```

Defaults: heartbeat 60 s (`PROMPT_WORKER_POLL_SEC` becomes the heartbeat), stall 600 s,
idle 120 s, gone 60 s, nudge 80 %, cap 7200 s. Report latency: immediate on a `done` wake.
Otherwise it is bounded by one heartbeat, because a single ready check replaces "two
matching samples". Today the activity already double-stats over 400 ms (mod.rs:405), and the
two-sample rule in the workflow doubles the wait.

Determinism: the `AgentWait` result is journaled like any activity. Its timeout comes from
`ctx.now_ms()` only. Replay reads results from the journal and never calls herdr. The runner
already runs activities synchronously (`engine/runner.rs:661`), so a blocking activity fits.
The build step must verify that a step which schedules only an activity, with no timer, keeps
the runner loop alive.

## Delete / change / keep

Line numbers are on `main` at `cb2df49`.

**Delete**

| What | file:line | lines |
|---|---|---|
| Event subscriber (only `pane.exited` used, drained per tick) | `crates/quaqua/src/herdr_events.rs` (whole) + `lib.rs:11` | 1139 |
| `execute_herdr_events` + `event_clients` field + dispatch | `engine/activities.rs:539-630`, `:25`, `:749` | ~95 |
| Event activity tests | `engine/activities.rs:1774-2016` | ~243 |
| `HerdrEventRequest/Result` + `ActivityRequest/Result::HerdrEvents` | `engine/activity.rs:99-125`, `:231`, `:241` | ~29 |
| `HerdrRequest::ReadAgent` / `HerdrResult::AgentOutput` + executor | `engine/activity.rs:62-65`, `:85-87`; `engine/activities.rs:129-~150` | ~30 |
| Pipeline event shim | `pipeline/mod.rs:133-168` | ~36 |
| Testkit scoped-event tests | `engine/tests.rs:910-996` | ~87 |
| Prompt event shim + ReadAgent shim | `worker/prompt/mod.rs:349-386`, `:425-439` | ~53 |
| `observe()` side effects in the AgentInfo arm: provider-timeout re-prompt, land/recover state, pane fingerprint | `worker/prompt/mod.rs:443-526` (+ mutexes `:339-345`) | ~90 |
| Workflow: `clean_pane_body` + `compute_fingerprint` | `worker/prompt/workflow.rs:215-253` | 39 |
| Workflow: subscribe + 5-activity scheduling (×2) | `workflow.rs:282-323`, `:663-695` | ~75 |
| Workflow: `read_agent_` and `poll_events_` arms | `workflow.rs:428-495` | 68 |
| Workflow: land/recover branch (moved to submit) | `workflow.rs:595-614` (+ state fields `:102-109`) | ~28 |
| Workflow: `question_seen` OR in the blocked check | `workflow.rs:528` | 1 |
| Glyph filters + question string matching | `observer.rs:25-77` (`question_seen`; glyphs :36-38, strings :50-59) | 53 |
| Pane read + body fingerprint + ctx % + last-tool (`▸`/`⏺` glyphs) | `observer.rs:229-297` | 69 |
| Harness guessed from name prefix (agent JSON has `agent`) | `observer.rs:219-227`, `engine/activities.rs:~300-307` | ~17 |
| Dead legacy loop (only re-exported, never called) | `worker/prompt/watchdog.rs:87-527` (`WatchdogParams` + `run_watchdog`), re-export `mod.rs:12` | ~440 |
| Pane/question/event-based tests | `workflow.rs` tests at :901 (stall via pane), :1264 (question), :1398 (pane churn), :1822 (scoped events) | ~264 |
| Shell assert on scraped ctx | `tests/test_prompt_worker_watchdog.sh:528` (`ctx=37%`) | 1 |
| OWNERS K2 row | `engine/OWNERS.md:17` | 1 |

**Decision needed (recommend delete):** `is_provider_timeout` (`observer.rs:79-96`,
`mod.rs:470-478`). It is the last pane read: a string match that sends `continue` once.
Without it, a provider timeout surfaces as exit 3 "idle without report". Per
`worker-timebox.md`, the CoS already reads the pane on exit 3 and resumes. The fallback,
if the principal wants auto-recovery, is one lazy `agent read` only on the
idle-without-report path. It is never on the tick.

**Change**

- `mod.rs:727-729`: background `agent_prompt_wait_background(…, 1800000)` becomes a
  synchronous land check (pseudocode above). `herdr.rs:155` `agent_wait_idle` becomes
  `agent_wait(name, until: &[&str], timeout_ms)`.
- `HerdrRequest::AgentInfo` becomes `HerdrRequest::AgentWait { name, until, timeout_ms }` →
  `HerdrResult::AgentWait { status, seq, session_kind, session_value, harness, cwd, err, lead_waiting }`.
  `timeout_ms = 0` is a plain `agent get`.
- New `SessionMtime { harness, session_kind, session_value, cwd }` → `Option<u64>`. Path
  logic is shared with `cost.rs` (it already reads the opencode db and the omp sessions).
- `CostTracker::check` takes `(session_id, cwd, harness)` instead of an `Observation`.
- Heartbeat and exit lines print `status`, `quiet`, `diff`. Drop `ctx=` and `last=`, which
  were pane-scraped.
- `hook_active` / `is_fallback_idle` (300 s for opencode/omp without a hook) stay as they
  are. The integration hook is what makes `blocked`/`idle` trustworthy. Revisit only with
  data.

**Keep:** exit codes and their order (cap › blocked › budget › idle › stall, report first),
`validate_report_content`, grace/auto-extend arithmetic, nudge, `--resume` / `--extend`,
the journal and snapshot, `report_ready`, `DiffHash`, `CostTracker`, `quaqua status`.

## Journal and replay

Removing the `HerdrEvents`, `ReadAgent` and `AgentInfo` variants breaks serde for in-flight
journals in `runs/quaqua.db`, so `--resume` across the upgrade would fail. Plan: keep the
old variants **deserialize-only** (`#[serde(other)]`-style tombstones, never scheduled) for
one release. New state fields (`baseline_seq`, `last_store_mtime`, `errors`) get
`#[serde(default)]`. Then drop the tombstones.

## Rejected alternatives

| Alternative | Why not |
|---|---|
| Drop the engine; plain loop in `mod.rs` | Loses the resume snapshot (elapsed, grace, nudged, autoext) and the 26 virtual-clock testkit tests, and breaks the journal/replay constraint. The cost was never the engine; it was five activities per 30 s tick. |
| Keep a long-lived `events.subscribe` client (`herdr_events.rs`) | Needs the pin/snapshot/re-sync contract (evtprobe). 1139 lines for one `pane.exited` drained per tick. `agent wait` gives the same push latency and reports a pane close as `agent_not_running`. |
| Raw `events.wait` on the socket | Same signal as `agent wait` plus our own socket client. The CLI is enough. |
| `pane_output_changed` / revision as progress | Bumps on idle TUI chrome (wdprobe §3). It is a wake-up, not progress. |
| `pane process-info` or a `/proc` child walk for silent tools | process-info sees the foreground only (U3). A `/proc` walk is not native and not portable (macOS). |
| Move the glyph filters to `team.toml` | Principal: delete, not move. |
| `quaqua watch` command | The CoS can run `herdr agent wait <n>` directly. `quaqua status` covers the table. |
| Keep `question_seen` as a fallback | `blocked` is verified on all three harnesses (wdprobe run 2). The string list is the bug class #87 is about. |

## Risks

1. **Silent tool call longer than 600 s** (cold `cargo build`, long test) with no diff and no
   store write gives a false exit 3 stall. If the tool printed output, the old pane
   fingerprint would have counted that as progress, so this is a regression. Mitigations:
   the CoS reads the pane and runs `--resume` (the current rule), or a builder raises
   `PROMPT_WORKER_STALL_SEC`.
2. **Unchanged:** opencode idle at turn end with a live background shell, and upstream API
   errors (`working→done`), still end as exit 3 after `idle_sec`. Removing the provider
   retry makes the second case need a CoS turn.
3. **herdr's 5000 ms land window is fixed.** A cold harness can exceed it; covered by the
   one-enter retry plus a 10 s re-wait. Measured land time was 511 ms.
4. **`state_change_seq` semantics** are verified only for status transitions. The design uses
   it only as a secondary check.
5. **herdr daemon restart mid-wait** gives a socket error, then a backoff timer. A long
   outage shows as gone, so exit 3 after `gone_sec`.
6. **Claude session-store path** needs the `cwd → <slug>` mapping (wdprobe derived it). Until
   a reader exists, claude review workers are budget-only for stall.

## Follow-ups (not in this change)

- `rules/_all/worker-timebox.md`: "within 2 polls" becomes "within 2 heartbeats". Stall reads
  "no worktree diff and no harness session write for 600 s". Remove "pane content" from the
  stall wording.
- Build order: (1) submit and land check, (2) `AgentWait` and the loop, with tests ported to
  the new mock set, (3) deletions, (4) journal tombstones. Targeted tests:
  `cargo test --locked -p quaqua worker::prompt` and `engine::`. `--slow` is run at the Lead's gate.
