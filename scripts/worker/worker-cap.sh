#!/usr/bin/env bash
# Usage: worker-cap.sh <owner> [<exclude-worker>]
#   Print the number of active worker slots used by <owner> (cos or <slug>-lead).
#   Each live task worker in herdr counts as 1.
#   Each active pipeline (finished=0 and alive) reserves 2 slots (builder + reviewer).
#   Project Leads (<slug>-lead) do not count toward the cap.
#   Max allowed slots per owner is 4.
set -euo pipefail

owner=${1:-cos}
exclude_worker=${2:-}
runs_dir="${QUAQUA_RUNS_DIR:-$HOME/quaqua/runs}"
. "$(dirname "$0")/../lib/common.sh"

count_owner_slots "$runs_dir" "$owner" "$exclude_worker"
