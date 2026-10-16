# Workers — routing

Pick by task shape. Name = kind passed to `spawn-worker.sh`. Update "Seen" after each run (from `runs/*.md` + project Log).

| name | harness · model · effort | Use for | Avoid | Seen |
|------|--------------------------|---------|-------|------|
| tieude | opencode · opencode-go/deepseek-v4.1-flash | **default**: every task unless anh iu names another; cheap well-specced impl, QA passes | ambiguous design | mdqa, mdchip, mdbold |
| detu | opencode · opencode-go/deepseek-v4.1-flash | quick lookup, research, read-only scan | implementation | – |
| sude | omp · google-antigravity/gemini-3.8-flash · high | hard multi-file impl, audits (on request) | – | mdtree, mdstatus, mdflat |
| ni | claude · opus-5.5 · low | small Claude edits, quick answers | design, long work | – |
| suhuynh | claude · opus-5.5 · medium | refactor, review of intent | long mechanical edits | mdside |
| truonglao | claude · opus-5.5 · high | design, architecture, hard trade-offs | anything a cheaper tier can do | – |

## Naming
- Worker name = `<name>-<task>`, lowercase, no diacritics, ≤20 chars, e.g. `sude-fonts`, `suhuynh-fonts`.
- Task must not end in `-lead` — `*-lead` names are project Leads (`scripts/lead/spawn-lead.sh <slug>` → `<slug>-lead`, own `<slug>` workspace, no worktree); herdr caps agent names at 32 chars, so slugs are ≤27.
- Spawn: `scripts/worker/spawn-worker.sh [--base <ref>] tieude-fonts tieude <repo>` → branch `quaqua/tieude-fonts` (from <ref>, default HEAD), report `runs/tieude-fonts.md`.
- Old names (`mdtree`, `mdqa`…) stay as-is.

## Rules
- No name given → `tieude` (anh iu, 2026-10-15).
- Implementer and reviewer must be different harnesses (e.g. sude builds → suhuynh reviews).
- Read-only question → `detu`; design unclear → `truonglao` before any builder.
- Unsure between two → cheaper tier first (tieude < sude < ni < suhuynh < truonglao); escalate only on a failed run.
