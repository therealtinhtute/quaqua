#!/usr/bin/env bash
export QUAQUA_SCRIPT_DIR="${QUAQUA_SCRIPT_DIR:-$(cd "$(dirname "$0")" && pwd)}"
exec "${QUAQUA_BIN:-quaqua}" worker spawn "$@"
