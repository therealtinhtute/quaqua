# tieude-sweep — sweep branch + worktree cũ trong ~/quaqua

Base: `main` = 573b8f0 (unchanged). No push, no remote operation. `git branch -D` never needed — all 35 deletions passed `-d`.
Summary: branches 47 → 12 (35 merged `quaqua/*` deleted); worktrees 4 → 2 (clean-integ, unified-integ removed).

## Changes

### 1) Worktrees removed (2, no force — both clean beforehand)
- `~/.wt/clean-integ` (branch `quaqua/clean-integ` @ e367313): `git status --porcelain` empty → `git worktree remove` OK.
- `~/.wt/unified-integ` (branch `quaqua/unified-integ` @ a708fcd): clean → removed OK.
- Command: `git -C ~/quaqua worktree remove <path>` (never `--force`; no dirty/untracked case encountered).

### 2) Branches deleted — 35 merged `quaqua/*`, all via `git branch -d` (35/35, `-D` fallback never triggered)
quaqua/clean-integ e367313 · quaqua/quaqua-grid-integ 0b43802 · quaqua/quaqua-rs-integ 70d6caa · quaqua/sude-archaudit a708fcd · quaqua/sude-auditor e69c1eb · quaqua/sude-bashclean a35a54d · quaqua/sude-finalfix e367313 · quaqua/sude-o2 d1de5f5 · quaqua/sude-pollaud ae9a7ef · quaqua/sude-pollpre 72e54b1 · quaqua/sude-pollusage 49db2fd · quaqua/sude-testaudit 2bd0732 · quaqua/sude-w1 20cd4d2 · quaqua/sude-w2 22e6a76 · quaqua/sude-w3 463732f · quaqua/sude-w3m d73c1f8 · quaqua/sude-w4a 5d22e4b · quaqua/sude-w4b c404f9b · quaqua/sude-w5 255942d · quaqua/sude-w6 6b65929 · quaqua/suhuynh-bashcleanr1 a35a54d · quaqua/suhuynh-finalreview c404f9b · quaqua/suhuynh-w1r1 cfbda41 · quaqua/suhuynh-w1r2 20cd4d2 · quaqua/suhuynh-w2r1 dfff138 · quaqua/suhuynh-w2r2 22e6a76 · quaqua/suhuynh-w3r1 f33ddfa · quaqua/suhuynh-w3r2 463732f · quaqua/suhuynh-w5r1 7d3dc4e · quaqua/suhuynh-w6r1 1c10944 · quaqua/suhuynh-w6r2 6b65929 · quaqua/tieude-ofix 83a8c56 · quaqua/tieude-plugren a511231 · quaqua/tieude-t1fix 694f425 · quaqua/unified-integ a708fcd

### 3) Kept + reason
- `main` @ 573b8f0 — base.
- 7 unmerged `quaqua/*` (`git merge-base --is-ancestor` non-zero), matches brief exactly: bacdau-gridplan df4e33e, tieude-clkfake 904d5e1, tieude-evtprobe 99e1b34, tieude-gridconv d844f53, tieude-omp 452d269, tieude-wdflaky 91994d3, truonglao-gridplan df4e33e.
- `quaqua/tieude-sweep` @ 573b8f0 — merged, but checked out in `~/.wt/tieude-sweep`; git refused: `error: cannot delete branch 'quaqua/tieude-sweep' used by worktree at '~/.wt/tieude-sweep'` (own branch; delete after this worktree's cleanup — CoS follow-up).
- Outside prefix, untouched: chore/zharness 268da0c (unmerged), docs/herdr-grid-plan 0f97eea (unmerged), docs/rust-herdr-plugin-brainstorm 95eadd1 (merged).

## Evidence
- Before: `git branch | wc -l` = 47 (43 `quaqua/*` = 36 merged + 7 unmerged); `git worktree list` = quaqua [main], .wt/clean-integ, .wt/tieude-sweep, .wt/unified-integ. (CoS's 46/42 predates `quaqua/tieude-sweep`.)
- Delete gate: per branch `git merge-base --is-ancestor <b> main` exit 0 required; 7 `KEEP-UNMERGED` lines in the run log confirm the gate fired.
- After: `git branch | wc -l` = 12; `git worktree list` = `~/quaqua [main]` + `~/.wt/tieude-sweep [quaqua/tieude-sweep]` only; `ls ~/.wt/clean-integ ~/.wt/unified-integ` → `No such file or directory` (both).
- Deleted count: 35 `DELETED` lines in run log; `git log -1 main` still 573b8f0.
- Commands per group: worktrees → `git -C ~/quaqua worktree remove <path>` (dirtiness pre-checked `git -C <wt> status --porcelain`); branches → loop over `refs/heads/quaqua`: ancestor check → `git branch -d`; on refusal: re-check ancestor/worktree → `-D` (skipped, never needed); checked-out → skip.

## Test
Verification commands + results:
- `git merge-base --is-ancestor <b> main` for every held branch → all kept branches non-ancestor or non-`quaqua/` (PASS).
- `git branch | wc -l` → 47 → **12** (PASS).
- `git worktree list` → only `main` + `tieude-sweep` worktrees remain (PASS).
- Both target dirs deleted from disk (PASS).
Result: PASS.

## Risks
- Deleted SHAs remain reachable via reflog (~90 days) — recoverable if a merge was misjudged; gate used strict ancestor check, so risk minimal.
- No network command run; remote refs untouched.
- Untracked CoS files in `~/quaqua/rules/_all/*.md` noticed and left as-is (not mine).
- `quaqua/tieude-sweep` still exists (blocked by my worktree); needs deletion at cleanup.

## Blockers
none

## Lessons
Sweep order matters: `git worktree remove` stale integ worktrees before branch deletion (frees their checked-out branches); merged `quaqua/*` never need `-D` — all 35 passed `-d`.
