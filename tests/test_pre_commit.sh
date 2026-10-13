#!/usr/bin/env bash
# Test for hooks/pre-commit (rules/_all/portable-paths.md): a commit that ADDS
# a machine-specific absolute path (/home/<name>/…, /Users/<name>/…) is
# rejected; the policy file and tests/** are allowlisted; existing content is
# never re-judged; ~/, $HOME and placeholders pass.
#
# The sandbox is a throwaway `git init` repo in a temp dir — the real
# workspace's index and hooks are never touched. The hook is invoked both
# directly and through a real `git commit` with core.hooksPath pointed at
# scripts/hooks/ (what init-workspace.sh wires).
# Usage: tests/test_pre_commit.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
hookdir="$(cd "$here/.." && pwd)/scripts/hooks"
hook="$hookdir/pre-commit"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-pre-commit.XXXXXX")
T=$(cd "$T" && pwd -P)
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
[[ -f $hook ]] || fail "missing hook: scripts/hooks/pre-commit"

R="$T/repo"
mkdir -p "$R"
git -C "$R" init -q
git -C "$R" config user.email test@example.invalid
git -C "$R" config user.name test
git -C "$R" config core.hooksPath "$hookdir"   # the wiring init-workspace.sh installs

# hook_rc — run the hook directly from the sandbox repo (hooks read the index
# of their cwd); stdout+stderr land in $T/out
hook_rc() { local rc=0; (cd "$R" && bash "$hook") >"$T/out" 2>&1 || rc=$?; echo "$rc"; }
commit_rc() { local rc=0; git -C "$R" commit -qm "subject" >"$T/out" 2>&1 || rc=$?; echo "$rc"; }
stage() { git -C "$R" add -A; }
reset_all() { git -C "$R" reset -q --hard HEAD >/dev/null 2>&1 || true; git -C "$R" clean -qfd; }

# baseline: a policy file and a tests/ fixture (the two allowlisted paths)
mkdir -p "$R/tests" "$R/rules/_all" "$R/src"
printf '# portable paths\n' > "$R/rules/_all/portable-paths.md"
printf '#!/usr/bin/env bash\necho baseline\n' > "$R/tests/fixture.sh"
printf '#!/usr/bin/env bash\necho baseline\n' > "$R/src/tool.sh"
printf 'baseline\n' > "$R/README.md"
stage; git -C "$R" commit -qm baseline
echo "ok: sandbox repo initialized (core.hooksPath -> scripts/hooks)"

# ── 1. an added machine path in a script is rejected ───────────────────────
printf '#!/usr/bin/env bash\ncd /home/alice/project\n' > "$R/src/tool.sh"
stage
[[ $(hook_rc) == 1 ]] || fail "added /home/<user>/ path not rejected (direct run)"
grep -q 'src/tool.sh' "$T/out" || fail "hook output does not name the offending file"
grep -q 'home/alice/project' "$T/out" || fail "hook output does not show the offending line"
grep -q 'portable-paths' "$T/out" || fail "hook output does not point at rules/_all/portable-paths.md"
echo "ok: added /home/<user>/ path -> rejected, file + line + policy named"

# and through a real commit, so the installed hook really fires
[[ $(commit_rc) != 0 ]] || fail "git commit succeeded despite a machine path"
grep -q 'src/tool.sh' "$T/out" || fail "commit output does not name the offending file"
reset_all
echo "ok: real git commit through core.hooksPath -> blocked"

# ── 2. /Users/<user>/ is rejected too (macOS form) ─────────────────────────
printf '#!/usr/bin/env bash\nopen /Users/bob/notes\n' > "$R/src/tool.sh"
stage
[[ $(hook_rc) == 1 ]] || fail "added /Users/<user>/ path not rejected"
grep -q 'Users/bob/notes' "$T/out" || fail "hook output does not show the macOS path"
reset_all
echo "ok: added /Users/<user>/ path -> rejected"

# ── 3. allowlist: tests/** and rules/_all/portable-paths.md ────────────────
printf '#!/usr/bin/env bash\ncd /home/alice/project\n' > "$R/tests/fixture.sh"
stage
[[ $(hook_rc) == 0 ]] || fail "tests/** fixture rejected but must be allowlisted"
echo "ok: tests/** fixture with a machine path -> allowed"

printf '# portable paths\n- never /home/alice/…\n' > "$R/rules/_all/portable-paths.md"
stage
[[ $(hook_rc) == 0 ]] || fail "rules/_all/portable-paths.md rejected but must be allowlisted"
echo "ok: rules/_all/portable-paths.md with the pattern -> allowed"

# the allowlist is exact, not a prefix match: a sibling file is still checked
printf '#!/usr/bin/env bash\ncd /home/alice/project\n' > "$R/tests-helper.sh"
stage
[[ $(hook_rc) == 1 ]] || fail "tests-helper.sh is not under tests/ and must be checked"
reset_all; rm -f "$R/tests-helper.sh"
echo "ok: allowlist is exact (tests-helper.sh still checked)"

# ── 4. portable spellings and placeholders pass ───────────────────────────
{
  printf '#!/usr/bin/env bash\n'
  printf 'a=$HOME/thing\n'
  printf 'b=~/thing\n'
  printf 'c=$(dirname "$0")/thing\n'
  printf 'd=/home/<user>/thing\n'
  printf 'e=/Users/<name>/thing\n'
  printf 'f=/etc/hosts\n'
  printf 'g=/usr/local/bin\n'
} > "$R/src/tool.sh"
stage
[[ $(hook_rc) == 0 ]] || fail "portable spellings/placeholders rejected: $(cat "$T/out")"
reset_all
echo "ok: ~/, \$HOME, \$(dirname \"\$0\"), placeholders, /etc -> allowed"

# ── 5. only ADDED lines are judged (existing violations do not brick) ─────
printf '#!/usr/bin/env bash\ncd /home/legacy/project\n' > "$R/src/legacy.sh"
stage
git -C "$R" commit -q --no-verify -m "legacy violation"   # pre-existing state
printf '#!/usr/bin/env bash\ncd /home/legacy/project\necho more\n' > "$R/src/legacy.sh"
stage
[[ $(hook_rc) == 0 ]] || fail "a clean modification of a legacy-violating file was rejected"
grep -q 'echo more' "$R/src/legacy.sh" || fail "modification did not land"
reset_all
echo "ok: existing violations are not re-judged; clean edits still commit"

# ── 6. renames: content is re-checked only when the commit adds lines ─────
git -C "$R" mv src/legacy.sh src/renamed.sh
stage
[[ $(hook_rc) == 0 ]] || fail "a pure rename re-judged the file's existing content"
printf '#!/usr/bin/env bash\ncd /home/legacy/project\ncd /home/carol/other\n' > "$R/src/renamed.sh"
stage
[[ $(hook_rc) == 1 ]] || fail "rename + added machine path not rejected"
grep -q 'src/renamed.sh' "$T/out" || fail "rename+modify output does not name the new path"
reset_all
echo "ok: pure rename allowed; rename + added machine path rejected under the new name"

# ── 7. binary files pass (no lines to scan) ───────────────────────────────
printf '\x00\x01\x02/home/alice/inside\x00\x03' > "$R/blob.bin"
stage
[[ $(hook_rc) == 0 ]] || fail "binary file rejected"
reset_all; rm -f "$R/blob.bin"
echo "ok: binary diff -> allowed (no added lines)"

# ── 8. empty index, and the unborn-branch first commit ───────────────────
[[ $(hook_rc) == 0 ]] || fail "clean index rejected"
F="$T/fresh"
mkdir -p "$F"; git -C "$F" init -q
git -C "$F" config core.hooksPath "$hookdir"
printf '#!/usr/bin/env bash\ncd /home/dave/x\n' > "$F/first.sh"
git -C "$F" add -A
( cd "$F" && bash "$hook" ) >"$T/out" 2>&1 && fail "unborn-branch first commit with a machine path was allowed"
grep -q 'first.sh' "$T/out" || fail "unborn-branch output does not name the file"
echo "ok: clean index -> allowed; unborn-branch first commit with a machine path -> blocked"

# ── 9. the hook source itself must stay portable ─────────────────────────
if grep -qE '/(home|Users)/[A-Za-z0-9._-]+' "$hook"; then
  fail "scripts/hooks/pre-commit contains a literal machine path"
fi
[[ -x $hook ]] || fail "scripts/hooks/pre-commit is not executable (git needs it to run)"
echo "ok: hook source is free of literal machine paths and is executable"

echo "PASS"
