# quaqua layouts, runtime gate, mail, events — design

Related: [`plan.md`](plan.md) (decisions + P0 addendum) · [`research-piggery.md`](research-piggery.md) (piggery evidence) · [`event-monitor.md`](event-monitor.md) (absorbed by §D / P2+P6) · [`metrics.md`](metrics.md) (absorbed by §D `farm_events`) · index [`README.md`](README.md)

Design only. Nothing in `~/Lab/quaqua` or `~/quaqua` was edited except this report. Line citations are
against engine `c99261a` (worktree `quaqua/truonglao-layout`) unless they say `slpcfg` (branch
`origin/quaqua/tieude-slpcfg`, PR #113). piggery claims come from
`research-piggery.md` ("research §n").

## Playbook

1: done. Read the research report, `config.rs`, `cap.rs` (c99261a and slpcfg), `worker/spawn.rs`, `lead/{spawn,prompt}.rs`, `taskforce/mod.rs`, `worker/prompt/{mod,workflow,observer}.rs`, `engine/{journal,journal_sqlite}.rs`, `status.rs`, `grid/hook.rs` and `plugins/herdr-quaqua/herdr-plugin.toml`. Also the live `~/quaqua/team.toml`, `runs/*.inbox.md` and `runs/quaqua.db`, and the herdr 0.9.5 CLI and API schema.
2: done. Overview → Key concepts → A/B/C/D (each covers data model, placement, failure modes and what stays) → two sequences → rollout → open decisions → Where things live → Gotchas. Tradeoff tables appear where the brief asks for a choice.
3: done. Every Evidence line was re-checked with a script. `Verified: high c99261a` is in the Verification section.

## Overview

The verdict, in short:
- Build all four parts as **verbs over files plus one SQLite file quaqua already owns**. No daemon is needed: herdr is already the daemon, and its plugin `[[events]]` hook is the push channel.
- I disagree with the research report on one point. Mail and events belong in **new tables in `runs/quaqua.db`**, not in jsonl and not in the engine's `events` table. The reasons are in §C.
- Freeze **topology only** per project. Keep kinds and models live: re-resolving `team.toml` at every spawn is deliberate (dynamic-config rule, the "Claude out" profile).

What the code shows today:
- **The cap is the only spawn rule that is enforced.**
  - At c99261a: `worker/spawn.rs:363-369` and `taskforce/mod.rs:112-124`.
  - On slpcfg: `cap::check_owner_cap` (`slpcfg cap.rs:70-89`), called from `slpcfg spawn.rs:360` and `slpcfg taskforce/mod.rs:119`, with `roles.lead.max_peers`.
- **`can_spawn`, `can_prompt` and `limits.depth` are declared but never read.** They are parsed and validated (`config.rs:158-164`, `:97`, `:491-505`), and nothing outside `config.rs` reads them (verify §1).
- **The Lead→CoS inbox has no reader or writer in code.** It is a markdown convention (`templates/workspace/rules/_all/lead-inbox.md:3-5`), and `grep -rn inbox crates/` finds nothing (verify §2). The live inboxes hold 264 lines across 5 files.
- **The watchdog wakes on a heartbeat of up to 60 s** (`worker/prompt/mod.rs:502-506`, `workflow.rs:474,533`) and checks the report file, the diff and the session store.
- **Caller identity fails open.**
  - `resolve_owner` defaults to `cos` when `QUAQUA_OWNER` is unset (`spawn.rs:257-265`).
  - A worker pane is skipped as a caller (`spawn.rs:237-239`). So a Peer that runs `quaqua worker spawn` is counted as the CoS.
  - Taskforce reads the env only (`taskforce/mod.rs:101`).
  - `quaqua lead spawn` writes no `runs/<name>.owner` (verify §9), so there is no depth chain to walk.

## Key concepts

| Term | Meaning |
| --- | --- |
| **Layout** | A named topology: roles (each with a class `cos/lead/peer/taskforce`), allow-edges per verb, `reports_to`, depth, per-role child caps. It never contains models: roles point to `purposes` and `kinds`, which stay live. |
| **Legacy synthesis** | When `team.toml` has no `[layout]`, the binary builds the `slp` layout from today's `[roles.*]` and `[limits]`, so a v2 file means exactly what it means now. |
| **Snapshot** | `runs/<slug>.layout.toml`, written by `quaqua lead spawn`: the resolved layout plus its sha256. A running team keeps its topology when `team.toml` changes. |
| **Gate** | `layout::check(layout, caller, verb, target) -> Decision` is one pure function, called at every spawn or prompt verb quaqua mediates and also by `quaqua layout why`. It is not `gate/` (that is the PreToolUse shell classifier, `gate/policy.rs:1-11`), which stays as-is. |
| **Caller** | `{name, role, depth, source}`, taken positively from herdr pane tokens. "No tokens" is never read as `cos`. |
| **Bus** | Two new tables, `mail` and `farm_events`, in `runs/quaqua.db`, opened with the journal's connection setup. |
| **Incident** | A `farm_events` row with a `UNIQUE incident_key`. One insert gives one notice, enforced by the database. |

## A. Layout manifest

**Data model** (`crates/quaqua/src/layout/model.rs`):

```rust
pub struct Layout { pub name: String, pub roles: BTreeMap<String, LayoutRole>, pub depth: u32 }
pub struct LayoutRole {
    pub class: RoleClass,               // Cos | Lead | Peer | Taskforce (today's ROLE_CLASSES, config.rs:15)
    pub can_spawn: Vec<String>,         // role names, allow-list; no match = deny
    pub can_prompt: Vec<String>,        // also governs `mail send` to that role
    pub reports_to: Option<String>,     // role name | "owner"; implies a mail edge upward
    pub max_children: Option<i64>,      // absorbs roles.lead.max_peers (slpcfg config.rs) / limits cap
    pub naming: Naming,                 // SlugLead ("<slug>-lead") | KindTask ("<kind>-<task>") — load-bearing for grid
    pub members: Vec<Member>,           // taskforce-class only: purpose/kind per seat (advisor=1, dual-lens=2, council=N)
}
```

`team.toml` v3 (additive):

```toml
schema_version = 3
[layout]
use = "slp"                          # built-in name, or a key of [layouts.*]
[layouts.review-council]             # user-defined; same shape as a built-in
depth = 2
[layouts.review-council.roles.cos]   { class = "cos",  can_spawn = ["chair"], can_prompt = ["chair"] }
[layouts.review-council.roles.chair] { class = "lead", can_spawn = ["council"], reports_to = "cos", max_children = 3 }
[layouts.review-council.roles.council] { class = "taskforce", members = [{purpose="review"},{purpose="review"},{purpose="advise"}] }
```

**Built-ins** (`include_str!` of `templates/layouts/<name>.toml`, same mechanism as `config.rs:408`):

| Layout | Shape |
| --- | --- |
| `slp` | Today's live graph: `cos→{lead,peer,taskforce}`, `lead→{peer,taskforce}`, `peer→∅`, depth 2, `lead.max_children=4` (`~/quaqua/team.toml:23-33`, slpcfg default). |
| `lead-peer` | A single Lead owned by the CoS spawns Peers. The CoS cannot spawn Peers directly. |
| `advisor` | A taskforce role with 1 member; the caller reads the report. Replaces the hardcoded mode (`taskforce/mod.rs:26-31,144`). |
| `dual-lens` | A taskforce role with 2 members, chaired by the caller (`taskforce/mod.rs:208`). |
| `council` | A taskforce role with N members (3–5) and a quorum. It is new; `TaskforceWorkflow` is fixed at 1 or 2 members today (`taskforce/mod.rs:924`), so it needs generalizing. |

**Legacy synthesis.** `Layout::from_legacy(&cfg)` keeps `ROLE_NAMES` (`config.rs:17`) as the legacy role set and maps each `[roles.r]` to a `LayoutRole`. The mapping covers:
- `can_spawn` and `can_prompt`, copied as they are;
- `reports_to`;
- `[limits].depth`;
- `roles.lead.max_peers` → `lead.max_children`;
- `limits.max_workers_per_orchestrator` → `cos.max_children`.

A v2 file then resolves to the same graph the docs already describe, and `validate()` for v2 is unchanged.

**Per-project snapshot.**
- `quaqua lead spawn` writes `runs/<slug>.layout.toml` (layout + `sha256` + `source=team.toml@<mtime>`) using `state::atomic_write` (`state.rs:9-20`).
- The gate reads the snapshot for any caller whose owner chain reaches `<slug>-lead`, and the live file for CoS-owned work.
- `quaqua layout refreeze <slug>` is the explicit way to apply a `team.toml` edit to a running team.
- Freezing is cheap: a full live resolve of the 2.9 kB `team.toml` measures under 10 ms (`quaqua config get` three times, 0.00 s each). The real cost is semantic, so the snapshot holds **topology only**. Kinds, models, timers and caps stay live (the dynamic-config rule).

**`quaqua config migrate`.**
- It rewrites line by line, the same way piggery's `MigrateKey` does (research §1). The rewrite:
  1. changes `schema_version = 2` to `3`;
  2. inserts `[layout] use = "slp"` when `[roles.*]` matches the `slp` built-in, and otherwise moves the `[roles.*]` tables to `[layouts.custom.roles.*]`.
- Comments are kept and no TOML-editing dependency is added (the dependency pins are at `crates/quaqua/Cargo.toml:7-14`).
- It runs as a dry run by default and prints a diff. `--write` keeps `team.toml.bak` and then calls `atomic_write`.
- A `schema_version` above 3 is refused with a migrate hint and never silently downgraded.

**Failure modes.**
- **Rollback hazard:** `validate()` accepts exactly `2` (`config.rs:443`), and `Config` has `deny_unknown_fields` (`config.rs:28`). A v3 file therefore breaks every older `quaqua` on PATH, including the plugin's `quaqua grid hook`. Mitigations:
  - Phase 1 ships a reader for 2 and 3.
  - `migrate` is opt-in.
  - The embedded template (`config.rs:408`) stays v2 until `quaqua doctor` reports every binary as v3-capable.
- **Snapshot drift:** `quaqua layout show <slug>` prints `frozen sha ≠ live sha`. That is a warning, never a block.
- **Unknown built-in name:** a hard error at load time (first failure wins, `config.rs:1-5`).

**What stays as-is:** `[kinds]`, `purposes`, `strategy`, `[taskforce.<mode>]` member kinds, `<slug>-lead` naming, grid tab derivation (`spawn.rs:203-220`), and the `config.rs` resolve order (`:385-424`).

## B. Runtime gate

**Placement** (`crates/quaqua/src/layout/{gate.rs,identity.rs}`): one pure `check`, called from each choke point. `spawn_worker` has 7 callers (verify §3), so a check at verb entry alone would miss pipeline and engine spawns.

| Choke point | Where | Check |
| --- | --- | --- |
| Every worker spawn | inside `spawn_worker` right after `resolve_owner` (`spawn.rs:358`), next to the cap check; new `origin` arg `Cli/Pipeline/Activity/TaskforceMember` | `Spawn → peer`. A `TaskforceMember` skips the edge check (already done by the taskforce) but still counts toward caps. |
| Fallback relaunch | `spawn.rs:587` | `check_spawn_policy` only. The role is the same, so the layout is not re-checked. |
| Lead spawn | `lead/spawn.rs:257-264`, before `workspace create` (`:278`) | `Spawn → lead` |
| Taskforce | `handle_taskforce` (`taskforce/mod.rs:99-124`), before the weight and cap check | `Spawn → taskforce` (+ member count from the layout) |
| Worker prompt | `quaqua worker prompt` entry (`worker/prompt/mod.rs`) | `Prompt → target`: allowed when the caller owns the target (`runs/<name>.owner`) or has a `can_prompt` edge to the target's role |
| Lead prompt | `lead/prompt.rs:22` | `Prompt → lead` |
| Mail send | new `quaqua mail send` | `Mail → to`: a `can_prompt` edge or `reports_to` |

**Order of checks**, as layer and `rule_id`:
1. `identity` (`identity.unknown`)
2. `role` (`role.unknown`)
3. `edge` (`edge.none`)
4. `depth` (`depth.exceeded`)
5. `cap` (`cap.exceeded`; this layer replaces the `check_owner_cap` call and keeps its source string)

`check_spawn_policy` (`config.rs:640-654`) stays a separate kind-level gate.

**Identity** (`identity.rs`). Today's pane-token lookup in `caller_pane_owner` (`spawn.rs:227-245`) is reused and turned positive:

| Pane tokens of `$HERDR_PANE_ID` | Caller |
| --- | --- |
| `grid_role=worker`, `grid_owner=O` | `peer` (or `taskforce` when a `runs/taskforce-*.state` lists it), depth = depth(O)+1 |
| `grid_role=orchestrator`, `grid_owner=<slug>-lead` (`lead/spawn.rs:317-327`) | `lead`, depth = depth(`runs/<slug>-lead.owner`)+1 |
| `grid_role=cos` (**new**, reported once by the CoS: open decision D3) | `cos`, depth 0 |
| no pane / no tokens | `identity.unknown`: allowed with a warning in shadow mode, denied in enforce mode. `QUAQUA_TEST=1` falls back to env. |

Depth comes from walking `runs/<name>.owner`. The walk is bounded at `depth+1` and is cycle-safe. Phase 0 makes `lead spawn` write `runs/<slug>-lead.owner` (`WorkerMetadata::save_owner`, `state.rs:92`). Pipelines and engine activities run in the calling Lead's process and pane, so they inherit the Lead's identity.

**Deny shape.** The exit code stays 1, as the cap error is today (`spawn.rs:365-370`). The output is one stderr line plus JSON under `--json`, modelled on piggery (research §2):

```
error: denied by layout slp@3f2a…: lead 'mu-lead' cannot spawn role 'lead' (edge.none; lead.can_spawn = [peer, taskforce])
{"code":"denied","verb":"spawn","caller":"mu-lead","caller_role":"lead","target_role":"lead",
 "rule_id":"edge.none","layer":"edge","layout":"slp","layout_sha":"3f2a…","source":"snapshot"}
```

Every decision appends a `farm_events` row: a deny writes `denied`, an allow writes `spawned` carrying the `rule_id`.

**`quaqua layout why <caller> <verb> <target>`** resolves the caller by name from `runs/<name>.owner` and the pane tokens, runs the **same** `check`, and prints the trace, one line per layer, with ✓/✗. It never writes anything. `quaqua layout show [--project <slug>]` prints the effective layout and where it came from.

**Failure modes.**
- **Fail-closed lockout:** this is why there is a shadow phase. It follows the hook lesson: block the guarded set and allow+warn the rest.
- **Herdr down:** identity becomes unknown. In enforce mode the spawn is denied, which is safe because a spawn needs herdr anyway.
- **Bypass:** a Lead can still run `herdr agent prompt` on a Peer directly. Rules must say "enforcement covers what quaqua mediates" (research §5). The PreToolUse gate could later classify `herdr agent prompt` as Ask; that is out of scope here.

**What stays as-is:** `gate/` (PreToolUse), the `check_spawn_policy` contract, the cap message text and its sources, and the grid placement code.

## C. Inter-team / worker mail

**Decision: new `mail` table in `runs/quaqua.db`.**

| Option | For | Against |
| --- | --- | --- |
| jsonl `runs/<slug>.mail.jsonl` (research pick) | greppable, no schema | A `cat >>` cannot assign `seq`, so a `quaqua mail send` verb is needed anyway, plus `flock` for seq. Ack needs a second file and a join. Atomic appends over a size limit are not guaranteed. Once a verb exists, the grep advantage mostly goes away. |
| reuse engine `events` table | already exists | The PK `(workflow_id, seq)` gives no global cursor (verify §11). `status.rs:332-338,372-378` groups **every** `workflow_id` with no prefix filter, so mail streams would show up as workflows in `quaqua status`. Replay semantics would be mixed into mail, and each insert costs a `MAX(seq)` scan (`journal_sqlite.rs:518`). |
| **new tables, same db file** ✅ | Global `AUTOINCREMENT` seq, `UNIQUE` idempotency key, atomic ack, WAL multi-writer safety already proven by tests (`journal_sqlite.rs:833,870`). Reuses the opener (`:254-308`: `busy_timeout`, WAL, `retry_while_busy`). | A schema to own. Mitigation: `quaqua mail ls --md` renders the old inbox view for grep. |

```sql
CREATE TABLE IF NOT EXISTS mail (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,   -- global seq; the cursor
  msg_key     TEXT NOT NULL UNIQUE,                -- idempotency: sha256(from|client_id), client_id defaults to sha256(body)
  ts_ms       INTEGER NOT NULL,
  from_name   TEXT NOT NULL, from_role TEXT NOT NULL,
  to_name     TEXT NOT NULL,                       -- 'cos' | '<slug>-lead' | worker name
  kind        TEXT NOT NULL,                       -- handoff|accept|challenge|escalate|blocked (lead-inbox.md:4) | note
  project     TEXT, reply_to INTEGER REFERENCES mail(id),
  body        TEXT NOT NULL CHECK (length(body) <= 4096),
  deliveries  INTEGER NOT NULL DEFAULT 0, last_delivered_ms INTEGER,
  acked_ms    INTEGER, acked_by TEXT
);
CREATE INDEX IF NOT EXISTS mail_unacked ON mail(to_name, acked_ms, id);
```

**Verbs** (`crates/quaqua/src/bus/mail.rs`):

| Verb | Behaviour |
| --- | --- |
| `quaqua mail send --to <name> --kind <k> [--body-file -]` | Runs the gate (`Mail`), inserts the row with `INSERT … ON CONFLICT(msg_key) DO NOTHING`, then prints `mail id=<n>`. A retry after a crash is a no-op. |
| `quaqua mail inbox [--for <name>]` | Prints the unacked rows, bumps `deliveries`, and marks any repeat as `(redelivery #n)`. |
| `quaqua mail ack <id>…` | Sets `acked_ms` only if the row is still unacked and addressed to the caller. |
| `quaqua mail import runs/<slug>-lead.inbox.md` | Parses the `## <ISO> <lead> <kind>` headers into rows. It is idempotent (`msg_key` = hash of the header line). |

Redelivery means an unacked row keeps showing up on every read until it is acked. When a row stays unacked past `timers.mail_ack` (default 30 m), a `mail_unacked` incident is written once (§D).

**Failure modes.**
- **DB locked:** `busy_timeout` 5 s, then exit 1 with the path. The sender retries, which is safe because of `msg_key`.
- **DB corrupt or missing:** `send` falls back to appending the markdown inbox and warns. That keeps the report even when the store is down.
- **Lead dies after send but before the CoS reads:** the row persists. That is the whole point.

**What stays as-is:**
- `quaqua lead prompt`, the CoS → Lead direction (`lead/prompt.rs:1-8`).
- `runs/<name>.md` as the worker completion signal.
- During phase 5, `mail send` **also** appends to `runs/<slug>-lead.inbox.md`, so the CoS's current reading habit keeps working until the rule is changed.

## D. Event-driven monitoring

**Data model** (`crates/quaqua/src/bus/events.rs`, same db, a different name from the journal's `events`):

```sql
CREATE TABLE IF NOT EXISTS farm_events (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_ms        INTEGER NOT NULL,
  type         TEXT NOT NULL,     -- spawned|spawn_failed|denied|would_deny|prompted|report_written|stalled|
                                  -- budget_exceeded|blocked|agent_exited|malformed|mail_sent|mail_acked|mail_unacked|cleaned
  actor TEXT, subject TEXT, owner TEXT, project TEXT,
  incident_key TEXT UNIQUE,       -- NULL = plain fact; set = notice-worthy, inserted at most once
  payload_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS farm_events_owner ON farm_events(owner, seq);
CREATE TABLE IF NOT EXISTS cursors (consumer TEXT PRIMARY KEY, seq INTEGER NOT NULL);
```

**Writers.** No daemon: every source already has a process that runs at the right moment.

| Event | Writer |
| --- | --- |
| `spawned` / `denied` / `would_deny` / `spawn_failed` | the gate inside `spawn_worker`, `lead spawn`, `taskforce` |
| `report_written`, `stalled`, `budget_exceeded`, `blocked`, `malformed` | `quaqua worker prompt` at its exit paths 0/2–6 (`worker/prompt/mod.rs:1043-1090`) |
| `agent_exited`, `blocked` for an unwatched pane | **herdr plugin hook**: a new `[[events]]` entry, `on = "pane.agent_status_changed"` / `"pane.exited"`, with command `["quaqua","events","hook"]`, reading `HERDR_PLUGIN_EVENT_JSON` the way `grid/hook.rs:386-387` already does |
| `mail_*` | the mail verbs |
| `cleaned` | `worker cleanup` / `lead cleanup` |

`incident_key` examples: `stall:<name>:<budget_epoch>`, `blocked:<pane>:<status_since>`, `mail_unacked:<id>`, `deny:<caller>:<rule>:<target>:<hour>`. An insert uses `INSERT OR IGNORE`, and a notice fires only when `changes() = 1`. That gives one notice per incident by construction. The watchdog's `.nudged`, `.grace` and `.autoext` markers (`state.rs:170-177`) stay as they are: they are internal timer state, not notices.

**Consumers: push or cursor.**

| Consumer | Mechanism | Why |
| --- | --- | --- |
| **CoS agent** | Cursor. `quaqua inbox` at turn start prints unacked mail plus incidents since `cursors['cos']`, then advances the cursor. `--follow` re-reads SQLite every 2 s. | Pushing into the CoS pane is barred (`lead-inbox.md:3`: a prompt lands in anh iu's chat). |
| **anh iu (human)** | Push via `herdr notification show "<title>" --body … --sound request` (herdr 0.9.5 CLI), sent only on an incident insert of type `escalate`, `blocked` or `mail_unacked` | One-shot, deduped by the key. Open decision D7. |
| **Watchdog** | Keeps the heartbeat, but sleeps inside the existing activity with `herdr agent wait <name> --until blocked --until done --timeout <poll_ms>` **only while the agent is `working`**. A move to blocked or done then wakes it at once instead of up to 60 s later. A file write (`runs/<name>.md`) is not a herdr event, so the report itself is still found by the heartbeat. A cheap `stat` can be split from the expensive diff and session scan. | Exit 5 (blocked) is the case where latency hurts, because a question waits on a human. `agent wait` matches the current state (handoff rule), so calling it when the agent is already idle would spin. |
| **Lead** | Same as the CoS: `quaqua inbox --for <slug>-lead` | Leads already run `worker prompt`, which keeps working. |

**Can it be done without a daemon?**
- Yes for every row above. herdr has `events.subscribe` and `events.wait` on its socket (schema protocol 22; subscriptions include `pane.agent_status_changed`, `pane.exited` and `pane.closed`, verify §12). However, quaqua has no socket client (`herdr.rs` has no `UnixStream`, verify §6), and the CLI exposes only `agent wait`. The plugin hook already delivers the same events to a short-lived `quaqua` process, so a socket client is not needed.
- One gap stays: a `stalled` incident for a worker **nobody is watching** needs a clock. That is true today as well, and it is accepted here. If it ever matters, the herdr hook on `pane.agent_status_changed` can at least record `idle`/`done` transitions.

**Failure modes.**
- **Hook slow or failing:** the hook must exit 0 within a hard timeout (hook lessons) and never block herdr. A missing event costs a late notice, never a wrong decision: decisions are still made by the verbs.
- **Event loss on DB error:** writers log to stderr and carry on. Events are audit; decisions never read them.
- **Cursor ahead of a rolled-back row:** this cannot happen, because `AUTOINCREMENT` is assigned at commit inside one transaction.

**What stays as-is:** the exit-code contract 0–6, budget, grace and auto-extend logic (`worker-timebox.md`), the report file as the completion signal, and the journal `events` and `snapshots` tables.

## Sequences

**1. A Lead spawns one Peer** (`mu-lead` runs `quaqua worker spawn tieude tieude-x …`):
1. `load_team_config()` resolves kinds live (`config.rs:418`). The layout comes from the snapshot `runs/mu.layout.toml`, because the owner chain reaches `mu-lead`.
2. `identity::resolve`: pane tokens `grid_role=orchestrator`, `grid_owner=mu-lead` → `Caller{mu-lead, lead, depth 1}`, since `runs/mu-lead.owner` = `cos`.
3. `check_spawn_policy(tieude)` passes (`spawn.rs:336`).
4. `layout::check(Spawn, peer)`:
   - edge `lead→peer` ✓;
   - depth 2 ≤ 2 ✓;
   - cap: `count_owner_slots` = 2 < `lead.max_children` 4 ✓;
   - result: `Decision::allow("slp.lead.can_spawn.peer")`.
5. Unchanged steps: worktree, `pane split`, the tokens (`spawn.rs:488-492`), `save_owner` (`:425`) and the grid hook (`:613`).
6. A `farm_events` row is written: `spawned{actor=mu-lead, subject=tieude-x, owner=mu-lead, project=mu, rule}`.

On a deny, step 4 returns `edge.none` and the spawn exits 1 before any worktree or pane exists, writing a `denied` row with an incident key so a repeated deny gives one notice per hour.

**2. A Lead reports to the CoS** (`mu-lead` finishes acceptance):
1. `quaqua mail send --to cos --kind accept --body-file - <<'EOF' … EOF`
2. Identity → lead. `check(Mail, cos)` passes on `lead.reports_to = cos`.
3. In one transaction: `INSERT mail` (deduped by `msg_key`) and `INSERT farm_events(mail_sent)`. During phase 5 the same entry is also appended to `runs/mu-lead.inbox.md`. Output: `mail id=42`.
4. Only for `kind ∈ {escalate, blocked}`: incident `mail:42` → one `herdr notification show`.
5. The CoS, at its next turn, runs `quaqua inbox`, which shows `#42 mu-lead accept …` and advances `cursors['cos']`. After acting, the CoS runs `quaqua mail ack 42`.
6. If #42 is not acked after 30 m, the next `quaqua inbox` or any other mail verb lazily inserts the `mail_unacked:42` incident once, and #42 keeps showing with `(redelivery #n)`.

## Phased rollout

Every phase ships on its own, `templates/workspace/team.toml` (v2) and a copy of the live `~/quaqua/team.toml` keep loading in every phase's tests, and the tests are targeted (`cargo test --locked -p quaqua <filter>`). One test per invariant, no wording tests.

| # | Phase | Size | Proving tests (one per invariant) | Risk |
| --- | --- | --- | --- | --- |
| P0 | Identity + depth plumbing. `lead spawn` writes `runs/<slug>-lead.owner`. `identity::resolve` is positive: no tokens gives `Unknown`. No behaviour change: identity is only computed and logged. | S, ~150 LOC | resolve over fake pane tokens: worker, orchestrator, cos, none → peer, lead, cos, unknown. The depth walk stops at a cycle. | `lead/spawn.rs` and the herdr mocks need the test-isolation env (`env -i`, rules) |
| P1 | `layout/` model, built-ins, `from_legacy`, reader accepts schema 2 and 3, `quaqua layout show` / `why` (read-only). Absorbs PR #113's `max_peers`. | M, ~450 | `from_legacy(v2 template)` equals the `slp` built-in edge set. A v3 `use="slp"` equals the legacy one. v4 is refused with a migrate hint. `why` returns the same `rule_id` as `check`. | Merge order with PR #113 (it touches `config.rs`, `cap.rs`) |
| P2 | Bus foundation: shared connection opener pulled out of `SqliteJournal::open`. `farm_events` and `cursors` tables. Writers at the spawn and watchdog exits. | S–M, ~300 | Concurrent inserts give monotonic `seq`. A duplicate `incident_key` inserts once. `quaqua status` output is unchanged with the new tables present. | `journal_sqlite.rs` belongs to engine-lead E1 (`engine/OWNERS.md`), so the opener extraction needs its sign-off |
| P3 | Gate in **shadow** at all 7 spawn sites plus lead spawn, taskforce, worker prompt and lead prompt. A deny would print `would_deny` and write an event, but never block. Shadow is the default (precedent: `grid/hook.rs:24` `is_shadow_mode`). | M, ~350 | For each choke point, a test where the SLP fixture denies `peer→spawn` while the spawn still succeeds and a `would_deny` row exists. | Noise from legacy callers with no tokens; check `would_deny` counts before P4 |
| P4 | Gate **enforce**: flip the default, keep the `QUAQUA_LAYOUT_SHADOW=1` escape, move the `check_owner_cap` call into the `cap` layer. | S, ~100 | A peer spawn is denied with exit 1 and leaves no worktree or pane. The CoS→lead spawn is allowed. The cap message source is preserved. | A lockout if CoS identity is wrong (D3), so ship only after P3 shows 0 false denies |
| P5 | Mail: `send`, `inbox`, `ack`, `import`, `ls --md`, plus dual-write to the md inbox. `rules/_all/lead-inbox.md` → "use `quaqua mail send`" (managed template). | M, ~450 | A retried send gives one row. Ack by a non-recipient is refused. Unacked mail is redelivered with the count. Import of the 5 live inbox files is idempotent. | Leads trained on `cat >>`; the md fallback keeps the old way working |
| P6 | Consumers: `quaqua inbox` digest plus cursor, `herdr notification` on incidents, a plugin `[[events]]` entry with `quaqua events hook`, and the watchdog `agent wait` sleep while the agent is working. | M, ~400 | Exactly one notification per incident key across repeated hook calls. Watchdog exit 5 latency is under 5 s on a mocked blocked transition (virtual clock, `engine/testkit.rs`). A cursor read is idempotent. | Hook latency inside herdr; the plugin smoke must be isolated (HOME/XDG, unset `HERDR_SOCKET_PATH`) |
| P7 | Freeze: `runs/<slug>.layout.toml` at lead spawn, `layout refreeze`, drift warning in `layout show`. `quaqua config migrate` (dry run / `--write`). | M, ~350 | A `team.toml` edit after lead spawn does not change that team's decisions. `refreeze` applies it. `migrate` on the v2 template is byte-identical except the version line and `[layout]`, and the result validates. | Operators surprised that edits do not apply. `layout show` prints the frozen/live sha. |
| P8 | User layouts plus `council` (generalize `TaskforceWorkflow` to N members and a quorum). | L, ~650 | A 3-member council reaches quorum at 2/3 and fails cleanly at 1/3. A user layout with a `can_spawn` cycle past `depth` is denied. | `taskforce/mod.rs` belongs to worker-lead K4 (`engine/OWNERS.md`) |

Before any `team.toml` value changes in P1, P4 or P7: grep `crates/quaqua/tests/*.rs` and `tests/*.sh` for it (rules).

## Open decisions for anh iu

| # | Decision | Recommendation |
| --- | --- | --- |
| D1 | What freezes per project: topology only, the whole config, or nothing? | **Topology only.** Kinds and models stay live for the "Claude out" switch. |
| D2 | Which layout governs CoS-owned workers (no `<slug>`)? | **Live** `team.toml`. There is no team to freeze for. |
| D3 | The positive CoS identity marker | **A pane token `grid_role=cos`**, written by `quaqua cos tag` once per CoS session, rather than `runs/cos.pane` or env. Without it, P4 cannot fail closed. |
| D4 | Mail store | **SQLite tables in `runs/quaqua.db`**, overriding the research report's jsonl pick (§C) |
| D5 | Deny exit code | **1**, the same as the cap and kind-policy errors today, with `rule_id` in the message and JSON |
| D6 | Enforce on legacy v2 files, or only when `[layout]` exists? | **Shadow, then enforce for both.** The current `can_*` values are what the docs promise. |
| D7 | Desktop notices via `herdr notification show` for escalate, blocked and unacked mail | Yes, deduped by incident. Say no if it is noisy. |
| D8 | How long the dual-write to `*.inbox.md` lasts | Two weeks after P5, then a rule change and `mail ls --md` as the view |
| D9 | `council` quorum and chair | A majority quorum, with the caller as chair (like dual-lens today) |

## Where things live

| New | Path |
| --- | --- |
| Layout model, built-ins, legacy, snapshot | `crates/quaqua/src/layout/{mod,model,builtin,legacy,snapshot}.rs`, `templates/layouts/*.toml` |
| Gate, identity, `layout why`/`show` | `crates/quaqua/src/layout/{gate,identity}.rs`; dispatch in `main.rs` |
| Mail, events, cursors | `crates/quaqua/src/bus/{mod,mail,events}.rs`; opener shared with `engine/journal_sqlite.rs` |
| Plugin hook entry | `plugins/herdr-quaqua/herdr-plugin.toml` (+ `[[events]]`) |
| Rules text | `templates/workspace/rules/_all/lead-inbox.md`, `templates/workspace/AGENTS.md:34` (P5, one home per fact) |

## Gotchas

- **The `gate/` name is taken.** Name the new module `layout::gate`, and never extend the PreToolUse classifier for this: it sees shell text, not roles (research §2).
- **Taskforce mutates process env.** It runs `std::env::set_var("QUAQUA_OWNER", …)` (`taskforce/mod.rs:179,274`), so in-process callers see a changed env. Identity must come from pane tokens plus an explicit `origin`, never from env.
- **`spawn_worker` excludes `-lead` names** (`spawn.rs:146`). Lead spawn is a separate code path, so the gate has to be wired there too.
- **The research report says the cap is not enforced. That is wrong:** it is enforced at c99261a and on slpcfg (Overview). The layout `cap` layer generalizes it and does not add a second count.
- **`agent wait` has the stale-idle trap** (handoff rule). Use it only to wait for a change away from the current `working` state.
- **The runner replays by `workflow_id`.** Putting non-workflow rows into `events` would pollute replay and the status listing. The bus uses its own tables.

## Verification

Script: `/tmp/claude-1000/-home-tinhpt-Lab--wt-truonglao-layout/859dc9bf-2690-43d2-823f-1b1a7adf489f/scratchpad/verify-layout.sh <engine-checkout>`. Run against `c99261a`, it exited 0, and its output is saved at `/tmp/claude-1000/-home-tinhpt-Lab--wt-truonglao-layout/859dc9bf-2690-43d2-823f-1b1a7adf489f/scratchpad/verify-layout.out`:

| Section | What it checks | Result |
| --- | --- | --- |
| §1 | `can_spawn` / `can_prompt` / `depth` readers outside `config.rs` | NONE |
| §2 | `inbox` in `crates/` | NONE |
| §3 | callers of `spawn_worker` | 7, listed |
| §4–5 | strict `schema_version` check and `deny_unknown_fields` on `Config` | `config.rs:443`, `config.rs:28` |
| §6 | herdr socket client | NONE |
| §7 | `GROUP BY workflow_id` without a filter | `status.rs:337,377` |
| §8 | sources of `QUAQUA_OWNER` | listed |
| §9 | `.owner` write in `lead/spawn.rs` | NONE |
| §10 | plugin `on =` events and hook env | listed |
| §11 | live journal schema | PK `(workflow_id, seq)`, 49,979 events / 140 workflows |
| §12 | herdr 0.9.5 subscribe/wait vocabulary | 27 subscription types, 19 wait events |
| §13 | slpcfg `check_owner_cap` | `cap.rs:70`, `roles.lead.max_peers` |

Verified: high c99261a. Every quaqua claim was re-read at the cited line or is reproduced by the script, following principle-prove-it-works. Not run: herdr `events.subscribe` over the socket, which is a schema read only, and any piggery code.

## Summary

- **Changes:** this design doc only. No code and no config changed.
- **Tests:** none (design task). The evidence check is `bash verify-layout.sh ~/Lab/.wt/truonglao-layout`, rc 0. Test lines added/removed: 0/0.
- **Risks:**
  - the v3 schema breaks older binaries (P1 mitigates);
  - fail-closed identity can lock users out (P3 shadow, D3);
  - the opener extraction touches E1-owned `journal_sqlite.rs`;
  - D4 overrides the research report's jsonl recommendation.
- **Cost:** spend was not readable from inside this claude session. The CoS reads it at acceptance.

## Blockers

none

## Lessons

- Before placing a "gate at the verb", grep every caller of the shared spawn function: quaqua has 3 verbs but 7 `spawn_worker` call sites (pipeline, engine activity).
- Before reusing an event-sourcing table for another purpose, check its readers: `quaqua status` groups every `workflow_id`, so new streams would leak in as fake workflows.
