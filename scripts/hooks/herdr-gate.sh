#!/usr/bin/env bash
# PreToolUse gate (rules/_all/fanout-agent-grid.md). Binary resolution:
# $QUAQUA_BIN, else `quaqua` on PATH; the binary owns the real tokenizer +
# policy (`quaqua gate`).
#
# The fence must not vanish when the binary is missing: Claude Code treats every
# exit code other than 2 as a non-blocking error, so a bare `quaqua gate` that
# dies 127 would let every pane/workspace spawn+move through (review r1 finding,
# runs/suhuynh-r6r1.md). It must also not lock an agent out of the shell before
# it can build the binary, so the fallback blocks only the guarded commands and
# lets everything else through with a warning.
#
# The fallback is deliberately cheap (no tokenizer): it scans the raw hook JSON
# on stdin for `herdr … <guarded pair>`, the same six verb pairs
# crates/quaqua/src/gate/policy.rs gates. That is a conservative superset — it
# also blocks argument text such as `echo "herdr pane split"` and the `--help`
# forms — which errs toward blocking while the binary is unavailable.
# `h\e\rdr`-style escaping is the one known miss; install the binary for exact
# semantics.
set -uo pipefail

bin=${QUAQUA_BIN:-quaqua}
if command -v "$bin" >/dev/null 2>&1; then
  exec "$bin" gate "$@"
fi

# pane split | pane move | tab create | workspace create | workspace close |
# agent start — kept in step with policy.rs `guarded_pair`.
guarded='(pane[[:space:]]+(split|move)|tab[[:space:]]+create|workspace[[:space:]]+(create|close)|agent[[:space:]]+start)'

stdin=$(cat)
if grep -Eq "herdr.*$guarded" <<<"$stdin"; then
  echo "quaqua gate: '$bin' not found — fallback match; blocking pane/workspace spawn+move (fail-closed)" >&2
  echo "blocked: spawn/move panes and workspaces only via the quaqua binary — install quaqua or set QUAQUA_BIN, then retry — rules/_all/fanout-agent-grid.md" >&2
  exit 2
fi

echo "quaqua gate: '$bin' not found — install quaqua or set QUAQUA_BIN; only pane/workspace spawn+move is blocked meanwhile" >&2
exit 0
