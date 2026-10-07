# Quaqua 🍑 — SLP orchestration on Herdr

A personal multi-agent setup: one Claude Code **Lead** (Quả Quả) that turns the owner's vision
into briefs, **Peer** workers running in [Herdr](https://herdr.dev) panes, and a read-only
**Supervisor** on a 2-day launchd timer. The model follows SLP (Supervisor – Lead – Peer).

This repo is a snapshot of the live workspace at `~/quaqua`. Refresh it with
`scripts/sync.sh`. Personal state (`USER.md`, `MEMORY.md`, `memory/`, `projects/`) is
intentionally excluded.

## Architecture

```
            Human (owner) — vision, priorities, irreversible / external actions
              ▲  │                                   ▲
   escalate   │  │ vision                            │ beyond-authority report
              │  ▼                                   │
 Lead (Quả Quả, Claude Code)  ◄──── drift report ─── Supervisor (launchd, every 2d)
 framing → brief → route → accept                    codex exec, read-only sandbox
              │  ▲                                   → /tmp/sup-<date>.md
   Assignment │  │ Handback (6 fields)
   via Herdr  ▼  │ COMPLETE · REOPEN_REQUEST · DEPENDENCY_REQUEST · BLOCKED
 Peers — one pane each in tab <project>-workers, roster in models.md
   Engineer (write, own worktree, commits SHA) · Scout · Architect · Reviewer (read-only)
```

Invariants: one scope has one writer · every redirect flows through the Lead · the writer never
accepts its own work · lifecycle status (idle, exit 0, tests pass) is not acceptance · an
independent Reviewer is spawned only when one of the conditions in `LEAD.md` holds.

## Files

| File | Role |
|---|---|
| `AGENTS.md` | Lead entrypoint: persona override, hard rules, memory policy |
| `IDENTITY.md`, `SOUL.md` | Lead persona and voice |
| `LEAD.md` | SLP judgment layer: assignment format, handbacks, review, acceptance |
| `PEER.md` | Worker contract: boundaries, contract-before-test, git commit gates, handback |
| `herdr.md` | Herdr mechanics: spawn, layout, lifecycle, monitor, fan-out |
| `models.md` | Worker roster: nickname → kind / model / effort |
| `.claude/settings.json` | Denies the built-in `Agent` tool so all workers go through Herdr |
| `.claude/skills/herdr-ops/` | Herdr command recipes |
| `supervisor/supervisor.sh` | Collects agent state + docs, pre-checks dead refs, runs codex review |
| `supervisor/com.quaqua.supervisor.plist` | launchd job (every 172800s) |
| `codex-config.toml` | Codex config used by codex workers |
| `SETUP-PROMPT.md` | Original prompt used to bootstrap this setup |
| `slp.tldr` | tldraw diagram: SLP vs Quaqua |

## Install

```bash
cp -R . ~/quaqua                     # then add your own USER.md / MEMORY.md
cp supervisor/com.quaqua.supervisor.plist ~/Library/LaunchAgents/
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.quaqua.supervisor.plist
```

Run the Lead with Claude Code from `~/quaqua` inside Herdr (`HERDR_ENV=1`).
