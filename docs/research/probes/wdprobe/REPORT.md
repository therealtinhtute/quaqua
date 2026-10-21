# herdr 0.9.5 watchdog signals — can we drop pane-text scraping (issue #87) and 30 s polls (issue #47 P1)?

Probe run 2026-10-21 on herdr **0.9.5**, branch `quaqua/tieude-wdprobe`.
Throwaway workspace `w9` (label `wdprobe`), three panes, all created and closed by this
probe. No product code changed. Scripts: `probe.py` (1 s sampler + event subscriber),
`question-probe.py` (question UI sequence), `scenario.sh` (prompt driver), raw logs in `logs/`.

Method (exact commands):

```
herdr workspace create --label wdprobe --cwd /tmp/opencode/wdprobe-cwd --no-focus   # w9
herdr tab create --workspace w9 --label oc|claude|omp ...
herdr agent start wd-oc2 --kind opencode --pane w9:p7 -- --standalone
herdr agent start wd-cc   --kind claude   --pane w9:p5 -- --model haiku
herdr agent start wd-omp  --kind omp      --pane w9:p6
python3 probe.py <out> 75 w9:p7=wd-oc2=opencode w9:p5=wd-cc=claude w9:p6=wd-omp=omp
# monitor: events.subscribe {pane.agent_status_changed,pane.exited}; every 1 s agent get +
# pane read + per-session store mtime (opencode: sqlite session_message.time_updated,
# claude: ~/.claude/projects/*/<id>.jsonl, omp: agent_session path)
herdr agent prompt <name> 'Run exactly this bash command ... sleep 40 ...'
herdr workspace close w9
```

Run 1 window 14:48:51–14:50:09 (75 s, 74 samples); question probe 14:51; a second
claude window 14:52. `herdr` was called as the CLI binary; no socket writes besides
`events.subscribe`.

## 1) Does `agent_status=blocked` appear while a question/permission UI is on screen?

| harness | UI actually triggered | observed status sequence | verdict |
|---|---|---|---|
| claude | Yes: `AskUserQuestion` UI (`Option A / Option B / Type something / Enter to select · ↑/↓ to navigate · Esc to cancel`) | `working → blocked` at t+53.9 s, held **blocked for the full 28 s window** while the prompt was rejected with `agent_blocked`; Esc → `done` | **yes** |
| opencode | Yes: question tool rendered (`# Questions / Pick A or B? / You picked B.`) | live sequence **not captured**: the prompt was queued behind the 40 s sleep turn, so `working` never dropped before the window ended. Plugin contract maps `question.asked|permission.asked → blocked` (evtprobe 2026-10-17, plugin v13) | **partial** |
| omp | No question UI: the ask prompt ran 48 s and ended `working → done`; pane logged an HTTP 400 (`~/.omp/logs/http-400-requests/…`) | never `blocked` in 74 samples | **no evidence** — do not rely on it |

Also observed: `blocked` is not question-specific. A fresh claude trust prompt during
`agent start` was `blocked` too, and `herdr agent prompt` refused it with
`{"code":"agent_blocked"}`. `blocked` therefore means “waiting for interactive input”
(question **or** permission/trust), which is exactly the watchdog's exit-5 bucket.

## 2) `agent_session.value` → session store, and last-write behaviour

| harness | `agent_session` | store used | readable | last-write advances while working? | stale while blocked? |
|---|---|---|---|---|---|
| opencode | `kind=id`, `ses_ea290ccb…` | `~/.local/share/opencode/opencode.db`, `max(session_message.time_updated) where session_id=…` | yes | yes: +1.0 s, +3.0 s, +3.8 s, +13.2 s, +6.3 s … | n/a |
| claude | `kind=id`, `95f94f1c-…` | `~/.claude/projects/<slug>/95f94f1c-….jsonl` mtime | yes | yes (sub-second while working) | yes: 0 ms change over 28 s blocked |
| omp | `kind=path`, `…/sessions/-tmp-opencode-wdprobe-cwd/2026-10-21T07-47-17-063Z_….jsonl` | the path itself (mtime) | yes | only at turn boundaries in this sample (short turns) | n/a |

No harness lacked a session id or a readable store. The important negative: during
opencode's `sleep 40` tool call the per-session store was **still for 40.0 s** while the
status stayed `working` (store_ms 1792568958542 → 1792568998578, exactly the silent shell
call). A store mtime is therefore a real progress signal for model/tool traffic, but it is
**blind to a silent tool call** — it cannot be the only stall detector.
(Claude's Bash prompt in this probe returned in ~6 s without actually sleeping, so the
40 s figure is opencode's; treat the blind spot as ≥40 s, not 40 s exactly.)

## 3) Does `pane_output_changed` / revision work as a progress signal?

- `events.subscribe` accepts only `pane.output_matched`, `pane.agent_status_changed`,
  `pane.scroll_changed` (`herdr api schema --json`, Subscription oneOf). There is **no
  `pane.output_changed` subscription**; it exists as an `EventMatch` for `events.wait`
  with `{event:"pane_output_changed", pane_id, min_revision}`.
- Raw pane text changed every second **while status was `done`** (sleep2: hash
  `71b2d33…` at 0 s, `a055b0f…` at 6 s, `96e1dde…` at 7 s — TUI clock/status bar), so output
  revisions bump on idle chrome, not only on agent work.
- The `agent` object's `revision` stayed `1` for opencode across the whole 70 s working
  window; it is not an output revision.

Verdict **partial**: `pane_output_changed` (with `min_revision`) is a good *wakeup*
(“something moved, read again”), but it is not a progress/stall signal — an idle TUI
keeps producing it. It cannot replace the filtered pane body or worktree diff for “is the
worker advancing?”.

## 4) Built-ins in herdr 0.9.5 we may have missed

| primitive | shape | verdict |
|---|---|---|
| `events.wait` | `EventMatch` for `pane.agent_status_changed`, `pane.exited`, `pane_output_changed` + `min_revision`, timeout_ms | **usable wait primitive**; must pin panes and re-sync on topology change (evtprobe contract) |
| `agent.wait <name> --until <status> --timeout` | status-only; `herdr agent wait wd-cc --until blocked --timeout 1500` → `{"error":{"code":"timeout"…}}` after 1.5 s | works, no progress dimension |
| `pane.wait_for_output` | substring/regex over `visible|recent|recent-unwrapped`, timeout | still glyph/text matching (issue #87 not solved by it) |
| `agent.explain` | detection-state explanation | no progress/stall timestamp |

No primitive exposes “last real activity” or “stalled”; the only real-activity timestamps
available are the harness session stores and the worktree.

## Recommendation

1. **Wait (issue #47 P1): yes, drop the 30 s poll.** Use `events.wait` /
   `events.subscribe` on `pane.agent_status_changed` + `pane.exited` for pinned panes,
   with the evtprobe connect-time re-sync contract (read state at connect, reconnect when
   panes are added). Keep the run-file guard; never act on `idle` alone.
2. **Progress (issue #87): use the session store per harness, but keep a second signal.**
   - opencode: `max(session_message.time_updated)` for the session id (per-session, not
     the shared `opencode.db-wal` mtime);
   - claude: mtime of `~/.claude/projects/<slug>/<session-id>.jsonl`;
   - omp: mtime of `agent_session.value` (`kind=path`).
   All three are glyph-free and track model/tool traffic, but a silent tool call leaves
   them flat for ≥40 s (measured), so keep the filtered pane body / worktree diff as the
   liveness fallback. Do **not** use raw output revision as progress.
3. **Questions: use `blocked` where proven, keep the text matcher as fallback.**
   claude verified (question UI and trust/permission UI both `blocked`); opencode is
   plugin-contract (partial live observation); omp showed none (ask prompt failed with an
   HTTP 400). Until omp is proven, keep `question_seen()` for omp panes, or classify
   `blocked` + pane text. `herdr agent prompt` rejecting with `agent_blocked` is a cheap
   pre-check.

Residual risks: single 75 s window; opencode question transition not caught live; omp
question path unproven (HTTP 400); no long silent call measured on claude; idle TUI
revision measured only on claude.
