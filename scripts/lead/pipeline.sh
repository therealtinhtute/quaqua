#!/usr/bin/env bash
# Usage: pipeline.sh <task> <repo> --brief <file>
# Reviewer contract: ends with 'Verdict: clean' (exit 0) or 'Verdict: <N> findings'
export QUAQUA_SCRIPT_DIR="${QUAQUA_SCRIPT_DIR:-$(cd "$(dirname "$0")" && pwd)}"
exec "${QUAQUA_BIN:-quaqua}" pipeline "$@"
