# tieude-outgate — `quaqua gate` gates outward actions (git push / gh writes)

Verdict: done. Commit `2908676` on `quaqua/tieude-outgate` (not pushed/merged). Touched only
`crates/quaqua/src/gate/{mod,policy}.rs`, `crates/quaqua/tests/gate.rs`,
`scripts/hooks/herdr-gate.sh`, `tests/test_gate_shim.sh` — `.claude/settings.json`,
`.git/config`, `AGENTS.md` untouched.

## What changed

- `policy.rs` — `Verdict` gains `Ask(reason)` / `Deny(reason)`; new `check_ctx(command, cwd)`; new
  `git` arm (`check_git` → `check_push`) and `gh` arm (`check_gh`) reusing the existing tokenizer
  (so `&&`, `;`, `|`, `cd x &&`, `env VAR=1`, `sudo`, `bash -c '…'`, `git -C <dir>` all work).
  Precedence `Deny > Block > Ask > Allow`: a herdr block outranks an ask so approving a compound
  command can never let a pane/workspace spawn ride through.
- `mod.rs` — reads `cwd` from the hook JSON alongside `.tool_input.command`; `Ask`/`Deny` print
  PreToolUse JSON on **stdout, exit 0** (`{"hookSpecificOutput":{"hookEventName":"PreToolUse",
  "permissionDecision":"ask|deny","permissionDecisionReason":"…"}}`, built via serde_json so no
  reason can break the JSON); `Block` keeps stderr + exit 2; unparseable non-empty stdin still
  fails closed (exit 2). Read-only inputs (empty stdin) still exit 0 silently.
- `scripts/hooks/herdr-gate.sh` — the no-binary fallback now also refuses outward commands with
  **exit 2** (fail-closed superset: any `git … push`, the enumerated gh write subcommands);
  read-only git/gh still pass with the warning. herdr fallback behaviour unchanged.

## Decision table (each row run through `./target/debug/quaqua gate` with hook JSON on stdin)

| command | decision |
|---|---|
| `git push` | ask (exit 0) |
| `git push origin feature` | ask |
| `git push -u origin feature` / `git push --tags origin` | ask |
| `git push origin feature \| cat`, `echo x; git push origin feature` | ask (compound) |
| `cd ~/repo && git push origin topic` | ask (compound) |
| `sudo git push origin feature`, `env GIT_SSH_COMMAND=ssh git push origin feature` | ask |
| `git -C /repo push origin feature`, `git -c push.default=current push origin feature` | ask |
| `bash -c 'git push origin feature'` | ask |
| `gh pr create\|edit\|close\|comment\|ready …`, `gh pr merge 42 --squash` | ask |
| `gh issue create\|edit\|close\|comment …` | ask |
| `gh release create\|upload …` | ask |
| `gh repo create\|delete\|fork\|sync\|transfer …` | ask |
| `git push` (cwd = repo whose checked-out branch is `main`) | deny |
| `git push origin` / `git push -u origin` (same repo, branch `main`) | deny |
| `git -C <main repo> push` (cwd elsewhere) | deny |
| `git push` (cwd = repo on `feature`) → no refspec visible | ask |
| `git push -f origin topic`, `git push --force`, `--force-with-lease[=main]`, `--force-if-includes` | deny (force push) |
| `git push origin +topic` / `+main` | deny (force) |
| `git push origin main` / `master`, `git push origin HEAD:main`, `git push origin feature:main`, `…:refs/heads/master` | deny (push to main) |
| `git push origin :main`, `git push --delete origin main` | deny (deletes main) |
| `git push --all\|--branches\|--mirror` | deny (may push main) |
| `git push origin main && gh pr create` | deny (deny > ask) |
| `git push -f && herdr pane split` | deny (deny > block) |
| `herdr pane split && git push origin feature` | **block, exit 2** (block > ask — unchanged herdr behaviour) |
| `gh pr merge main` / `master` / `refs/heads/main` | deny |
| `git status`, `git log`, `git fetch`, `git diff`, `git show`, `git worktree list`, `git remote -v` | allow (exit 0, silent) |
| `gh pr view\|list\|status\|diff\|checks`, `gh issue view\|list`, `gh release view\|list\|download`, `gh repo view\|list\|clone`, `gh run list`, `gh api`, `gh auth status` | allow |
| `git push --help`, `git push -h`, `git --help`, `gh pr create --help`, `gh pr -h`, `gh -h` | allow (help exits before the verb) |
| `printf '%s\n' 'git push origin main'`, `echo "run git push to deploy"`, `grep -rn "git push" README.md` | allow (argument text) |
| `cat <<EOF … git push --force origin main … EOF` | allow (heredoc body) |

## Unknowns answered

1. **ask in `bypassPermissions` mode — what the docs actually say** (no guessing):
   - `code.claude.com/docs/en/hooks` (PreToolUse decision control): `"ask"` “prompts the user to
     confirm”; `permissionDecisionReason` — “For `"ask"`, shown to the user in the permission
     prompt”; and “A hook’s `"ask"` also forces a permission prompt in **auto mode** … the
     classifier … can’t approve the call silently” (shipped v2.1.211 per the Claude Code
     CHANGELOG: “a hook `ask` now floors the decision at a prompt”). It also documents that in a
     `-p` run where nobody can answer, Claude reads the reason from the tool result instead.
   - `code.claude.com/docs/en/hooks-guide`: `"ask"`: “show the permission prompt to the user as
     normal”.
   - `code.claude.com/docs/en/permission-modes`: `bypassPermissions` “Skips permission prompts,
     except for the [actions no mode auto-approves]”; that list enumerates explicit ask **rules**,
     connector asks, user-interaction tools, critical-path `rm`, cross-session safeguards and
     restricted reads — a hook `ask` is **not** in it. The only place the docs say a mode overrides
     a hook decision is plan mode for `"allow"` (`/docs/en/permissions`, “Hook approvals in plan
     mode”); no page states that bypassPermissions suppresses a hook `ask`, and none states it
     honours one either. **Conclusion: the docs promise the prompt in every mode they describe
     explicitly (default/auto) and never carve bypassPermissions out for hook asks — but they do
     not state bypassPermissions behaviour for a hook `ask` in so many words.** The deny path
     (force push / push to main / `gh pr merge main`) does not depend on this: `"deny"` “prevents
     the tool call” and no documented override exists short of a plugin mod.
2. **`git push` with no refspec to a branch whose upstream is main** — the tokenizer only sees
   words, so the refspec (and therefore the destination) is invisible; per the brief the gate then
   decides from the **checked-out branch name**, read from the hook `cwd` via
   `git -C <dir> symbolic-ref --short -q HEAD` (honouring a `git -C <dir>` in the command itself).
   `main`/`master` → deny; any other/unknown branch → ask. This is an approximation, say so:
   a feature branch whose *upstream* is main, or `push.default=matching`, is not caught (ask);
   a `cd <dir> && git push` compound resolves the branch at the hook cwd, not at `<dir>`.

## Tests

- `cargo test --locked --test gate` → **14 passed / 0 failed** (outward ask, deny, read-only,
  compound precedence, cwd branch resolution, binary JSON contract). Test lines: gate.rs +281/-3.
- `cargo test --locked --lib gate::` → 0 tests (the gate has no in-src unit tests; the suite lives
  in `tests/gate.rs`), exit 0.
- `bash tests/test_herdr_gate.sh` → PASS (fallback path) and
  `QUAQUA_BIN=$PWD/target/debug/quaqua bash tests/test_herdr_gate.sh` → PASS (herdr behaviour
  unchanged through the new binary). Test lines: test_gate_shim.sh +31/-4.
- `bash tests/test_gate_shim.sh` → PASS, now also: 10 outward commands exit 2 fail-closed with no
  binary; 10 read-only git/gh calls still exit 0 with the warning.
- `cargo build --locked --all-targets` clean; clippy reports no warnings in `src/gate/`.

## Deliberate limits / risks

- `gh pr merge` can only see main/master when the branch argument names it; the PR's base branch
  is not in the command text, so a bare `gh pr merge 42` asks rather than denies.
- A lone `git push main` is a git *repository* argument (verified on git 2.56: git treats the
  single positional as the repo, even `HEAD:main`/`:main`); a refspec spelling there is denied
  anyway, a bare `main` falls to the branch rule above.
- `git push --dry-run origin feature` asks (nothing is pushed) — accepted over-ask.
- The fallback over-blocks (`git log --grep push` → exit 2 with no binary); it is the
  missing-binary fence, and its diagnostic says to install quaqua.
- `git push` in a compound after `cd <dir>` uses the hook cwd for the branch lookup (above).

## Blockers

none

## Lessons

- A gate that grows an "ask" tier needs an explicit rank between tiers: block (exit 2) must outrank
  ask, or an approval of a compound command would let the blocked action through as a side effect.
- Doc-required behaviour can be unstated: cite what the docs do say (ask prompts; auto mode floors
  it) and mark the bypassPermissions case unstated rather than guessing it.
