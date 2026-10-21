#!/usr/bin/env python3
"""herdr 0.9.5 watchdog-signal probe: pane events, agent status, session-store mtime.

Reads only; the driver (run.sh) prompts its own throwaway panes and calls this
monitor for a fixed window. Writes NDJSON (events, polls) into --out.
"""
import json, os, socket, subprocess, sys, threading, time

def now_ms(): return int(time.time() * 1000)

def herdr(*args, timeout=10):
    try:
        r = subprocess.run(["herdr", *args], capture_output=True, text=True, timeout=timeout)
        return json.loads(r.stdout) if r.stdout.strip() else {"_error": r.stderr.strip()}
    except Exception as e:
        return {"_error": str(e)}

class Log:
    def __init__(self, path):
        self.f = open(path, "a", buffering=1); self.lock = threading.Lock()
    def w(self, o):
        with self.lock: self.f.write(json.dumps(o, separators=(",", ":")) + "\n")

def store_mtime(kind, session):
    paths = []
    if kind == "claude" and session:
        for d in os.listdir(os.path.expanduser("~/.claude/projects")):
            p = os.path.expanduser(f"~/.claude/projects/{d}/{session}.jsonl")
            if os.path.exists(p): paths.append(p)
    elif kind == "omp" and session:
        paths.append(session)
    elif kind == "opencode":
        # last write of this session's rows (per-session, not the shared WAL mtime)
        try:
            out = subprocess.run(["sqlite3", "-readonly", os.path.expanduser("~/.local/share/opencode/opencode.db"),
                f"select max(time_updated) from session_message where session_id='{session}'"],
                capture_output=True, text=True, timeout=5).stdout.strip()
            return int(out) if out.isdigit() else None
        except Exception:
            return None
    return max((os.stat(p).st_mtime_ns // 1_000_000 for p in paths if os.path.exists(p)), default=None)

def main():
    out = sys.argv[1]; dur = float(sys.argv[2])
    # --pane id=name=kind[=session]
    targets = []
    for a in sys.argv[3:]:
        parts = a.split("=", 3)
        targets.append({"pane": parts[0], "name": parts[1], "kind": parts[2],
                        "session": parts[3] if len(parts) > 3 else ""})
    os.makedirs(out, exist_ok=True)
    ev = Log(os.path.join(out, "events.jsonl")); polls = Log(os.path.join(out, "polls.jsonl"))
    meta = {"t0": now_ms(), "dur": dur, "herdr": subprocess.run(["herdr","--version"],capture_output=True,text=True).stdout.strip(),
            "socket": os.environ.get("HERDR_SOCKET_PATH"), "targets": targets}
    json.dump(meta, open(os.path.join(out, "meta.json"), "w"), indent=1)

    subs = []
    for t in targets:
        subs.append({"type": "pane.agent_status_changed", "pane_id": t["pane"]})
        subs.append({"type": "pane.exited", "pane_id": t["pane"]})
    stop = threading.Event()

    def reader():
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(meta["socket"]); s.settimeout(1)
        s.sendall((json.dumps({"id":"wdprobe","method":"events.subscribe","params":{"subscriptions":subs}})+"\n").encode())
        buf = b""
        while not stop.is_set():
            try:
                chunk = s.recv(65536)
                if not chunk: ev.w({"_eof": True, "t": now_ms()}); return
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    if line.strip():
                        try: o = json.loads(line)
                        except Exception: o = {"raw": line.decode(errors="replace")}
                        o["_recv"] = now_ms(); ev.w(o)
            except socket.timeout: continue
            except Exception as e: ev.w({"_error": str(e), "t": now_ms()}); return
    th = threading.Thread(target=reader, daemon=True); th.start()
    time.sleep(0.4)
    t_end = time.time() + dur
    while time.time() < t_end:
        sample = {"t": now_ms(), "agents": []}
        for t in targets:
            g = herdr("agent", "get", t["name"])
            a = (g.get("result") or {}).get("agent") or {}
            st = a.get("agent_session") or {}
            sess = st.get("value") or t["session"]
            if sess: t["session"] = sess
            raw = subprocess.run(["herdr","pane","read",t["pane"],"--source","visible","--lines","80"],capture_output=True,text=True,timeout=10)
            text = raw.stdout or ""
            sample["agents"].append({
                "name": t["name"], "kind": t["kind"], "status": a.get("agent_status"),
                "revision": a.get("revision"), "seq": a.get("state_change_seq"),
                "session": sess, "store_ms": store_mtime(t["kind"], sess),
                "read_rev": None, "text_len": len(text),
                "text_sha": __import__("hashlib").sha256(text.encode()).hexdigest()[:12],
                "tail": [l.strip() for l in text.strip().splitlines()[-3:]],
            })
        polls.w(sample)
        time.sleep(1.0)
    stop.set()
    try: th.join(timeout=2)
    except Exception: pass

if __name__ == "__main__":
    main()
