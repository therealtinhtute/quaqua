# Quaqua 🍑 — SLP orchestration on Herdr

A personal multi-agent setup following SLP (Supervisor – Lead – Peer) on [Herdr](https://herdr.dev).
The owner is the **Principal**, outside the system; Quả Quả (Claude Code, CoS) is the
**Supervisor**; each project gets a **Lead**; **Peers** are task workers in Herdr panes.
A read-only **Auditor** runs twice a day on a systemd user timer. Diagram: `slp.tldraw`.

This repo is the live workspace: clone it to `~/quaqua`. Per-workspace personal state
(`USER.md`, `MEMORY.md`, `memory/`, `projects/`, `raw/`) is gitignored and stays local; start
`USER.md` from `USER.example.md`.

## Architecture

![SLP architecture](docs/slp.png)

```
 Principal (owner) — goal, priorities, trade-offs; approves push / merge to main
        │ goal                ▲ digest
        ▼                     │
 Supervisor (CoS Quả Quả) — intake, sizing, brief, cross-project view ◄── report ── Auditor
        │ small task (<15 min, 1 file, or needs live feedback) ──► one worker, no SLP   (systemd
        │ spawn Lead + brief                                                           timer, 2×/day)
        ▼
 Lead (<slug>-lead) — project state, plan approval, acceptance,
        │ task brief                   merges into the project integration branch
        ▼
 Peers — build (sude) ⇄ review (quansu, codex read-only), own worktree each
        │ result                     default 2 review rounds (--rounds overrides); kill after ≥3 stuck
        ▼
 runs/pipeline-<task>.md ──► Lead accepts / merges      runs/telemetry.jsonl (1 line per run)
```

Rules:
- Brief has 5 sections: Goal / Constraints / Current design / Unknowns / Evidence; it never pre-solves.
- Peers may challenge premises in a `## Challenge` section; the Lead resolves it before acceptance.
- CoS never acts as Lead on pipeline-size work (no self-review); every redirect flows through CoS
  and is recorded in Lead state.
- Telemetry decides which mechanisms stay: a reviewer that rarely changes outcomes gets turned off.

Spec and rollout plan: `spec-raw/quaqua-slp-slim-lock.md`, `spec-raw/quaqua-slp-slim-plan.md`.

## Files

| File | Role |
|---|---|
| `AGENTS.md` | Entrypoint: SLP roles, memory policy, worker protocol |
| `identity.md`, `SOUL.md` | CoS persona and voice |
| `LEAD.md`, `herdr.md` | Legacy judgment/mechanics docs — superseded by AGENTS.md + scripts/* |
| `PEER.md` | Worker contract: boundaries, contract-before-test, git commit gates, handback |
| `rules/_all/` | Worker rules injected as the system prompt (brief shape, fanout grid, timebox) |
| `models.md` | Worker roster: nickname → kind / model / effort |
| `.claude/settings.json` | Denies the built-in `Agent` tool so all workers go through Herdr |
| `.claude/skills/herdr-ops/`, `.claude/skills/herdr-orchestration/` | Herdr command recipes + the worker spawn/monitor/cleanup flow |
| `scripts/supervisor.sh` | Auditor: read-only drift + pipeline-run audit → `reports/supervisor-*.md` |
| `scripts/install-supervisor.sh`, `systemd/` | Auditor scheduling: symlinks the systemd user service+timer (2×/day) |
| `codex-config.toml` | Codex config used by codex workers |
| `USER.example.md` | Template for the per-workspace `USER.md` |
| `SETUP-PROMPT.md` | Original prompt used to bootstrap this setup |
| `slp.tldraw` | tldraw diagram: Principal → CoS → Lead → Peers |

## Install

```bash
git clone git@github.com:therealtinhtute/quaqua.git ~/quaqua && cd ~/quaqua
cp USER.example.md USER.md           # customize per workspace
mkdir -p memory projects runs reports   # gitignored personal state AGENTS.md expects
printf '| slug | repo | owner |\n|---|---|---|\n' > projects/INDEX.md   # one row per project
scripts/install-supervisor.sh        # symlinks systemd/quaqua-supervisor.{service,timer}
systemctl --user enable --now quaqua-supervisor.timer
```

Run CoS (Quả Quả) from `~/quaqua` inside Herdr (`HERDR_ENV=1`); it spawns project Leads with
`scripts/spawn-lead.sh <slug>`.
