# Shared team metrics: one cost/time row per worker (issue #47 P3, #48)

Related: [`plan.md`](plan.md) · [`design.md`](design.md) (§D `farm_events`) · [`event-monitor.md`](event-monitor.md) · [`README.md`](README.md)

> **Folded into farm-layouts.** The per-worker metric row lands in `farm_events` in
> `runs/quaqua.db` ([`design.md`](design.md) §D, phases P2 and P6). That overrides both the
> `runs/telemetry.jsonl` pick in §Verdict and the "no table in `runs/quaqua.db`" rejection below.
> The rest of this file — the offline spend readers in §Evidence and their traps — still applies.
> See [`README.md`](README.md) for the milestone that builds it.

Status: **design** (not built) · truonglao-metrics · 2026-10-21
Inputs: `event-monitor.md` §3, `docs/research/plans/watchdog-event-driven.md`,
issues #47 and #48 (with comments), `docs/research/plans/harness-effort-cost-audit.md`, live stores below.

## Verdict

Write **one JSON line per worker, at `quaqua worker cleanup`**, into the existing
`runs/telemetry.jsonl`. Read it back with **`quaqua status --metrics`**. That gives the CoS and
the Leads one log of time, prompts, exit code, dollars and tokens for every harness. No daemon, no
new store, and no new command.

1. **Where:** `runs/telemetry.jsonl`. It is append-only and already holds one row per pipeline
   stop. New rows carry `"rec":"worker"`, and pipeline rows gain `"rec":"pipeline"` (rows without
   `rec` are read as pipeline rows). It does not go into `runs/quaqua.db`, which is the engine's
   workflow journal (see Rejected).
2. **Who and when:** `cleanup_worker` (`crates/quaqua/src/worker/cleanup.rs:54`). Every worker
   passes through it, including pipeline builders and reviewers. It already calls
   `herdr agent get` (`cleanup.rs:65-67`), which returns the harness, the session id and the cwd. The row
   must be written **before** `cleanup_worker_files` (`cleanup.rs:124`), because that call deletes
   `.opencode-state/<name>` (`state.rs:197`), and that directory can hold the worker's only
   opencode DB (`cost.rs:192-199`).
3. **Spend reader:** extend `crates/quaqua/src/worker/prompt/cost.rs`, the reader the watchdog
   already uses, so it returns tokens as well as dollars. Claude gets a token-only branch.
   `read_worker_cost` becomes a thin wrapper over it, so the 80%/100% warnings and the metrics
   read the same numbers.
4. **Scope split with #48:** P3 *records* what each store gives. #48 keeps the two decisions
   that change behaviour: a USD price for claude tokens, and warn vs kill.

## Evidence: can each harness's spend be read offline?

| Harness (kinds) | Dollars | Tokens | Source, proven on live data 2026-10-21 |
|---|---|---|---|
| opencode (`tieude`, `suhuynh`) | **yes** | **yes** | `session_v2.cost, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read` (`usage.rs:826-830`). Live: `~/.wt/tieude-p74lock` → `0.0458 \| 94574 \| 16991 \| 3754496` |
| omp (`sude`, `detu`) | **yes** | **yes** | each assistant line of `~/.omp/agent/sessions/<dir>/*.jsonl` has `message.usage = {input, output, cacheRead, cacheWrite, totalTokens, reasoningTokens, cost:{…, total}}` and a `timestamp`. Live: `-Lab-.wt-detu-claudediff`, 67 usage lines, first `{"input":17371,"output":952,…,"cost":{"total":0.0166}}` |
| claude (`ni`, `truonglao`) | **no** | **yes** | `~/.claude/projects/<slug>/<session>.jsonl` holds `message.usage {input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens}` and `message.model`. There is no `costUSD` or `total_cost_usd` key (grep count 0 on this session). The session id comes from `herdr agent get` → `agent_session.value` (live: `79fca857-…`) |

Two traps found in the live data, which the reader must handle:

- **claude repeats usage per message.** This session had 36 usage lines but 16 distinct
  `message.id`s. A naive sum gives `cache_read 2,641,593 / output 16,928`; deduplicating by `message.id`
  gives `1,262,078 / 6,995`. Without the dedupe the totals are about 2× too high.
- **The opencode reader keeps only the newest session.** `cost.rs:211` does
  `… WHERE directory = '{cwd}' ORDER BY time_updated DESC LIMIT 1`. Live, 5 worktree dirs have
  more than one session, for example `.wt/tieude-promptfix`: 2 sessions, sum `0.1848`, max `0.1308`. A
  worker that respawned is under-reported. Metrics must take the `SUM` over the dir inside the
  time window.

## What exists today, and why none of it is the metric

| Piece | What it has | Why it is not enough |
|---|---|---|
| `runs/telemetry.jsonl` (`pipeline/phases/mod.rs:280-303`) | 81 rows, 2026-10-11..10-18, one schema `{ts,task,repo,builder,reviewer,review,rounds,review_changed_outcome,status,minutes}` | pipeline only, and it has no cost. It is built with `format!` and no JSON escaping (`:294-296`). No code reads it (grep: only docs) |
| `runs/quaqua.db` journal | `worker:<name>#k` workflows (`worker/prompt/mod.rs:557, :703`). The snapshot holds `input.kind, budget_sec, cost_budget, harness`, and `workflow_completed` holds `{exit, report_path}` | spend is never journaled: 0 `Cost` results in the live DB, because `CostTracker::check` runs outside the journal (`worker/prompt/mod.rs:411`). `prev_elapsed_ms` is clamped to the budget (`workflow.rs:239-240`), so it cannot show an overrun |
| `CostTracker` (`cost.rs:350-375`) | live dollars for opencode and omp | it reads only when a budget is set, stops once both warnings have fired, and keeps only the `.cost80/.cost100` marker files, which cleanup deletes (`state.rs:192-193`) |
| `quaqua usage` (`usage.rs`) | opencode-only audit: console request logs, meter, local DB | it is one harness and a network-backed report, not a per-worker log. Its `read_sessions` SQL is the model for the opencode token columns |
| `engine/telemetry_otel.rs` → `runs/traces.jsonl` | GenAI span sink | not wired: no caller outside `engine/`, and the file does not exist on the live workspace |
| CoS by hand (`templates/workspace/rules/_all/worker-timebox.md:4`, `AGENTS.md:44`) | SQL/grep at acceptance, then `cost=$y/$x` in memory | manual, and missing claude. P3 replaces this step |

## Data model: one `rec:"worker"` row

```json
{"ts":"2026-10-21T20:40:00+07:00","rec":"worker","name":"tieude-p74lock","kind":"tieude",
 "harness":"opencode","owner":"cos","pipeline":null,"repo":"~/quaqua",
 "prompts":1,"exit":0,"wall_sec":409,"budget_sec":1500,
 "usd":0.0458,"usd_budget":1.0,"tokens":{"in":94574,"out":16991,"reasoning":0,"cache_read":3754496,"cache_write":0},
 "spend_src":"opencode-db","model":"deepseek-v4.1-flash"}
```

| Field | Source (all read before `cleanup_worker_files`) |
|---|---|
| `name, owner` | the argument, and `QUAQUA_OWNER` / `.owner` (`cleanup.rs:102-104`). The owner is loaded today but only after the pane closes, which is fine because the files are still there |
| `harness, session, cwd` | the `herdr agent get` JSON already parsed at `cleanup.rs:69`: `agent`, `agent_session.value`, `cwd`. If the agent is already gone (`agent_not_found`), fall back to the journal snapshot's `harness` and `<repo>/../.wt/<name>` |
| `kind, budget_sec, usd_budget` | the latest `worker:<name>#k` snapshot `input.*` (live dump of `tieude-p74lock#1`) |
| `prompts, exit, wall_sec` | journal: the number of `#k` workflows in this lifecycle, the last `workflow_completed.exit`, and `ts` minus the first event `ts_ms` of `#1` |
| `pipeline` | `.pipeline` (`state.rs:34`), which links builder and reviewer rows to the task's `rec:"pipeline"` row |
| `usd, tokens, spend_src, model` | the extended `cost.rs` reader. `usd: null` with `spend_src:"claude-tokens"` for claude. The reader never guesses a price |

**Lifecycle fence.** A worker name can come back (the journal keeps counting `#k`, omp and claude
keep the old dirs and files). This lifecycle starts after the last `rec:"worker"` row with the same
`name`. Journal workflows and session-store entries with a timestamp before that fence are ignored.
omp and claude lines carry `timestamp`, and opencode rows carry `time_created`. This needs no new
state file.

**Failure rule.** Writing the row never fails or blocks cleanup. If a reader cannot read, that
field is `null` and `spend_src` names the reason. This matches the "never a kill switch" rule
(`worker-timebox.md:4`).

## Output: the minimum that is still useful

`quaqua status --metrics [--since 24h] [--owner <o>] [--json]`. Plain `quaqua status` stays as it
is, because the CoS runs it every turn (decision 2026-10-19).

```text
=== Metrics (runs/telemetry.jsonl, since 24h) ===
name              kind/harness       owner  exit  wall/budget  $/budget      tokens in/out/cache   pipeline
tieude-p74lock    tieude/opencode    cos    0     7m/25m       0.046/1.00    95k/17k/3.8M          -
detu-claudediff   detu/omp           cos    0     7m/40m       0.41/2.00     …                     -
truonglao-metrics truonglao/claude   cos    0     38m/40m      -/3.00        …                     -
pipelines: w2 clean r2 84m · w6 blocked r1 27m
total: 3 workers · $0.46 (+1 claude unpriced) · 52m
```

The columns are exactly what the CoS writes by hand today (`cost=$y/$x`), plus time vs budget and
tokens, so claude is not blank.

## Phases

| # | Change | Acceptance test (targeted, one per invariant) |
|---|---|---|
| M1 | `cost.rs`: `read_worker_spend(name, session, cwd, harness, since_ms) -> Spend{usd: Option<f64>, tokens: Option<Tokens>, src}`. opencode: `SUM` over the dir with token columns. omp: same lines, plus `message.usage` tokens. claude: dedupe by `message.id`. `read_worker_cost` = `.usd` | `cargo test --locked -p quaqua cost::spend`, with 3 fixtures: an opencode DB with 2 sessions in one dir (sum, not latest; **fails on today's `LIMIT 1`**), omp lines (tokens and dollars), and a claude transcript with duplicate ids (deduped total) |
| M2 | `cleanup_worker` appends the row (serde_json) before `cleanup_worker_files`. `append_telemetry` adds `"rec":"pipeline"` and uses serde_json | `cargo test --locked -p quaqua cleanup::metrics`: a fake herdr `agent get` and fixture stores give exactly one row with the expected fields. Every reader failing still cleans up and writes a row with nulls. A second lifecycle with the same name excludes the first one's spend |
| M3 | `quaqua status --metrics` reader | `cargo test --locked -p quaqua status::metrics`: a fixture with old rows (no `rec`) and new rows renders pipeline and worker sections and the totals. `--json` round-trips |
| M4 | docs: `worker-timebox.md:4` and `AGENTS.md:44` point to `quaqua status --metrics` instead of hand-written SQL/grep (one home per fact) | none (docs) |

M1 → M2 → M3 run in order and each lands on its own. M4 lands with M3.

## Rejected

- **A `metrics` table in `runs/quaqua.db`.** The DB is the engine's event/snapshot journal
  (`engine/journal.rs:6`), and `status.rs:91-99` checks its schema. Rows written by a non-workflow
  writer would mix two lifetimes. JSONL is already the pattern, and the CoS can `jq` it without the binary.
- **Journaling spend as a `Cost` activity in the worker workflow.** That makes every wake pay for a
  store read, and it still misses whatever happens after the last `prompt` (resume, manual turns).
  Cleanup is the one point every worker reaches.
- **Writing at the end of `worker prompt`.** One worker gets many prompts (`#k`) and `--resume`
  calls, and Leads sometimes prompt by hand, so there is no single "end".
- **Wiring `engine/telemetry_otel.rs` (`traces.jsonl`).** That is per-span GenAI tracing, which is
  more than we need, and it needs a call per model request that quaqua cannot see.
- **A daemon or service.** Rejected in `event-monitor.md` §Rejected.

## Risks

- **Store formats drift.** omp and claude transcripts are not a stable API. The omp reader already
  falls back to "first `cost.total` anywhere" (`cost.rs:70-97`). Do the same for the claude
  `usage` keys and log `spend_src` so a drift shows up as `null` rather than a wrong number.
- **Concurrent appends.** Several cleanups at once (pipeline plus CoS). Use one `write_all` of one
  line under 4 KiB with `O_APPEND`, the same as `append_telemetry` today. Not proven atomic on
  every filesystem (Unknowns).
- **Workers that are never cleaned get no row.** `quaqua status` already flags `DONE-NOT-CLEANED`,
  so the gap is visible, not silent.
- **Claude tokens priced as zero.** `usd:null` must render as `-` and be counted apart in the total,
  never summed as 0.

## Unknowns

- Whether `herdr agent get` returns `agent_session` for omp and opencode as `kind=id` or
  `kind=path`. Only claude was observed live (`kind=id`). `watchdog-event-driven.md` says omp is
  `kind=path`. Check: `herdr agent get <omp-worker>` during M1.
- Whether a pipeline's builder and reviewer always go through `cleanup_worker` (the
  `pipeline/mod.rs:63` header says "reviewers are cleaned right after… the caller cleans the
  builder"). Not traced to the call site in this pass.
- Whether a single `O_APPEND` write is atomic for concurrent writers on APFS. Not tested.
- Claude USD: Claude Code here may be on a subscription, so a token price could be notional. This is
  left to #48.

## Questions for anh iu

1. **Claude dollars:** token-only (`usd:null`) in P3, with a price table decided in #48? (recommended)
   Or add a list-price table now?
2. **One file:** reuse `runs/telemetry.jsonl` with `rec` (recommended), or start a new
   `runs/metrics.jsonl`?
3. **Plain `quaqua status`:** add a one-line 24 h total footer, or keep it unchanged and use only `--metrics`?
4. **The opencode `LIMIT 1` under-count:** fix it in M1 (it changes what the 80%/100% warnings see), or
   leave `CostTracker` as it is and use the new sum only for metrics?
