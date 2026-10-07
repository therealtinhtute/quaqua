#!/usr/bin/env bash
# Snapshot the live ~/quaqua setup into this repo. Personal state (USER.md,
# MEMORY.md, memory/, projects/, supervisor logs) is never copied.
set -euo pipefail
SRC="$HOME/quaqua"
DST="$(cd "$(dirname "$0")/.." && pwd)"
cp "$SRC"/{AGENTS,IDENTITY,SOUL,LEAD,PEER,herdr,models,SETUP-PROMPT}.md "$SRC"/slp.tldr "$SRC"/codex-config.toml "$DST"/
cp "$SRC"/.claude/settings.json "$DST"/.claude/
cp "$SRC"/.claude/skills/herdr-ops/SKILL.md "$DST"/.claude/skills/herdr-ops/
cp "$SRC"/supervisor/supervisor.sh "$DST"/supervisor/
cp "$HOME"/Library/LaunchAgents/com.quaqua.supervisor.plist "$DST"/supervisor/
git -C "$DST" status --short
