#!/usr/bin/env python3
"""Drive a question UI on the given agents and print status+tail on every status change.

Usage: question-probe.py <seconds> <name:pane> [<name:pane> ...]
Prompts are short; panes are the caller's own throwaway panes.
"""
import json, subprocess, sys, time


def h(*a):
    r = subprocess.run(["herdr", *a], capture_output=True, text=True, timeout=10)
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        return {"_raw": r.stdout}


dur = float(sys.argv[1])
agents = [tuple(a.split(":", 1)) for a in sys.argv[2:]]
for n, _ in agents:
    h("agent", "prompt", n,
      "Use your question/ask-user tool to ask me exactly: Pick A or B? options A and B. "
      "If you have no such tool reply NO_ASK_TOOL.")
t0 = time.time()
seen = set()
while time.time() - t0 < dur:
    for n, p in agents:
        a = (h("agent", "get", n).get("result") or {}).get("agent") or {}
        st = a.get("agent_status")
        txt = subprocess.run(["herdr", "pane", "read", p, "--source", "visible", "--lines", "30"],
                             capture_output=True, text=True, timeout=10).stdout
        if (n, st) not in seen:
            seen.add((n, st))
            print(json.dumps({"t": round(time.time() - t0, 2), "name": n, "status": st,
                              "tail": [l.strip() for l in txt.strip().splitlines()[-6:]]}))
    time.sleep(1)
