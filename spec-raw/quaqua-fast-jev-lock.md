# Lock: fast-jev-compaction in quaqua (project scope) — 2026-10-08

Source: https://github.com/tamaratran/fast-jev-compaction (Claude Code function-hook plugin; Jev prunes stale tool calls/results instead of summarizing).

## Decisions (anh iu)
- Scope: project scope `~/quaqua` → CoS + orchestrators. Workers (other repos' worktrees) unaffected.
- Data egress accepted: full transcript sent to api.typesafe.ai on each compact.
- Auto-compact kept at `compactAtPercent=60` (plugin defaults for all options).
- Key: `TYPESAFE_API_KEY` from env (auto-loaded by anh iu's shell); `apiKey` option left empty; no key in any file.

## Change
1. `claude plugin marketplace add tamaratran/fast-jev-compaction`
2. `claude plugin install fast-jev-compaction@fast-jev-compaction --scope project` (in `~/quaqua`)
3. Merge into `~/quaqua/.claude/settings.json` (keep `permissions.deny: ["Agent"]`):
   `"env": { "CLAUDE_CODE_ENABLE_FUNCTION_HOOKS": "1" }`, `"enabledPlugins": { "fast-jev-compaction@fast-jev-compaction": true }`
   (`.claude/` is gitignored → local only.)

## Verify
- Restart Claude in `~/quaqua`, run `/compact` → toast `fast-jev-compaction: kept N/M…` (or `fallback…` = hook loaded, session too short).
- Risk: types generated for 2.1.274, local is 2.1.293 → hook may not load.

## Rollback
Remove the two keys from settings.json; `claude plugin uninstall fast-jev-compaction@fast-jev-compaction --scope project`.
