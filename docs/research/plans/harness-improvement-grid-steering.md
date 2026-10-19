# Harness improvement — grid-steering

## Baseline
- representative job: CoS spawns 1–4 workers / an SLP team and arranges panes
- accepted outcome: one spawn call per worker, no extra grid calls, no GRID_SKIP exception
- concrete failure and evidence: 3 conflicting grid instructions — `.claude/rules/herdr-fanout-grid.md:5` (run grid-workers.sh after every spawn), `AGENTS.md` Protocol 1/6 (reconcile MANDATORY after spawn/cleanup), `rules/_all/fanout-agent-grid.md:4` (spawn/cleanup grid themselves). Code: `crates/quaqua/src/worker/spawn.rs:482`, `cleanup.rs:168` call `run_grid_hook`. Session 985f4d82: R0 pipeline grid block + GRID_SKIP exception.
- human steering required: anh iu approved GRID_SKIP exception (2026-10-15)
- worker / revision / tools: CoS Claude Code, main @ dea1236

## Gap owner
- earliest gap: context
- assigned owner: repository-harness

## Intervention
```
If the always-loaded grid rule and AGENTS.md Protocol point to rules/_all/fanout-agent-grid.md
and say spawn/cleanup grid themselves, then a fresh CoS will skip redundant reconcile calls
on a worker spawn, because only one instruction remains.
Evidence that would weaken this: a fresh session still runs reconcile after every spawn, or panes end misplaced without it.
Maintenance owner and removal condition: CoS; remove the pointer file if .claude/rules is dropped.
```

## Native proof
- command: `rg -n "grid-workers.sh BEFORE|Then \`quaqua grid reconcile\`" AGENTS.md .claude/rules/` → no hits; `bash tests/test_docs.sh`
- result: rg no hits (exit 1); tests/test_docs.sh PASS (2026-10-16)

## Fresh rerun
- different session: no
- Decision: pending fresh rerun
- comparison:
