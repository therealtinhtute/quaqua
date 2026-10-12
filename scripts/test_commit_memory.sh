#!/usr/bin/env bash
# Sandbox test for commit-memory.sh: a temp repo mirroring this checkout's
# layout (real .gitignore — memory/, projects/ and raw/ are all ignored, so
# seed notes are force-added to model tracked notes inside ignored dirs).
# The script under test is copied into <repo>/scripts/ and always invoked from
# a foreign cwd — the repo must be resolved from the script location.
# Usage: scripts/test_commit_memory.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-commit-memory.XXXXXX")
T=$(cd "$T" && pwd -P)  # normalize trailing slashes and macOS /tmp symlinks
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── sandbox: temp repo mirroring this checkout ──────────────────────────────
R="$T/quaqua"
mkdir -p "$R/scripts" "$R/memory" "$R/projects" "$R/raw" "$R/docs"
cp "$here/commit-memory.sh" "$R/scripts/commit-memory.sh"
cp "$here/../.gitignore" "$R/.gitignore"  # same ignores as this repo (memory/ projects/ raw/ all ignored)
echo m > "$R/memory/m.md"; echo p > "$R/projects/p.md"
echo s > "$R/scripts/s.sh"; echo d > "$R/docs/d.md"
git -C "$R" init -q
# local identity: CI runners have no global user.name/user.email, and the
# script under test commits without -c flags — the repo must carry its own.
git -C "$R" config user.name "Test"
git -C "$R" config user.email "test@example.com"
git -C "$R" add -A
# note dirs are gitignored here; force-add the seed notes so they are tracked —
# the only way a note under memory/ projects/ raw/ can ever be committed in
# this checkout (untracked ignored notes are deliberately never staged).
git -C "$R" add -f memory/m.md projects/p.md
git -C "$R" commit -qm seed

reset() { git -C "$R" reset --hard -q HEAD && git -C "$R" clean -fdxq; }
run() { (cd / && bash "$R/scripts/commit-memory.sh" "$1"); }  # foreign cwd
staged() { git -C "$R" diff --cached --name-only --no-renames | sort; }
committed() { git -C "$R" show --format= --name-only HEAD | sort; }

# ── 1. notes-only commit succeeds; unstaged/ignored files are left alone ────
echo change >> "$R/memory/m.md"
echo change >> "$R/projects/p.md"
echo dirty  >> "$R/scripts/s.sh"   # unstaged, must NOT be committed
echo e      > "$R/raw/e.md"        # ignored, must NOT be committed
echo n      > "$R/memory/new.md"   # untracked inside an ignored dir — must NOT be committed
run "log today" >/dev/null || fail "notes-only commit refused"
[[ "$(committed)" == $'memory/m.md\nprojects/p.md' ]] || fail "commit contents: $(committed)"
body=$(git -C "$R" log -1 --format=%B)
[[ "$(head -1 <<<"$body")" == "log today" ]] || fail "message lost: $body"
[[ "$(tail -1 <<<"$body")" == "👾 · from therealTINHTUTE with love" ]] || fail "attribution: $body"
[[ -n $(git -C "$R" status --porcelain -- scripts/s.sh) ]] || fail "scripts/s.sh change vanished"
[[ -f $R/memory/new.md ]] || fail "untracked ignored note vanished"
echo "ok: memory+projects committed on the branch; unstaged/ignored untouched"

# ── 2. a pre-staged non-note file refuses; index left untouched ─────────────
reset
echo dirty >> "$R/scripts/s.sh"; echo note >> "$R/memory/m.md"
git -C "$R" add scripts/s.sh
head_before=$(git -C "$R" rev-parse HEAD)
if out=$(run "x" 2>&1); then fail "pre-staged non-note: expected refusal"; fi
[[ "$out" == *"scripts/s.sh"* ]] || fail "unclear refusal: $out"
[[ "$(git -C "$R" rev-parse HEAD)" == "$head_before" ]] || fail "HEAD moved on refusal"
[[ "$(staged)" == "scripts/s.sh" ]] || fail "index mutated on refusal: $(staged)"
echo "ok: pre-staged scripts/s.sh -> refusal, no commit, index untouched"

# ── 3. a staged rename scripts/x -> memory/x refuses ────────────────────────
reset
git -C "$R" mv scripts/s.sh memory/s.sh  # staged rename; --no-renames must expose scripts/s.sh
if out=$(run "x" 2>&1); then fail "rename bypass: expected refusal"; fi
[[ "$out" == *"scripts/s.sh"* ]] || fail "rename source not reported: $out"
[[ "$(staged)" == $'memory/s.sh\nscripts/s.sh' ]] || fail "index mutated: $(staged)"
echo "ok: staged rename scripts/s.sh -> memory/s.sh refuses"

# ── 4. nothing to commit refuses ────────────────────────────────────────────
reset
mkdir -p "$R/raw"; echo ignored > "$R/raw/only.md"  # raw/ is gitignored -> nothing stageable
if out=$(run "x" 2>&1); then fail "nothing-to-commit: expected refusal"; fi
[[ "$out" == *"nothing to commit"* ]] || fail "unclear refusal: $out"
echo "ok: clean notes -> 'nothing to commit' refusal"

# ── 5. an already-staged raw/ note is allowed and committed ─────────────────
reset
mkdir -p "$R/raw"; echo e > "$R/raw/e.md"
git -C "$R" add -f raw/e.md  # staged note inside an ignored dir
run "evidence" >/dev/null || fail "staged raw note refused"
[[ "$(committed)" == "raw/e.md" ]] || fail "commit contents: $(committed)"
echo "ok: staged raw/ note commits (allowlist covers ignored dirs)"

echo "PASS"
