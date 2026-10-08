#!/usr/bin/env bash
# Sandbox test for prompt-worker.sh: arg validation, the exact herdr prompt
# text, waiting for ~/quaqua/runs/<name>.md after herdr returns early, and the
# PROMPT_WORKER_MAX_SEC cap. herdr is stubbed on PATH (it logs calls and can
# fabricate the runs file after FAKE_DELAY seconds); HOME is a temp dir. No
# real herdr, agent, or home touched.
# Usage: scripts/test_prompt_worker.sh   (exit 0 = pass)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
T=$(mktemp -d "${TMPDIR:-/tmp}/test-prompt-worker.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── sandbox: stub herdr, temp HOME/quaqua/runs ──────────────────────────────
mkdir -p "$T/bin" "$T/home/quaqua/runs"
cat > "$T/bin/herdr" <<'STUB'
#!/usr/bin/env bash
echo "herdr $*" >> "$CALLS"
if [ "${FAKE_DELAY:-0}" != 0 ]; then
  ( sleep "$FAKE_DELAY"; echo "fake summary" > "$HOME/quaqua/runs/$FAKE_NAME.md" ) &
fi
exit "${FAKE_RC:-0}"
STUB
chmod +x "$T/bin/herdr"

prompt() {  # prompt <name> <task> [VAR=val...]; sets rc, $T/out, $T/err, $T/calls
  local name=$1 task=$2; shift 2
  : > "$T/calls"
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" FAKE_NAME="$name" \
    env "$@" bash "$here/prompt-worker.sh" "$name" "$task" >"$T/out" 2>"$T/err" || rc=$?
}

reject() {  # reject <label> [args...]
  local label=$1; shift
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
    bash "$here/prompt-worker.sh" "$@" >"$T/out" 2>"$T/err" || rc=$?
  [[ $rc -eq 1 ]] || fail "$label: exit $rc, want 1"
  grep -q '^# Usage:' "$T/err" || fail "$label: no usage on stderr"
  echo "ok: $label -> exit 1, usage on stderr"
}

brief='Goal: do the thing
Hard constraints: keep tests passing
Current design (open to challenge): via the existing helper'

reject_brief() {  # reject_brief <label> <task>: missing headers -> exit 1
  local label=$1 task=$2
  rc=0
  CALLS="$T/calls" HOME="$T/home" PATH="$T/bin:$PATH" \
    bash "$here/prompt-worker.sh" sude-brief "$task" >"$T/out" 2>"$T/err" || rc=$?
  [[ $rc -eq 1 ]] || fail "$label: exit $rc, want 1"
  grep -q 'Goal.*Hard constraints.*Current design' "$T/err" \
    || fail "$label: no brief error on stderr: $(cat "$T/err")"
  echo "ok: $label -> exit 1, brief error on stderr"
}

# ── arg validation: rejected before herdr is called ──────────────────────────
: > "$T/calls"
reject "no args"
reject "one arg" sude-x
reject "bad name chars" 'bad_name' "do the thing"
reject "empty task" sude-x ""
[[ ! -s $T/calls ]] || fail "rejected args still called herdr: $(cat "$T/calls")"

# ── brief check: tasks missing any of the 3 headers rejected before herdr ────
reject_brief "no headers" "fix the flaky test"
reject_brief "missing constraints" "Goal: x
Current design (open to challenge): y"
reject_brief "missing current design" "Goal: x
Hard constraints: y"
reject_brief "missing goal" "Hard constraints: y
Current design: z"
[[ ! -s $T/calls ]] || fail "non-brief task still called herdr: $(cat "$T/calls")"


# ── happy path: file already written when herdr returns ─────────────────────
echo "done" > "$T/home/quaqua/runs/sude-quick.md"
prompt sude-quick "$brief"
[[ $rc -eq 0 ]] || fail "sude-quick: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/sude-quick.md" ]] \
  || fail "sude-quick: stdout not the runs path: $(cat "$T/out")"
grep -qF "herdr agent prompt sude-quick $brief. When done, write summary (changes, test cmd+result, risks) to ~/quaqua/runs/sude-quick.md; reply only the path. --wait --timeout 1800000" "$T/calls" \
  || fail "sude-quick: wrong herdr call: $(cat "$T/calls")"
grep -qF -- '--wait --timeout 1800000' "$T/calls" || fail "sude-quick: no --wait/--timeout"
echo "ok: sude-quick -> prompt sent verbatim, runs path printed"

# ── early return: herdr exits before the file exists; script keeps waiting ──
prompt sude-slow "Goal: slow task
Hard constraints: none
Current design: poll" FAKE_DELAY=1 PROMPT_WORKER_POLL_SEC=1
[[ $rc -eq 0 ]] || fail "sude-slow: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/sude-slow.md" ]] \
  || fail "sude-slow: stdout not the runs path: $(cat "$T/out")"
grep -q "waiting for $T/home/quaqua/runs/sude-slow.md" "$T/err" \
  || fail "sude-slow: no waiting notice on stderr"
echo "ok: sude-slow -> kept polling after herdr returned, printed runs path"

# ── early return AND herdr nonzero: still waits for the file ────────────────
prompt sude-flaky "Goal: flaky herdr
Hard constraints: none
Current design: retry" FAKE_DELAY=1 FAKE_RC=1 PROMPT_WORKER_POLL_SEC=1
[[ $rc -eq 0 ]] || fail "sude-flaky: exit $rc, want 0: $(cat "$T/err")"
[[ "$(cat "$T/out")" == "$T/home/quaqua/runs/sude-flaky.md" ]] \
  || fail "sude-flaky: stdout not the runs path"
echo "ok: sude-flaky -> nonzero herdr still waited, printed runs path"

# ── cap: no file within PROMPT_WORKER_MAX_SEC -> nonzero, error on stderr ───
prompt sude-never "Goal: never finishes
Hard constraints: none
Current design: cap" PROMPT_WORKER_MAX_SEC=2 PROMPT_WORKER_POLL_SEC=1
[[ $rc -eq 2 ]] || fail "sude-never: exit $rc, want 2 (out: $(cat "$T/out") err: $(cat "$T/err"))"
[[ ! -s $T/out ]] || fail "sude-never: printed a path on timeout: $(cat "$T/out")"
grep -q "error: no $T/home/quaqua/runs/sude-never.md" "$T/err" \
  || fail "sude-never: no timeout error on stderr"
echo "ok: sude-never -> exit 2 after cap, no path printed"

echo "PASS"
