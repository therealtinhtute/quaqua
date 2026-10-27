# Remaining work after pstack MVP + farm P0–P7 (2026-10-25)

Owner: CoS. Source of truth for what is left; each row ends in a check.

## Wave 1 (running, disjoint file areas)
| Unit | Team | Area | Done when |
|---|---|---|---|
| farm P6 consumers (absorbs pstack `watch`/drain trigger) | bus-lead | `bus/`, watchdog | DONE fa5144f on bus-integ (CoS re-ran 29 ok) |
| Test hermeticity: tests write live `runs/quaqua.db` (342/344 `would_deny` = `baro-lead` fixtures); QUAQUA_OWNER leak | gate-lead | test harness, `lead/prompt.rs` | live DB row count unchanged by full suite; genuine would_deny judged |
| Verdict level vocabulary (45/67 `inconclusive`) | tieude-verdictlvl | `engine/journal_sqlite.rs`, `worker/prompt`, rules doc | DONE 3e09c7e (verdict 32 + vocabulary 2 ok); no backfill |
| Lead per-Peer routing rule | tieude-leadroute | template AGENTS.md | DONE 5951d54 (tests 29+11 ok) |

## Wave 2 (gated)
- P4 gate enforce: after Wave 1 hermeticity merges, collect a clean shadow window (≥2 days real use) with 0 false denies (CP3). gate-lead.
- P8 user layouts + council: after P4 (CP4). freeze-lead.
- #138 report frontmatter: brainstorm after verdict vocabulary lands (overlaps `status`).
- S0 baseline: start a closed measurement window now so G1–G3 get a delta next time.

## Needs anh iu (irreversible / preference)
- Push + PR: quaqua/tieude-s8close (3d04669), quaqua/tieude-leadroute (5951d54), quaqua/tieude-verdictlvl (3e09c7e), later Wave 1 branches.
- Delete stale remotes: `pstack-integ` (dup), `quaqua/gate-integ` (3691090).
- Clean the 342 fixture rows from live runs/quaqua.db after the fix (data delete).
- `.quaqua/pending/models.md`, `team.toml` conflicts.
