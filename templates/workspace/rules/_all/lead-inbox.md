# Lead → CoS inbox

- A Lead reports to the CoS via `quaqua mail send --to cos --kind <kind> [--body-file -]`, never by `herdr agent prompt` to the CoS pane: a prompt lands in anh iu's main chat and is rejected while the CoS waits on a question — 2026-10-19, arch-lead E-side report.
- `mail send` writes to SQLite and dual-writes the entry to `~/quaqua/runs/<slug>-lead.inbox.md`. If the database is missing or corrupt, it falls back to the markdown append automatically with a warning.
- Manual fallback (when quaqua is not on PATH or database is offline): append in a single write (`cat >> ~/quaqua/runs/<slug>-lead.inbox.md <<'EOF'`), first line `## <ISO-8601 time> <lead-name> <kind>`, kind = `handoff | accept | challenge | escalate | blocked`, then ≤10 lines: verdict, SHA, verify output, cost, ask.
- Append only; never rewrite or truncate the file. The CoS answers by prompting the Lead.
