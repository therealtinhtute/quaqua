# Quaqua 🍑 — SLP orchestration on Herdr

A personal multi-agent setup following SLP (Supervisor – Lead – Peer) on [Herdr](https://herdr.dev).
The owner is the **Principal**, outside the system, and talks only to Quả Quả (Claude Code),
the **CoS** that fills SLP's Supervisor slot and owns the full SDLC; each project gets an
internal **Lead**; **Peers** are task workers in Herdr panes.
A read-only **Auditor** runs at 07:00 and 19:00 local time via systemd (Linux) or launchd
(macOS). Diagram: `docs/slp.png`.

This repo is the live workspace: clone it to `~/quaqua`. Per-workspace personal state
(`USER.md`, `MEMORY.md`, `memory/`, `projects/`, `raw/`, `runs/`, `reports/`) is gitignored and stays local; start
`USER.md` from `USER.example.md`.

## Architecture

![SLP architecture](docs/slp.png)

```
 Principal (owner) — goal, priorities, trade-offs; approves push / merge to main
        │ goal                ▲ digest
        ▼                     │
 CoS (Quả Quả) — full SDLC: intake, sizing, brief, verify, release  ◄── report ── Auditor
        │ small task (<15 min, 1 file, or needs live feedback) ──► one worker, no SLP   (systemd/launchd
        │ spawn Lead + brief                                                           2×/day)
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
- Brief has 5 sections: Goal / Hard constraints / Current design / Unknowns / Evidence; it never pre-solves.
- Peers may challenge premises in a `## Challenge` section; the Lead resolves it before acceptance.
- CoS never acts as Lead on pipeline-size work (no self-review); every redirect flows through CoS
  and is recorded in Lead state.
- Telemetry decides which mechanisms stay: a reviewer that rarely changes outcomes gets turned off.

Decision lock: `spec-raw/quaqua-slp-slim-lock.md`.

## Files

| File | Role |
|---|---|
| `AGENTS.md` | Entrypoint: SLP roles, memory policy, worker protocol |
| `identity.md`, `SOUL.md` | CoS persona and voice |
| `PEER.md` | Worker contract: boundaries, contract-before-test, git commit gates, handback |
| `rules/_all/` | Worker rules injected as the system prompt (brief shape, fanout grid, timebox) |
| `models.md` | Worker roster: nickname → kind / model / effort |
| `.claude/settings.json` | Denies the built-in `Agent` tool so all workers go through Herdr |
| `.claude/skills/herdr-orchestration/` | Worker flow + consolidated command, recovery, and assignment/acceptance references |
| `scripts/auditor.sh` | Auditor: read-only drift + pipeline-run audit → `reports/audit-*.md` |
| `scripts/install-auditor.sh`, `systemd/`, `launchd/` | Auditor scheduling: systemd user units (Linux) or a rendered LaunchAgent plist (macOS), 2×/day |
| `scripts/commit-memory.sh` | CoS notes commit on the current branch: stages only `memory/ projects/ raw/`, refuses foreign staged paths, never pushes or amends |
| `codex-config.toml` | Codex config used by codex workers |
| `USER.example.md` | Template for the per-workspace `USER.md` |
| `SETUP-PROMPT.md` | Original prompt used to bootstrap this setup |
| `docs/slp.png` | Diagram: Principal → CoS → Lead → Peers |

## Install

```bash
git clone git@github.com:therealtinhtute/quaqua.git ~/quaqua && cd ~/quaqua
cp USER.example.md USER.md           # customize per workspace
mkdir -p memory projects runs reports   # gitignored personal state AGENTS.md expects
printf '| slug | repo | owner |\n|---|---|---|\n' > projects/INDEX.md   # one row per project
bash scripts/install-auditor.sh  # installs scheduling files; prints manual activation command
```

Linux: the installer symlinks the user service/timer and calls only
`systemctl --user daemon-reload`. Activate manually with
`systemctl --user enable --now quaqua-auditor.timer`.

macOS: install a current Bash (e.g. `brew install bash`), then put its `bin` directory
first on PATH (`export PATH="$(brew --prefix bash)/bin:$PATH"` for Homebrew).
Run `bash scripts/install-auditor.sh` with `codex` on PATH. The installer requires
Bash >= 4.4 from `command -v bash` and refuses the macOS system Bash 3.2.
It renders `launchd/com.quaqua.auditor.plist.in` into
`~/Library/LaunchAgents/com.quaqua.auditor.plist`, pinning the discovered Bash,
this checkout's absolute `scripts/auditor.sh` path, and the install-time PATH (including
codex's directory). Logs go to `~/Library/Logs/quaqua-auditor.log`; reports still go
in this checkout's `reports/`. It never calls launchctl; run the printed bootstrap
command yourself. Reinstall after moving the checkout or changing toolchain paths.
Calendar intervals run at 07:00/19:00; unlike systemd's `Persistent=true`, launchd does
not replay missed runs while logged out (sleep wakeups may coalesce missed intervals).

Tests require Bash >= 4.4 (macOS system Bash 3.2 is insufficient), git, jq, Perl, and python3
(for plist validation). They use temporary homes and stub herdr/codex/managers; no
network, real agents, or service activation is needed:

```bash
# On macOS, put your installed Bash's bin directory first on PATH as above.
for test in scripts/test_*.sh; do bash "$test" || exit; done
```

CI runs the same suite on `ubuntu-latest` and `macos-latest`; macOS uses a Homebrew Bash.

Run CoS (Quả Quả) from `~/quaqua` inside Herdr (`HERDR_ENV=1`); it spawns project Leads with
`scripts/spawn-lead.sh <slug>`.
