# AGENTS.md — quaqua engine

This repository is the **engine** (`crates/quaqua`), not a workspace. Your worker
rules arrive in `runs/<name>.rules.md` (`crates/quaqua/src/worker/spawn.rs`).

- The CoS document for the consuming workspace is `templates/workspace/AGENTS.md`.
- Managed workspace files live under `templates/workspace/`; seed-once files under `templates/seed/`.
- Engine-only: `crates/`, `tests/`, `plugins/`, `docs/`, `.github/`, `Cargo.*`, `README.md`, `SETUP-PROMPT.md`.
- Never write private workspace state (`memory/`, `projects/`, `runs/`, `raw/`, `reports/`, `USER.md`) here.
- The PreToolUse gate for engine worktrees is `.claude/settings.json` → `templates/workspace/scripts/hooks/herdr-gate.sh`.
- Work only in your worktree: no push, no merge to `main`, no branch deletes without anh iu's approval.
- Run only the targeted tests named in your brief, in the foreground.
