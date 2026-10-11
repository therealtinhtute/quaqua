# Workers — routing

Pick by task shape. Name = kind passed to `spawn-worker.sh`. Update "Seen" after each run (from `runs/*.md` + project Log).

| name | harness · model · effort | Use for | Avoid | Seen |
|------|--------------------------|---------|-------|------|
| tieude | pi · devin/deepseek-v4.1-flash · max | cheap well-specced impl, QA passes | ambiguous design | mdqa, mdchip, mdbold, dashcards |
| detu | pi · antigravity/gemini-3.8-flash · high | quick lookup, research, read-only scan | implementation | – |
| sude | pi · devin/swe-2 · max | **default**: every cheap task, hard multi-file impl, audits | – | mdtree, mdstatus, mdflat |
| ni | claude · opus-5.5 · low | small Claude edits, quick answers | design, long work | – |
| suhuynh | claude · opus-5.5 · medium | refactor, review of intent | long mechanical edits | mdside |
| truonglao | claude · opus-5.5 · high | design, architecture, hard trade-offs | anything a cheaper tier can do | – |
| bacdau | codex · astra · medium | specced impl + tests, second opinion | vague tasks (codex state can stick `unknown`) | – |
| quansu | codex · gpt-6.1-sol · high | advice only: review/diagnose; pipeline review findings return to the builder via pipeline.sh, advice-only findings go to CoS | editing code, fixing its own findings | quansu-roster |

## Naming
- Worker name = `<name>-<task>`, lowercase, no diacritics, ≤20 chars, e.g. `sude-fonts`, `quansu-fonts`.
- Task must not end in `-lead` — `*-lead` names are project Leads (`scripts/spawn-lead.sh <slug>` → `<slug>-lead`, own `<slug>` workspace, no worktree); herdr caps agent names at 32 chars, so slugs are ≤27.
- Spawn: `scripts/spawn-worker.sh [--base <ref>] sude-fonts sude <repo>` → branch `quaqua/sude-fonts` (from <ref>, default HEAD), report `runs/sude-fonts.md`.
- Old names (`mdtree`, `mdqa`…) stay as-is.

## Rules
- No name given → `sude`, per `memory/rules.md`.
- Implementer and reviewer must be different harnesses (e.g. sude builds → quansu reviews).
- Read-only question → `detu`; design unclear → `truonglao` before any builder.
- Unsure between two → cheaper tier first (tieude < sude < ni < suhuynh < truonglao); escalate only on a failed run.
