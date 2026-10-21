# Prompt — run the P2 migration (M0–M7)

## Outcome (2026-10-21) — **DONE, M0–M7 all PASS**
The prompt was run on 2026-10-21 with anh iu present; the state block below is the pre-run handoff and is kept for the record. Where it was wrong, this list wins:

- `~/quaqua` was at `6c8934c`, not `bb1f27f`; it never pulled `main`, and after M5 it is a non-git workspace with `.quaqua/template.lock` (`engine: 0.1.0+47f4903`), no `.git`, no `crates/`.
- `~/Lab/quaqua` **did exist** (old bash-era checkout `3889cc8`, 39 local commits, 17 branches); it was moved to `~/Lab/quaqua-old-2026-10-12` (anh iu), then a fresh clone landed at `14e7d96` and 214 `quaqua/*` refs were imported.
- M0 first failed on 3 live worktrees + 2 prunable `/tmp` ones; cleaned/pruned, then PASS.
- M4: the live herdr entry was the **dead** `herdr-grid` (missing path). anh iu ran the swap; the isolated smoke needed a short socket dir. `herdr-quaqua` now links from `~/Lab/quaqua/plugins/herdr-quaqua`.
- M5: both conflicts (`rules/_all/fanout-agent-grid.md`, `rules/_all/worker-timebox.md`) took the template version (anh iu); the manifest's 4 mismatches were all expected.
- M6 PASS (`tieude-p2smoke`, hook block confirmed in a fresh Claude session in `~/quaqua`). M7 upstream `095edc7` (`quaqua/tieude-p2upsync`) became PR #83, squash-merged as `47f4903` — not a `main` ancestor by design.
- Cleanup 2026-10-21 (anh iu): M1 bundle/tar/md5, `~/quaqua-pre-p2-2026-10-21.git`, `~/Lab/quaqua-old-2026-10-12`, `plugins.json.pre-m4`, the smoke/upsync branches, and all 213 local + 96 remote stale `quaqua/*` branches were deleted. Only `quaqua/truonglao-p2tmpl` (PR #74) remains.
- Still open: `quaqua init --git-hooks <repo>` (Challenge 4), being built separately (`tieude-githooks`).

Paste this into a fresh Claude Code session in `~/quaqua` (the CoS), with anh iu present.

---

Run the `~/quaqua` migration from `docs/research/plans/quaqua-rs-split.md` → `### ~/quaqua migration` (M0–M7). That section is the source of truth: read it in full first, from `origin/main` or the merged plan file, not from memory.

State at handoff (2026-10-20 — historical; the Outcome above supersedes it):
- B1–B3 are on `main` (#75 #76 #77 #78 #80 #81). M2 is done. M0, M1, M3–M7 had not run at handoff; all ran and passed on 2026-10-21.
- `~/quaqua` was at `6c8934c` (this note said `bb1f27f`, which was stale) and must **never** `git pull` `main` — that deletes its root `AGENTS.md`/`identity.md`.
- `~/Lab/quaqua` held an old bash-era checkout (`3889cc8`): M3 moved it to `~/Lab/quaqua-old-2026-10-12` and cloned fresh.
- Known local edit: `rules/_all/worker-timebox.md` → expected as a conflict in M5; it and `rules/_all/fanout-agent-grid.md` both took the template version.
- The live herdr entry pointed at the dead `~/quaqua/plugins/herdr-grid` (missing path); M4 links `~/Lab/quaqua/plugins/herdr-quaqua`.
- Decisions: one-time M1 backup, no timer · engine checkout `~/Lab/quaqua` · CoS edits to `AGENTS.md`/`rules/_all` are upstreamed to `templates/workspace/` the same day.

How to run:
1. Do the steps in order: M0 → M1 → M3 → M4 → M5 → M6 → M7. After each step, report what ran, its verify output, and pass/fail in ≤3 lines, then continue.
2. **Stop and ask anh iu (AskUserQuestion)** before M5 step 2 (`mv ~/quaqua/.git …`), and first show anh iu the M5 step 1 `--dry-run` table and the step 3 trash list. Abort if that list is empty or holds a private path (`memory/ projects/ runs/ raw/ reports/ rules/<slug>/ docs/plans/ USER.md scripts/ .claude/`).
3. Any verify fails → stop, do not improvise; follow the plan's **Rollback** for the step you are on and report.
4. Delete with `trash`, never `rm`. No push, merge or branch delete without anh iu's approval. Run the gate smoke `G` where the plan says.
5. M6 spawns one task worker through `quaqua worker spawn/prompt/cleanup` — the CoS does not edit engine code itself.
6. Done when M6 passes and M7's upstream worker commit is reviewed. Then log `- [quaqua-rs] P2 migration M0–M7 <outcome>` in `memory/<date>.md`, update the `quaqua-rs` card and `projects/INDEX.md`, and ask anh iu when to delete the M1 backup and `~/quaqua-pre-p2-<date>.git`. **Done 2026-10-21**: M6 passed, the M7 upstream was reviewed and merged as PR #83 (`47f4903`), the log/card/INDEX were updated, and the M1 backup plus `~/quaqua-pre-p2-2026-10-21.git` were deleted (anh iu).
