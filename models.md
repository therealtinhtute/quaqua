# Workers — routing

Pick by task shape. Name = kind passed to `quaqua worker spawn`. Update "Seen" after each run (from `runs/*.md` + project Log).

| name | harness · model · effort | Use for | Avoid | Seen |
|------|--------------------------|---------|-------|------|
| tieude | opencode · opencode-go/deepseek-v4.1-flash · max | **default**: every task unless anh iu names another; cheap well-specced impl, QA passes | ambiguous design | ci, gatefix, gridfix, mainsync, promptfix, slowtests (+10) |
| detu | omp · google-antigravity/gemini-3.8-flash · high | quick lookup, research, read-only scan | implementation | cleanexport |
| sude | omp · google-antigravity/gemini-3.8-flash · high | hard multi-file impl, audits (on request) | – | bpdeps, bpdocs, bpga |
| ni | claude · opus-5.5 · low | small Claude edits, quick answers | design, long work | – |
| suhuynh | claude · opus-5.5 · medium | refactor, review of intent | long mechanical edits | r0r1, pbslim, dashstyle |
| truonglao | claude · opus-5.5 · high | design, architecture, hard trade-offs | anything a cheaper tier can do | gridplan, bpaudit, pbglobal |

## Naming
- Worker name = `<name>-<task>`, lowercase, no diacritics, ≤20 chars, e.g. `sude-fonts`, `suhuynh-fonts`.
- Task must not end in `-lead` — `*-lead` names are project Leads (`quaqua lead spawn <slug>` → `<slug>-lead`, own `<slug>` workspace, no worktree); herdr caps agent names at 32 chars, so slugs are ≤27.
- Spawn: `quaqua worker spawn [--base <ref>] tieude-fonts tieude <repo>` → branch `quaqua/tieude-fonts` (from <ref>, default HEAD), report `runs/tieude-fonts.md`.
- Old names (`mdtree`, `mdqa`…) stay as-is.

## Rules
- No name given → `tieude` (anh iu, 2026-10-15).
- Implementer and reviewer must be different harnesses (e.g. sude builds → suhuynh reviews).
- Read-only question → `detu`; design unclear → `truonglao` before any builder.
- Unsure between two → cheaper tier first (tieude < sude < ni < suhuynh < truonglao); escalate only on a failed run.

## Claude out — ACTIVE since 2026-10-17 (only when anh iu says Claude tokens are out)
- CoS: anh iu opens opencode in `~/quaqua` (reads AGENTS.md); persona and rules unchanged.
- suhuynh → opencode · opencode-go/deepseek-v4.1-flash · max (anh iu, 2026-10-17); ni/truonglao paused.
- Builder in this mode = `sude` (omp), so the reviewer stays on a different harness.
- Back to the table above when anh iu says Claude is back.
