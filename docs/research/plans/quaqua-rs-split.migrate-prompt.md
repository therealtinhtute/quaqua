# Prompt — run the P2 migration (M0–M7)

Paste this into a fresh Claude Code session in `~/quaqua` (the CoS), with anh iu present.

---

Run the `~/quaqua` migration from `docs/research/plans/quaqua-rs-split.md` → `### ~/quaqua migration` (M0–M7). That section is the source of truth: read it in full first, from `origin/main` or the merged plan file, not from memory.

State at handoff (2026-10-20):
- B1–B3 are on `main` (#75 #76 #77 #78 #80 #81). M2 is done. M0, M1, M3–M7 have not run.
- `~/quaqua` is at `bb1f27f` and must **never** `git pull` `main` — that deletes its root `AGENTS.md`/`identity.md`.
- `~/Lab/quaqua` does not exist on this machine: M3 clones it first.
- Known local edit: `rules/_all/worker-timebox.md` → expect it as a conflict in M5.
- herdr plugin re-registers with `herdr plugin link <path>` (M4).
- Decisions: one-time M1 backup, no timer · engine checkout `~/Lab/quaqua` · CoS edits to `AGENTS.md`/`rules/_all` are upstreamed to `templates/workspace/` the same day.

How to run:
1. Do the steps in order: M0 → M1 → M3 → M4 → M5 → M6 → M7. After each step, report what ran, its verify output, and pass/fail in ≤3 lines, then continue.
2. **Stop and ask anh iu (AskUserQuestion)** before M5 step 2 (`mv ~/quaqua/.git …`), and first show anh iu the M5 step 1 `--dry-run` table and the step 3 trash list. Abort if that list is empty or holds a private path (`memory/ projects/ runs/ raw/ reports/ rules/<slug>/ docs/plans/ USER.md scripts/ .claude/`).
3. Any verify fails → stop, do not improvise; follow the plan's **Rollback** for the step you are on and report.
4. Delete with `trash`, never `rm`. No push, merge or branch delete without anh iu's approval. Run the gate smoke `G` where the plan says.
5. M6 spawns one task worker through `quaqua worker spawn/prompt/cleanup` — the CoS does not edit engine code itself.
6. Done when M6 passes and M7's upstream worker commit is reviewed. Then log `- [quaqua-rs] P2 migration M0–M7 <outcome>` in `memory/<date>.md`, update the `quaqua-rs` card and `projects/INDEX.md`, and ask anh iu when to delete the M1 backup and `~/quaqua-pre-p2-<date>.git`.
