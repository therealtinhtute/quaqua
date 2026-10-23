# piggery vs quaqua — layout manifest, gate, mail, events

Related: [`plan.md`](plan.md) · [`design.md`](design.md) (§A–§D, D1–D9) · [`README.md`](README.md)

> **Correction (CoS, 2026-10-23).** `max_workers_per_orchestrator` **is** enforced:
> `crates/quaqua/src/worker/spawn.rs:363-370` (cap comparison at `:365`) and
> `crates/quaqua/src/taskforce/mod.rs:112-121` (the taskforce reserves its weight, `:120`).
> The "not enforced" rows in §1 (Concurrency) and in §2 below stand as written for the rest —
> `can_spawn`, `can_prompt` and `limits.depth` are still declared but never read at runtime.

Sources read: `~/Lab/.research/piggery` at `14859909` (v0.10.4, MIT, `LICENSE:1`, Go + SQLite) and
`~/Lab/quaqua` at `c99261a` (main). No piggery binary or script was run; every piggery claim is a
read of source or its own docs.

## Playbook

1: done — located entry points and intent: piggery `internal/core/{engine,mail,agent,gate,wake,watch,deliver,timers}.go`, `manifests/`, `docs/reference.md`; quaqua `config.rs`, `worker/spawn.rs`, `worker/prompt/*`, `taskforce/mod.rs`, `gate/*`.
2: done — output structured by the brief's five questions plus Gotchas and a migration path.
3: done — `Verified: high c99261a`; see the closing block.

## Overview

piggery is a **daemon with a database as the team's state**: a team is founded from a YAML manifest
that is then frozen into `teams.manifest` (`internal/store/schema.sql:3-8`), and every agent verb
(`send`, `inbox`, `who`, `agent spawn|resume|kill`, `watch`, `join`) is an RPC over a unix socket
(`internal/server/transport_unix.go:11-14`) into one `Engine` that owns all rows. Nothing an agent
does can bypass that engine, so "the gate" is just the first code in each verb.

quaqua has **no daemon and no store**: a worker is a herdr pane running a harness, its state is
files in `runs/` (`<name>.md`, `.owner`, `.budget`, `.<name>.playbook`) plus pane metadata, and the
only cross-agent channel is `herdr agent prompt` / `send-keys`. `team.toml` is a *strategy* file
(kinds, purposes, timers, caps), not a topology: the only topology is hardcoded SLP
(`config.rs:17`, `worker/spawn.rs:203-220`).

Brief unknowns, resolved: the gate is **in-process** (no hook — `mail.go:147`, `agent.go:49`);
piggery's two hook surfaces keep no rule — the harness event relay (`internal/cli/hook.go:18-25`,
"it keeps no state and no rule") and the notify hooks (`internal/server/hook.go:16-70`); waking is
**daemon-driven** (`engine.go:93-124` → `wake.go:23-72` resume, `deliver.go:26` batch push,
`d.Deliver`/`d.Wake` per harness driver, else `hooks/notify.d`); licence **MIT**.

## 1. Layout manifest schema

piggery `manifests/slp.yaml:9-60` is the whole shape; structs at `internal/core/engine.go:236-274`.

| Concern | piggery | quaqua |
| --- | --- | --- |
| Role names | arbitrary (`supervisor`, `lead`, `peer`, `oracle`, `polecat` — `manifests/slp.yaml:12-48`) | exactly three: `ROLE_NAMES = ["cos","lead","peer"]` (`config.rs:17`), anything else fails validation (`config.rs:492`) |
| Role class vocabulary | none needed — roles are the vocabulary | fixed classes `cos|lead|peer|taskforce` (`config.rs:15`), checked at `config.rs:495` |
| Who may spawn whom | `roles.<r>.can_spawn: [role]` (`engine.go:256`) **and** `routing[]` must allow it (`agent.go:98,113-118`) | `Role.can_spawn: Vec<String>` (`config.rs:158-160`) — declared, validated, **never read at runtime** (no reference outside `config.rs`) |
| Who may mail whom | `routing[] {from,to,allow,cc}`, first match wins, **no match = deny** (`engine.go:269-274`, `:431-438`) | `can_prompt` (`config.rs:162`) — same fate: no runtime reader; Lead→CoS is a markdown file convention, not code |
| Escalation | `timers[].notify: reports_to` (`manifests/slp.yaml:55-56`) | `Role.reports_to` (`config.rs:164`), validated against `ROLE_NAMES ∪ {owner}` (`config.rs:500-503`), used only as prose |
| Depth | `limits.depth` enforced at spawn, `depth+1 > limit` → deny `limits.depth` (`agent.go:107`) | `limits.depth` default 2 (`config.rs:97`), validated as a parse only — never counted |
| Concurrency | `limits.concurrency` enforced per team, live headless count (`agent.go:110`, `:677`) | `limits.max_workers_per_orchestrator` (`config.rs:82`), range-validated (`config.rs:450-462`), **not enforced** at `worker spawn` |
| Flood guard | `limits.messages_per_participant_per_minute` — over it the message is *stored but held*, one notice (`limits.go:20-32`, `:36-60`) | none (no mail) |
| Respawn guard | `limits.max_respawn_per_hour` — parked + lead told (`agent.go:235`) | none |
| Harness per role | `roles.<r>.spawn.{harness,model,thinking,allow_tools}`, `inherit` walks role → `harness/<h>.json` → founding session → harness default (`engine.go:259-266`, `manifests/slp.yaml:20-24`) | `[kinds.<k>] harness|model|effort|fallback` (`config.rs:196-206`); `[roles]` has **no** kind field — role→kind only via `purposes`/`strategy` (`config.rs:169-190`, `:269`) |
| Tools per role | `roles.<r>.tools: [send,inbox,who,agent]`, checked per verb (`roles.go:115`, `mail.go:149`) | none — harness-side; quaqua cannot enumerate a worker's tools |
| Taskforce | top-level `taskforce:` block makes a template callable as a team (`engine.go:245-248`) | two hardcoded modes, `advisor|dual-lens` (`taskforce/mod.rs:26-31`), members from `[taskforce.<mode>]` (`taskforce/mod.rs:159-164,229-235`) |
| Per-project override | a team freezes its manifest at founding; nothing is re-resolved | `[projects.<slug>]` merge (`config.rs:426-440`) re-applied on **every** spawn |

**Versioning and inheritance.** There is no `extends`/`from` inheritance: a manifest must be
self-contained, and `Inline` (`manifests.go:441-492`) rewrites `instructions_file` into `instructions`
before core ever sees it. "Inheritance" exists only in two weaker forms:

1. *Template home* — `<home>/templates/<name>/manifest.yaml` plus its prompt files; built-ins are
   `go:embed`-ed (`manifests.go:24-25`) and unpacked with a per-file sha256 record
   `.builtin.json` (`manifests.go:38,159-218`): an unmodified built-in file is updated on upgrade, a
   user-edited or user-deleted one is left alone, a dropped built-in is cleaned out only where hashes
   still match. That is the whole "template versioning" story.
2. *Copy-on-create* — `template new <name> --from <built-in>` copies the built-in and renames the
   top-level `template:` key with a regex, touching no other byte (`manifests.go:328-363,366-377`).

**Frozen at founding**: `teams.manifest TEXT` (`schema.sql:5`) is the snapshot, so a template edit
applies to the next team only (`docs/reference.md:89`). A legacy key rename — top-level `model:` →
`template:` — is handled by `MigrateKey`, which rewrites one line per manifest and *moves the recorded
hash* so an unedited built-in keeps updating (`manifests.go:380-431`). That is the exact
backward-compatibility pattern quaqua needs for `team.toml`.

## 2. The gate

**Where it intercepts.** One function per verb, all before any write, all inside the daemon
transaction: `sendGate` (`mail.go:147-235`, called by `Send` `:16` and by `why`), the spawn gate
(`agent.go:49-140`), `Admit` (`admit.go:45-47`), `WatchAdd` (`timers.go:26-55`, which re-checks
routing because timer mail comes from the engine), and `gateConcurrency` (`agent.go:677`). Because
every agent action is an RPC (`internal/server/peer.go`, `transport_unix.go:11`), there is no
side door: a harness that does not speak the protocol cannot act.

**What it rejects**, in order, with layer:

| Check | Layer | Code |
| --- | --- | --- |
| token / run identity | `token` (`engine.go:143-146`) | `CodeUnauthorized` |
| role has the tool (`send`, `agent`) | `permission` (`roles.go:115`, `mail.go:149`) | `CodeDenied` |
| target exists and is in view (gate_closed, leaver, cross-team) | `visibility` (`gate.go:73-83`, `mail.go:185-199`) | `CodeDenied` |
| `routing` rule for (sender role, receiver role) | `routing` (`mail.go:197-201`, `agent.go:113-118`) | `CodeDenied` |
| `can_spawn` membership | `permission` (`agent.go:98`) | `CodeDenied` |
| `limits.depth` / `limits.concurrency` | `limit` (`agent.go:107`, `:110`) | `CodeDenied` |
| mail rate over the limit | not rejected — stored `held_reason=policy_hold` (`limits.go:36-44`) | event `held` |
| `can_set_cwd`, spawn bounds | `permission` / `bounds` (`agent.go:130-136`) | `CodeDenied` |
| `to: notify` | `routing` (`mail.go:165-166`) | `CodeDenied` |

**Error shape** (`api.go:413-419`):

```json
{"code":"denied","message":"routing does not allow peer -> peer","rule_id":"routing.no_rule","layer":"routing","details":{"to":"x"}}
```

`deny` also writes a `denied` event carrying `{verb, rule_id, layer, reason}` (`engine.go:141-165`),
so a refusal is auditable and the `top` UI can show it. `piggery why <from> <to>` replays the *same*
`sendGate` with a `gateTrace` and sends nothing (`mail.go:127-144`) — the gate is dry-runnable by
construction, not by a parallel reimplementation.

**quaqua's `gate` is a different animal**: a PreToolUse(Bash) hook (`gate/mod.rs:1-7`) that classifies
shell text — herdr `pane split`/`tab create`, `git push`, `gh pr merge` — as Allow/Ask/Deny
(`gate/policy.rs:1-11`). It polices *outward actions*, not spawn or messaging; it has no notion of a
role, a depth, or a recipient, and its fail-closed path blocks a guarded set while allowing the rest
(`gate/mod.rs`). Nothing in quaqua enforces `can_spawn`, `can_prompt`, `limits.depth` or
`max_workers_per_orchestrator` today; only the kind-level `spawn = "approve"` policy is enforced, and
only as a launch-time `--approve-kind` check (`config.rs:640-654`, called from `lead/spawn.rs:264`).

## 3. Inter-agent communication

**Store** (`schema.sql:33-66`):

| Table | Columns that matter |
| --- | --- |
| `messages` | `id`, `seq` (global, `UNIQUE`), `client_msg_id` (idempotency key, unique per `from_id` `:51`), `team_id`, `from_id`, `to_id` (a participant, `board`, `notify`, `engine`), `kind`, `thread_id`, `reply_to`, `expects_reply`, `op`/`target` (`assign`, board `replace`/`remove`), `body`, `created_at`, `acked_at`, `held_reason` |
| `deliveries` | one row per (message, run, batch) with `delivered_at`, `acked_at` — a message can be delivered more than once and acked once |
| `batches` | `(run_id, batch_seq)` with `completed_at`; "batch closed" is this table, events are audit only (`:68-76`) |

**Delivery** has two paths. Push: the harness adapter calls `HarnessTurnStart`, core opens a turn
batch, sets `working`, and hands the mail into the prompt (`harness.go:54-72`) via `giveMail`/
`giveMailWhere`, which renders with `RenderMail`, inserts `deliveries`, and is capped by
`maxMailText` (`harness.go:344-380`). For drivers declaring `CapDeliver`, core pushes batches itself
with a per-worker mutex so the model never reads batch n+1 before n (`deliver.go:10-60`). Pull:
`inbox` returns unacked messages and records a delivery with `batch_seq NULL` — those can never be
acked (`mail.go:271-300`).

**Ack** is batch completion, not read: `mail.go:434-450` sets `deliveries.acked_at`, then
`messages.acked_at` only if still addressed to that participant and still unacked. A cancelled batch
is not acked, so its mail is handed out again (`deliver.go:10-12`), and a re-delivery is marked as such
in the rendered text (`harness.go:352`).

**Waking** (`engine.go:93-124`): `notifyAfterCommit(id)` refuses to wake while the member is
`awaiting_permission` or has an open batch, or when it has no unacked unheld mail; then: gone member of
an open team → `autoWake` resumes its session as a headless worker (`wake.go:23-72`, `:78-87`), guarded
by `wake_seq` so the *same* unacked mail cannot wake it twice; headless → driver `Deliver`, else
`Wake`; otherwise the notify sink runs `hooks/notify.d/*` with one JSON line on stdin
(`notify.go:9-22`, `server/hook.go:44-70`). A team's gate is never woken — its mail is a queue.

**quaqua**: no mailbox. Lead→CoS is a *document*: append to `runs/<slug>-lead.inbox.md`, first line
`## <ISO> <lead> <kind>` (`templates/workspace/rules/_all/lead-inbox.md:3`, restated in
`templates/workspace/AGENTS.md:34`). `grep -rn inbox crates/` returns **nothing** — no reader, no
writer, no ordering, no ack. Worker→Lead is `runs/<name>.md` plus `herdr agent prompt`; the report file
is the completion signal precisely because `agent_status=done` is unreliable (repo rules). So: message
= file, ack = task acceptance, wake = the Lead/CoS being awake at all.

## 4. Monitoring and events

**Event log** (`schema.sql:83-90`): `events(seq, ts, type, participant, team_id, run_id, ref_id,
payload JSON)` with `(team_id,seq)` and `(participant,type,seq)` indexes. Types written by core:
`spawned`, `spawn_failed`, `resumed`, `respawn_limit`, `stopped`, `exited`, `aborted`,
`delivery_cancelled`, `denied`, `held`, `released`, `notice`, `watch_fired`, `team_up`, `team_down`,
`left`, `admitted`, `rerouted`, `reconcile`, `gc`, `gate_closed`/`gate_opened`, `gate_moved`,
`model_set` (grep of `evt{typ:` across `internal/core`). Events are deliberately *decisions and
lifecycle* only; message and turn history is read from the state tables (`team.go:259-261`).

**Push vs poll**: there is **no subscribe API**. `log [--after SEQ] [--team] [--limit]` is a plain
cursor query (`team.go:263-286`); the TUI `top` re-reads `ps --view` once per second
(`internal/cli/top.go:26`); the daemon's own 1s ticker runs `FireDue` and `Watch`
(`internal/server/server.go:306-320`). Everything else the operator sees is derived from the same
rows.

**Status and stall**: participant `state`/`state_since`/`last_turn_end` (`schema.sql:10-28`) are set
by the adapter at turn start/boundary. `timers[]` rules `{on, notify, silent_for}` (`watch.go:16-35`,
validated at `:47-77`) fire when `state == working` **and** `now - max(state_since, last_turn_end) >
silent_for` (`watch.go:189-207`); the incident key is `team/rule/participant/state_since`, deduped
against prior `watch_fired` events so one incident notifies once (`watch.go:156-166`). Content is
never judged ("Nothing is judged by content", `watch.go:12-14`). `taskforce.idle_for` is the same idea
for a whole team. Separately, `timers` rows schedule *model-visible* messages ("watch.add",
`timers.go:16-60`) with a one-minute floor (`:9`).

**quaqua**: no event log. Monitoring is `quaqua worker prompt` in the foreground of the caller, polling
`runs/<name>.md` every 60s (`PROMPT_WORKER_POLL_SEC`, `worker/prompt/mod.rs:502-505`), with stall =
600s without worktree diff or harness session write (`:506-511`, `:1040-1058`), idle 120s, gone 60s,
and an exit-code contract 0-6 (`:1045-1090`). Progress deliberately no longer reads pane text
("progress comes from the worktree diff and the harness session store",
`worker/prompt/observer.rs:1-5`); liveness is a pid (`observer.rs:38`) and a `pipeline-*.state` scan
gated on the `-lead` suffix (`observer.rs:11-13`).

## 5. What quaqua should borrow, ranked

| # | Borrow | Why it pays | Lands in |
| --- | --- | --- | --- |
| 1 | `routing[]` as **data, first match wins, no match = deny** (`engine.go:269-274`, `:431-438`) | the only mechanism that makes `can_spawn`/`can_prompt` mean anything; today they are dead config (`config.rs:158-162`) and the SLP shape is hardcoded | new `[layout]` table in `team.toml`, read by `worker/spawn.rs`, `lead/spawn.rs`, `taskforce/mod.rs` |
| 2 | One choke point + trace + `denied` record (`mail.go:147-235`, `engine.go:141-165`) | quaqua *does* have three chokes (`quaqua worker spawn`, `quaqua lead spawn`, `quaqua taskforce`), so a single `layout::check` called by all three is enough; the trace is what makes a denial debuggable and makes a `quaqua why` possible | `config.rs` next to `check_spawn_policy` (`:640`) |
| 3 | Manifest **frozen per team** + version key (`schema.sql:5`, `docs/reference.md:89`) | quaqua re-resolves `team.toml` on every spawn (repo rules §"Cấu hình động"), so editing a policy retroactively changes running projects — the opposite failure and the more dangerous one | `runs/<slug>.layout.toml` snapshot written at `quaqua lead spawn` (same pattern as `runs/<name>.owner`) |
| 4 | Mail with `id`/`seq`/`acked_at`/redelivery (`schema.sql:33-66`, `deliver.go:10-12`) | `runs/<slug>-lead.inbox.md` has **no reader in code** and no ack; a lost line is a lost report | append-only `runs/<slug>.mail.jsonl` with a `seq` cursor + `acked` marker, not SQLite |
| 5 | Watch rules as data with `reports_to` and one notice per incident (`watch.go:16-35`, `:189-207`) | quaqua hardcodes 600/120/60s; piggery's dedupe key (one notice per incident) is the part worth copying — repeated nudges are what makes a watchdog noisy | `[timers]` in `team.toml` already exists (`config.rs:125-143`); add per-role `silent_for` |
| 6 | Dry-run `why` (`mail.go:127-144`) | cheap, and the only way to answer "why was this spawn refused" without reading code | `quaqua layout why <from> <to>` |

**Skip, with reasons**

| Skip | Reason |
| --- | --- |
| SQLite store + daemon + unix socket | quaqua's state must stay greppable files in `runs/`; a daemon adds a lifecycle quaqua deliberately does not own (herdr is the process owner) |
| `gate_closed` visibility, cross-team mail, `gc`, `top` TUI, `gateAction` (`gate.go:94-162`) | all of it serves multi-team tenancy piggery has and quaqua does not |
| notify hooks (`hooks/notify.d`) | herdr already has agent state and the CoS is a pane; a second notification path doubles the failure surface |
| template **unpack** machinery (`.builtin.json`, 3-way hash sync, `manifests.go:159-218`) | quaqua already has a managed/seed template tree with modes (`init.rs:460`); a second sync engine is needless |
| `instructions_file` inlining + per-role prompts in the manifest | quaqua's prompts are `templates/workspace/AGENTS.md` + `rules/`; embedding them per role duplicates the one home per fact |
| `roles.<r>.tools` allowlists | quaqua cannot enumerate a harness's tools; a declared list would be a lie |
| rate-limit **hold** (rather than reject) | needs an admin `release` verb and a hold UI; without them it is a silent backlog |
| `taskforce.idle_for` | quaqua already has the watchdog idle/stall contract; two idle notions will disagree |

### Migration path: `team.toml` → layout manifest without breaking old `team.toml`

Additive, three steps, each independently shippable:

1. **`schema_version = 3` with a `[layout]` table**, structure copied from piggery but typed like
   quaqua's config (`#[serde(deny_unknown_fields)]`, same style as `config.rs:29-46`):

   ```toml
   schema_version = 3
   [layout.roles.cos]
   kind      = "tieude"          # reuse [kinds.<k>] — do not invent a second model table
   can_spawn = ["lead", "peer"]
   [layout.routing]
   cos  = ["lead", "peer"]       # edges out
   lead = ["peer"]
   [layout.limits]
   depth = 2
   ```

   `ROLE_NAMES` (`config.rs:17`) stops being a const and becomes the *legacy default set*: when
   `[layout]` is absent, `resolve()` synthesizes exactly today's graph from `[roles.cos|lead|peer]`
   (`templates/workspace/team.toml` `can_spawn`/`can_prompt`) and validation stays byte-identical.

2. **Enforce at the three verbs** (the only fork in the road), reading the resolved layout:
   `worker/spawn.rs` (owner role from `runs/<name>.owner`, or `-lead` suffix →
   `spawn.rs:203-220`), `lead/spawn.rs` (`<slug>-lead` at `:104`), `taskforce/mod.rs` (`:99-142`).
   Depth comes from an owner chain — record `depth=` in `runs/<name>.owner`'s sibling
   `runs/<name>.grid`-style file rather than querying anything. Deny shape: reuse the existing
   `(i32, String)` error convention so `quaqua`'s exit codes stay stable (`config.rs:648-653`).

3. **Freeze + migrate**: `quaqua lead spawn` writes `runs/<slug>.layout.toml` (a copy of the resolved
   layout, like piggery's `teams.manifest`); `quaqua config get layout...` reads the snapshot for a
   running project, the live file for a new one. `quaqua config migrate` rewrites v2 → v3 exactly as
   `MigrateKey` does one line at a time (`manifests.go:380-431`) and prints what it changed; a
   `schema_version` the binary does not know is refused with the migrate hint, never silently
   downgraded.

The one thing quaqua cannot borrow is the assumption that the gate sees everything: a Lead can still
`herdr agent prompt` a Peer by hand, bypassing `quaqua`. State that in the rules — "enforcement covers
what quaqua mediates" — instead of pretending the shape is closed.

## Gotchas

- **Naming is load-bearing.** `<slug>-lead` is the only runtime role signal (`worker/spawn.rs:211,220`,
  `lead/spawn.rs:104`, `worker/prompt/observer.rs:12`) and also drives the grid tab
  (`spawn.rs:203-220`). Any layout work must keep it, or panes land in the wrong tab.
- **`none` means absent.** `limitMap.UnmarshalYAML` drops `key: none` so a limit is never enforced
  (`engine.go:402-430`); `roles that can spawn need depth and concurrency` is only documented
  (`docs/reference.md`, manifest table), not enforced for a user-written template.
- **Holding is not failing.** Mail over the rate limit is stored `held` and never delivered until an
  admin releases it (`limits.go:36-60`) — an agent sees silence, not an error.
- **Frozen manifests surprise operators in the other direction**: editing a template does nothing for
  a team already founded (`docs/reference.md:89`). quaqua has the inverse surprise today; pick one and
  document it.
- **quaqua's gate cannot police spawn/mail** because it classifies *shell text* (`gate/policy.rs:1-11`);
  do not extend it — a manifest check belongs in the verb, before any spawn.
- **Two idle notions collide.** piggery watches `state=working && no turn end`; quaqua watches
  worktree diff + session write. Porting `timers[]` without quaqua's diff-based progress signal would
  re-introduce the pane-text stalls the current watchdog deliberately dropped (`observer.rs:1-5`).

## Verdict

Verdict: borrow routing-as-data ("no rule = deny"), a per-project frozen layout, and a mail record
with `seq`/`acked_at`; skip the daemon, the SQLite store, the notify hooks and the tenancy layer.
piggery's architecture is right for piggery and wrong for quaqua: it buys a closed gate by owning a
daemon, a database and every harness's transport, which quaqua has explicitly traded away for herdr
panes and greppable files. But three of its ideas are cheap and fix real quaqua defects — (1)
routing-as-data with *no rule = deny*, which is the missing enforcement behind `can_spawn`,
`can_prompt` and `limits.depth` (`config.rs:158-162`, `:97`); (2) freezing a resolved layout per
project instead of re-resolving `team.toml` at every spawn; (3) a message record with `seq`/`acked_at`,
because `runs/<slug>-lead.inbox.md` has no reader in code at all. Do those three behind a
`schema_version = 3` `[layout]` table that synthesizes today's edges when absent, enforce at the three
verbs quaqua already owns, and skip the store, the hooks and the whole tenancy layer. Note the ceiling
honestly: quaqua can only gate what it mediates.

Verified: high c99261a — every quaqua citation re-read at the cited line; `grep -rn 'can_spawn|can_prompt|limits.depth' crates/quaqua/src --include='*.rs'` outside `config.rs` returns nothing (the "never enforced" claim); `grep -rn inbox crates/` returns nothing (the "no reader" claim). piggery citations are reads of the cloned tree at `14859909`; nothing in piggery was executed, so behaviour claims beyond the source (e.g. driver wake paths at runtime) are marked as source-level only.

## Lessons

- A policy key that is validated but never read is worse than a missing one: it reads as a guarantee. Check `grep` for the key outside its schema file before trusting any `can_*` config.
- Freezing a resolved config per project and re-resolving it per spawn are opposite failure modes; name which one a system has before porting a config format from the other.
