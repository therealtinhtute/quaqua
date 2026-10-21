#!/bin/bash
# scenario.sh <outdir> <mode: silent|question>
# Env: PANES = space-separated pane=name=kind[=session] triples; DUR = monitor seconds.
# Starts probe.py for the window, prompts the three agents with one short task, and
# leaves the panes for the operator to close (the probe never closes a foreign pane).
set -u
out="$1"; mode="$2"; mkdir -p "$out"
rm -f "$out/events.jsonl" "$out/polls.jsonl"
DUR=${DUR:-70}
# shellcheck disable=SC2086
python3 "$(dirname "$0")/probe.py" "$out" "$DUR" ${PANES:?set PANES} > "$out/monitor.log" 2>&1 &
mon=$!
sleep 3
date +%s%3N > "$out/prompt_at"
if [ "$mode" = silent ]; then
  herdr agent prompt wd-cc "Run this shell command with your Bash tool and wait for it to finish: sleep 40. Then reply with exactly DONE_SLEEP. Do not do anything else." >/dev/null 2>&1 &
  herdr agent prompt wd-omp "Run this shell command with your bash tool and wait for it to finish: sleep 40. Then reply with exactly DONE_SLEEP. Do not do anything else." >/dev/null 2>&1 &
  herdr agent prompt wd-oc2 "Run this shell command with your bash tool and wait for it to finish: sleep 40. Then reply with exactly DONE_SLEEP. Do not do anything else." >/dev/null 2>&1 &
else
  herdr agent prompt wd-cc "Use the AskUserQuestion tool right now to ask me exactly: Pick A or B? with options A and B. Then stop and wait for my answer." >/dev/null 2>&1 &
  herdr agent prompt wd-omp "If you have a tool to ask the user a question, call it right now asking exactly: Pick A or B? with options A and B. Then stop and wait for my answer. If you have no such tool, reply NO_ASK_TOOL." >/dev/null 2>&1 &
  herdr agent prompt wd-oc2 "Use the question tool right now to ask me exactly: Pick A or B? with options A and B. Then stop and wait for my answer." >/dev/null 2>&1 &
fi
wait $mon
date +%s%3N > "$out/end_at"
