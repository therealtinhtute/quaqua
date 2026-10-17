#!/usr/bin/env python3
"""Usage audit — text-only report over OpenCode request logs, meter, local DB and runs.

Sources (live mode):
  * https://opencode.ai/console/api/request-logs?category=inference  (paged, limit<=100)
    token is fetched at runtime from `opencode auth export opencode` and NEVER printed;
  * https://opencode.ai/inference/go/v1/usage  (meter percentages);
  * ~/.local/share/opencode/opencode.db  (read-only, mode=ro) for session->task attribution,
    tool-call classification and idle gaps;
  * <repo>/runs/*.md + git branches for outcomes.

Offline mode (`--fixture-dir DIR`, no network, used by the test): reads
  DIR/request_logs.json  {"pages": [{"items": [...], "nextCursor": "..."}, ...]}
  DIR/meter.json         raw meter response
  DIR/opencode.db        sqlite fixture (built by the test)
  DIR/runs/              run files
Pass --local-version in offline mode (the fixture never shells out).

Days are VN calendar days (UTC+7), matching reports/usage-2026-10-17.md.
"""

import argparse
import collections
import datetime
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
import urllib.parse
import urllib.request

VN_TZ = datetime.timezone(datetime.timedelta(hours=7))
CONSOLE_BASE = "https://opencode.ai/console"
DB_DEFAULT = "~/.local/share/opencode/opencode.db"
REPO_DEFAULT = "~/quaqua"
PAGE_LIMIT = 100  # server rejects limit > 100 with HTTP 400

PROCESS_PATH_RE = re.compile(
    r"(^|/)(runs|rules|memory|raw|reports|docs/plans)(/|$)|(AGENTS|SOUL|USER|identity|models)\.md$|"
    r"team\.toml$|\.brief\.md$|\.plan\.md$|brief"
)
PROCESS_CMD_RE = re.compile(
    r"\bquaqua\s+(worker|pipeline|lead|config|merge|grid|gate|taskforce|advisor|usage)\b|"
    r"\bherdr\s+(agent|pane|tab|workspace)\b|prompt-worker|spawn-worker|cleanup-worker"
)
PRODUCT_PATH_RE = re.compile(
    r"(^|/)(crates|src|scripts|plugins|tests)/|Cargo\.(toml|lock)$|\.rs$|\.py$|\.sh$"
)
PRODUCT_CMD_RE = re.compile(
    r"^\s*(cargo|rustc|pytest|python3?|npm|node|bash\s+tests/|sh\s+tests/|zkill|cargo-nextest)\b|"
    r"(^|\s)cargo\s+(test|build|check|fmt|clippy)"
)

TOKEN_PAT = re.compile(r"(?:st_|rt_)[A-Za-z0-9._-]{10,}")  # a tripwire, not part of normal output


# ---------------------------------------------------------------- sources


def local_version():
    try:
        out = subprocess.run(
            ["opencode", "--version"], capture_output=True, text=True, timeout=15
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    m = re.search(r"(\d+\.\d+\.\d+)", out)
    return m.group(1) if m else None


def get_token():
    """Fetch the access token at runtime. Never print or persist it."""
    out = subprocess.run(
        ["opencode", "auth", "export", "opencode"], capture_output=True, text=True
    ).stdout
    entries = json.loads(out)
    best = None
    for e in entries:
        v = (e or {}).get("value") or {}
        if v.get("access"):
            if e.get("active") or best is None:
                best = v["access"]
    if not best:
        raise RuntimeError("no access token found in `opencode auth export opencode`")
    return best


def http_json(url, token, ua, attempts=4):
    last = "unknown error"
    for i in range(attempts):
        req = urllib.request.Request(
            url, headers={"Authorization": "Bearer " + token, "User-Agent": ua}
        )
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            if e.code < 500:
                raise RuntimeError("HTTP %s for %s" % (e.code, url)) from None
            last = "HTTP %s" % e.code
        except urllib.error.URLError as e:
            last = "network error: %s" % e.reason
        if i < attempts - 1:
            time.sleep(1.0 + i)
    raise RuntimeError("%s for %s" % (last, url))


def fetch_request_logs(token, ua, cutoff_ms, until_ms):
    """Page console request-logs backwards until rows older than cutoff_ms."""
    rows, cursor = [], None
    for _ in range(200):  # safety
        q = {"limit": str(PAGE_LIMIT), "category": "inference", "until": str(until_ms)}
        if cursor:
            q["cursor"] = cursor
        url = CONSOLE_BASE + "/api/request-logs?" + urllib.parse.urlencode(q)
        d = http_json(url, token, ua)
        items = d.get("items") or []
        rows.extend(items)
        cursor = d.get("nextCursor")
        oldest = min((i.get("startedAt") or 0) for i in items) if items else None
        if not items or not cursor or oldest is None or oldest < cutoff_ms:
            break
    seen, out = set(), []
    for r in rows:
        if r.get("id") not in seen:
            seen.add(r.get("id"))
            out.append(r)
    return out


def fetch_meter(token, ua):
    return http_json(CONSOLE_BASE + "/inference/go/v1/usage", token, ua)


def fixture_logs(dirpath):
    d = json.load(open(os.path.join(dirpath, "request_logs.json")))
    rows = []
    for page in d.get("pages") or []:
        rows.extend(page.get("items") or [])
    return rows


def fixture_meter(dirpath):
    return json.load(open(os.path.join(dirpath, "meter.json")))


# ---------------------------------------------------------------- local db


def open_db_ro(path):
    return sqlite3.connect("file:%s?mode=ro" % path, uri=True)


def read_sessions(con):
    sessions = {}
    try:
        cur = con.execute(
            "SELECT id, directory, title, cost, tokens_input, tokens_output,"
            " tokens_reasoning, tokens_cache_read, time_created, time_updated,"
            " idle_outcome FROM session_v2"
        )
    except sqlite3.Error:
        return sessions
    for (
        sid, directory, title, cost, tin, tout, trsn, tcr,
        tc, tu, idle,
    ) in cur:
        sessions[sid] = {
            "directory": directory,
            "title": title,
            "cost": cost or 0.0,
            "tokens": {
                "input": tin or 0,
                "output": tout or 0,
                "reasoning": trsn or 0,
                "cache_read": tcr or 0,
            },
            "time_created": tc,
            "time_updated": tu,
            "idle_outcome": idle,
        }
    return sessions


def read_messages(con, session_ids):
    """Per session: tool calls [(name, input)] and message timestamps."""
    tools = collections.defaultdict(list)
    times = collections.defaultdict(list)
    try:
        cur = con.execute(
            "SELECT session_id, time_created, data FROM session_message"
        )
    except sqlite3.Error:
        return tools, times
    for sid, t, data in cur:
        if session_ids is not None and sid not in session_ids:
            continue
        times[sid].append(t)
        try:
            d = json.loads(data)
        except (TypeError, ValueError):
            continue
        if d.get("type") == "assistant" or "content" in d:
            for c in d.get("content") or []:
                if isinstance(c, dict) and c.get("type") == "tool":
                    inp = ((c.get("state") or {}).get("input")) or c.get("input") or {}
                    tools[sid].append((c.get("name") or "?", inp))
    return tools, times


def classify_tool(name, inp):
    target = ""
    if isinstance(inp, dict):
        for key in ("command", "filePath", "file_path", "path", "pattern", "url"):
            if isinstance(inp.get(key), str):
                target += " " + inp[key]
    target = target.strip()
    if name == "shell":
        if PROCESS_CMD_RE.search(target):
            return "process"
        if PRODUCT_CMD_RE.search(target):
            return "product"
        if PROCESS_PATH_RE.search(target):
            return "process"
        if PRODUCT_PATH_RE.search(target):
            return "product"
        return "other"
    if PROCESS_PATH_RE.search(target):
        return "process"
    if PRODUCT_PATH_RE.search(target):
        return "product"
    if name in ("webfetch", "websearch"):
        return "other"
    return "other"


def task_from_directory(directory):
    if not directory:
        return None
    d = directory.rstrip("/")
    if d.endswith("/quaqua"):
        return "cos"
    base = os.path.basename(d)
    return base or None


# ---------------------------------------------------------------- outcomes


def git_outcome(repo, task):
    """Return list of outcome tokens from git, or [] when repo is unusable."""
    branch = "quaqua/%s" % task
    def run(args):
        return subprocess.run(
            ["git", "-C", repo] + args, capture_output=True, text=True, timeout=30
        )
    r = run(["rev-parse", "--verify", "--quiet", "refs/heads/" + branch])
    if r.returncode != 0:
        return []
    tip = r.stdout.strip()[:7]
    merged = run(["merge-base", "--is-ancestor", "refs/heads/" + branch, "refs/heads/main"])
    if merged.returncode == 0:
        return ["merged main %s" % tip]
    return ["branch %s" % tip]


def runs_outcome(runs_dir, task):
    """Return (tokens, respawned_by, review_rounds) from run files."""
    if not runs_dir or not os.path.isdir(runs_dir):
        return [], None, 0
    tokens = []
    if os.path.exists(os.path.join(runs_dir, task + ".md")):
        tokens.append("report")
    respawned = None
    for cand in (task + "2", task + "3", task + "4"):
        if os.path.exists(os.path.join(runs_dir, cand + ".md")):
            respawned = cand
            break
    rounds = 0
    # review files are named `<kind>-<slug>r<N>.md` (e.g. suhuynh-gridfix3r.md for tieude-gridfix3)
    slugs = {task}
    if "-" in task:
        slugs.add(task.split("-", 1)[1])
    pat = re.compile(r"^.+-(%s)r(\d*)\.md$" % "|".join(re.escape(s) for s in slugs))
    try:
        for fn in os.listdir(runs_dir):
            m = pat.match(fn)
            if m:
                rounds = max(rounds, int(m.group(2) or 1))
    except OSError:
        pass
    return tokens, respawned, rounds


# ---------------------------------------------------------------- analysis


def day_key(ms):
    return datetime.datetime.fromtimestamp(ms / 1000.0, datetime.timezone.utc).astimezone(VN_TZ).strftime("%d/%m")


def fmt_tokens(t):
    return {
        "input": t.get("input", 0),
        "output": t.get("output", 0),
        "reasoning": t.get("reasoning", 0),
        "cache_read": t.get("cache_read", 0),
    }


def blank_tokens():
    return {"input": 0, "output": 0, "reasoning": 0, "cache_read": 0}


def add_tokens(dst, src):
    for k in dst:
        dst[k] += src.get(k, 0)


def build_report(args):
    anchor_ms = args.until
    if anchor_ms is None:
        anchor_ms = int(datetime.datetime.now(tz=datetime.timezone.utc).timestamp() * 1000)
    anchor_dt = datetime.datetime.fromtimestamp(anchor_ms / 1000.0, datetime.timezone.utc).astimezone(VN_TZ)
    cutoff_dt = datetime.datetime(anchor_dt.year, anchor_dt.month, anchor_dt.day, tzinfo=VN_TZ) - datetime.timedelta(days=args.days - 1)
    cutoff_ms = int(cutoff_dt.timestamp() * 1000)

    offline = bool(args.fixture_dir)
    local_ver = args.local_version
    notes, warn = [], False

    if offline:
        raw_rows = fixture_logs(args.fixture_dir)
        meter_raw = fixture_meter(args.fixture_dir)
        db_path = args.db or os.path.join(args.fixture_dir, "opencode.db")
        repo = args.repo or args.fixture_dir
    else:
        token = get_token()
        if local_ver is None:
            local_ver = local_version()
        ua = "opencode/%s" % local_ver if local_ver else "opencode"
        raw_rows = fetch_request_logs(token, ua, cutoff_ms, anchor_ms)
        meter_raw = fetch_meter(token, ua)
        db_path = os.path.expanduser(args.db or DB_DEFAULT)
        repo = os.path.expanduser(args.repo or REPO_DEFAULT)

    rows = [r for r in raw_rows if cutoff_ms <= (r.get("startedAt") or 0) <= anchor_ms]
    runs_dir = args.runs_dir or (os.path.join(repo, "runs") if repo else None)

    sessions = {}
    if db_path and os.path.exists(db_path):
        try:
            con = open_db_ro(db_path)
            sessions = read_sessions(con)
        except sqlite3.Error as e:
            notes.append("local db unreadable: %s" % e)
            con = None
    else:
        con = None
        # fixture/tests always ship a db; live mode without db degrades to session ids
        notes.append("local db not found at %s" % db_path)

    # attribution: request logs -> session -> task
    tasks = collections.OrderedDict()
    day_stats = collections.OrderedDict()
    machine_stats = collections.OrderedDict()

    def task_entry(key, sid=None):
        if key not in tasks:
            tasks[key] = {
                "task": key,
                "sessions": set(),
                "requests": 0,
                "cost": 0.0,
                "tokens": blank_tokens(),
                "first": None,
                "last": None,
            }
        if sid:
            tasks[key]["sessions"].add(sid)
        return tasks[key]

    for r in rows:
        sid = r.get("sessionID")
        sess = sessions.get(sid) if sid else None
        task = task_from_directory(sess["directory"]) if sess else None
        key = task or ("(unknown %s)" % (sid[:12] if sid else "no-session"))
        e = task_entry(key, sid)
        e["requests"] += 1
        e["cost"] += r.get("cost") or 0.0
        add_tokens(
            e["tokens"],
            {
                "input": r.get("inputTokens") or 0,
                "output": r.get("outputTokens") or 0,
                "reasoning": r.get("reasoningTokens") or 0,
                "cache_read": r.get("cacheReadTokens") or 0,
            },
        )
        start = r.get("startedAt") or 0
        end = r.get("lastTokenAt") or r.get("finishedAt") or start
        e["first"] = start if e["first"] is None else min(e["first"], start)
        e["last"] = end if e["last"] is None else max(e["last"], end)

        dk = day_key(start)
        d = day_stats.setdefault(dk, {"date": dk, "requests": 0, "cost": 0.0, "tokens": blank_tokens(), "first": None, "last": None})
        d["requests"] += 1
        d["cost"] += r.get("cost") or 0.0
        add_tokens(
            d["tokens"],
            {
                "input": r.get("inputTokens") or 0,
                "output": r.get("outputTokens") or 0,
                "reasoning": r.get("reasoningTokens") or 0,
                "cache_read": r.get("cacheReadTokens") or 0,
            },
        )
        d["first"] = start if d["first"] is None else min(d["first"], start)
        d["last"] = end if d["last"] is None else max(d["last"], end)

        ua = r.get("userAgent") or "?"
        m = machine_stats.setdefault(ua, {"ua": ua, "requests": 0, "cost": 0.0})
        m["requests"] += 1
        m["cost"] += r.get("cost") or 0.0

    # outcomes / flags per task
    waste = {"respawn": [], "killed": [], "idle_gt_30m": [], "review_rounds": []}
    sessions_used = {sid for t in tasks.values() for sid in t["sessions"] if sid}
    tools, msg_times = ({}, {})
    if con is not None:
        try:
            tools, msg_times = read_messages(con, sessions_used)
        finally:
            con.close()

    for key, e in tasks.items():
        toks = []
        if e["sessions"]:
            idle = [sessions[s].get("idle_outcome") for s in e["sessions"] if s in sessions]
            if any(x in ("failed", "interrupted") for x in idle):
                bad = [x for x in idle if x in ("failed", "interrupted")][0]
                toks.append("killed(%s)" % bad)
                waste["killed"].append({"task": key, "idle_outcome": bad})
        if repo and os.path.isdir(os.path.join(repo, ".git")):
            toks.extend(git_outcome(repo, key))
        r_toks, respawned, rounds = runs_outcome(runs_dir, key)
        toks.extend(r_toks)
        if respawned:
            waste["respawn"].append({"task": key, "replaced_by": respawned})
        if rounds:
            waste["review_rounds"].append({"task": key, "rounds": rounds})
        e["outcome"] = toks or ["?"]
        # idle gaps from message timestamps
        gaps = []
        for sid in e["sessions"]:
            ts = sorted(msg_times.get(sid) or [])
            for a, b in zip(ts, ts[1:]):
                if (b - a) > 30 * 60 * 1000:
                    gaps.append(b - a)
        if gaps:
            idle_min = int(sum(gaps) / 60000)
            waste["idle_gt_30m"].append({"task": key, "idle_min": idle_min})
            e["idle_min"] = idle_min

    # product vs process: tool calls of in-window sessions, $ weighted per session cost (log)
    sess_cost = {}
    for t in tasks.values():
        share = t["cost"] / max(1, len(t["sessions"])) if t["sessions"] else 0.0
        for sid in t["sessions"]:
            sess_cost[sid] = sess_cost.get(sid, 0.0) + share
    split = {c: {"calls": 0, "cost": 0.0} for c in ("product", "process", "other")}
    for sid, calls in tools.items():
        if not calls:
            continue
        counts = collections.Counter(classify_tool(n, i) for n, i in calls)
        total = sum(counts.values()) or 1
        for cls, n in counts.items():
            split[cls]["calls"] += n
            split[cls]["cost"] += sess_cost.get(sid, 0.0) * n / total
    for sid in sessions_used:
        if sid not in tools or not tools[sid]:
            split["other"]["cost"] += sess_cost.get(sid, 0.0)
    split_total_calls = sum(v["calls"] for v in split.values()) or 1
    split_total_cost = sum(v["cost"] for v in split.values()) or 1.0
    split_pct = {
        c: {"callsPct": 100.0 * v["calls"] / split_total_calls,
            "costPct": 100.0 * v["cost"] / split_total_cost,
            "calls": v["calls"], "cost": v["cost"]}
        for c, v in split.items()
    }

    # meter windows: show % plus log $ inside each window when the fetched range covers it
    meter = {}
    usage = (meter_raw or {}).get("usage") or {}
    for win, w in usage.items():
        resets = w.get("resetsAt")
        start_ms = None
        if resets:
            rdt = datetime.datetime.fromisoformat(resets.replace("Z", "+00:00"))
            if win == "rolling":
                start = rdt - datetime.timedelta(hours=5)
            elif win == "weekly":
                start = rdt - datetime.timedelta(days=7)
            elif win == "monthly":
                m = rdt.month - 1 or 12
                y = rdt.year - (1 if rdt.month == 1 else 0)
                start = rdt.replace(year=y, month=m)
            else:
                start = rdt
            start_ms = int(start.timestamp() * 1000)
        entry = {"percent": w.get("percent"), "status": w.get("status"), "resets_at": resets}
        if start_ms is not None:
            entry["window_start"] = datetime.datetime.fromtimestamp(start_ms / 1000.0, datetime.timezone.utc).astimezone(VN_TZ).isoformat()
            lo = max(start_ms, cutoff_ms)
            entry["log_cost"] = round(sum((r.get("cost") or 0.0) for r in rows if lo <= (r.get("startedAt") or 0) <= anchor_ms), 4)
            entry["range_complete"] = start_ms >= cutoff_ms
        meter[win] = entry

    # machines / local warning
    machines = []
    machines_truncated = False
    for ua, m in sorted(machine_stats.items(), key=lambda x: -x[1]["cost"]):
        ver = None
        seg = ua.split("/")
        for s in seg:
            if re.match(r"^\d+\.\d+", s):
                ver = s
                break
        local = bool(local_ver) and ver == local_ver
        machines.append({"ua": ua, "version": ver, "requests": m["requests"], "cost": round(m["cost"], 4), "local": local})
        if not local and local_ver:
            warn = True
        # a UA without a version (e.g. bare 'opencode' or a browser) is only suspicious when rows exist
        if ver is None and local_ver:
            warn = True

    total_cost = sum(t["cost"] for t in tasks.values())
    total_req = sum(t["requests"] for t in tasks.values())
    total_tokens = blank_tokens()
    for t in tasks.values():
        add_tokens(total_tokens, t["tokens"])

    return {
        "generated": datetime.datetime.now(tz=datetime.timezone.utc).astimezone(VN_TZ).isoformat(),
        "offline": offline,
        "days": args.days,
        "until": anchor_dt.isoformat(),
        "cutoff": cutoff_dt.isoformat(),
        "local_version": local_ver,
        "totals": {"requests": total_req, "cost": round(total_cost, 4), "tokens": total_tokens},
        "warn_other_machine": warn,
        "machines": machines,
        "days_table": list(day_stats.values()),
        "tasks": [
            {
                "task": t["task"],
                "sessions": sorted(t["sessions"]),
                "requests": t["requests"],
                "cost": round(t["cost"], 4),
                "tokens": t["tokens"],
                "wall_min": int(((t["last"] or 0) - (t["first"] or 0)) / 60000) if t["first"] else 0,
                "idle_min": t.get("idle_min", 0),
                "outcome": "; ".join(t["outcome"]),
            }
            for t in sorted(tasks.values(), key=lambda x: -x["cost"])
        ],
        "split": split_pct,
        "waste": waste,
        "meter": meter,
        "notes": notes,
    }


# ---------------------------------------------------------------- render


def kfmt(n):
    if n >= 1_000_000:
        return "%.1fM" % (n / 1_000_000.0)
    if n >= 1_000:
        return "%.0fk" % (n / 1_000.0)
    return str(n)


def render_text(rep):
    L = []
    L.append("Usage audit — %s → %s (%d VN days) · generated %s" % (
        rep["cutoff"][:10], rep["until"][:10], rep["days"], rep["generated"][:16]))
    if rep["offline"]:
        L.append("(offline fixture mode)")
    t = rep["totals"]
    L.append("Local opencode: %s · %d requests · $%.4f · in %s / out %s / rsn %s / cacheRead %s" % (
        rep["local_version"] or "?", t["requests"], t["cost"],
        kfmt(t["tokens"]["input"]), kfmt(t["tokens"]["output"]),
        kfmt(t["tokens"]["reasoning"]), kfmt(t["tokens"]["cache_read"])))
    if rep["warn_other_machine"]:
        L.append("[WARN] requests seen with a UA version different from local %s — possibly another machine:" % rep["local_version"])
        for m in rep["machines"]:
            if not m["local"]:
                L.append("       %s: %d req · $%.4f" % (m["ua"], m["requests"], m["cost"]))

    L.append("")
    L.append("== Per day ==")
    L.append("%-6s %6s %9s %8s %8s %8s %10s %7s" % ("day", "req", "$", "in", "out", "rsn", "cacheRead", "wall"))
    for d in rep["days_table"]:
        wall = int(((d["last"] or 0) - (d["first"] or 0)) / 60000)
        L.append("%-6s %6d %9.4f %8s %8s %8s %10s %6dm" % (
            d["date"], d["requests"], d["cost"],
            kfmt(d["tokens"]["input"]), kfmt(d["tokens"]["output"]),
            kfmt(d["tokens"]["reasoning"]), kfmt(d["tokens"]["cache_read"]), wall))

    L.append("")
    L.append("== Per task ==")
    L.append("%-26s %5s %9s %7s %7s %6s %9s %6s %-34s %s" % (
        "task", "req", "$", "in", "out", "rsn", "cacheRead", "wall", "outcome", "flags"))
    for t in rep["tasks"]:
        flags = []
        for w in rep["waste"]["respawn"]:
            if w["task"] == t["task"]:
                flags.append("respawn→%s" % w["replaced_by"])
        for w in rep["waste"]["killed"]:
            if w["task"] == t["task"]:
                flags.append("killed")
        for w in rep["waste"]["idle_gt_30m"]:
            if w["task"] == t["task"]:
                flags.append("idle %dm" % w["idle_min"])
        for w in rep["waste"]["review_rounds"]:
            if w["task"] == t["task"]:
                flags.append("review r%d" % w["rounds"])
        L.append("%-26s %5d %9.4f %7s %7s %6s %9s %5dm %-34s %s" % (
            t["task"][:26], t["requests"], t["cost"],
            kfmt(t["tokens"]["input"]), kfmt(t["tokens"]["output"]),
            kfmt(t["tokens"]["reasoning"]), kfmt(t["tokens"]["cache_read"]),
            t["wall_min"], t["outcome"][:34], ",".join(flags)))

    s = rep["split"]
    L.append("")
    L.append("== Product vs process (tool calls; $ weighted per session) ==")
    for cls in ("product", "process", "other"):
        L.append("%-8s %5.0f%% calls · %5.0f%% $  (%d calls · $%.4f)" % (
            cls, s[cls]["callsPct"], s[cls]["costPct"], s[cls]["calls"], s[cls]["cost"]))

    L.append("")
    L.append("== Waste flags ==")
    for w in rep["waste"]["respawn"]:
        L.append("respawn     %s → %s" % (w["task"], w["replaced_by"]))
    for w in rep["waste"]["killed"]:
        L.append("killed      %s (session idle outcome: %s)" % (w["task"], w["idle_outcome"]))
    for w in rep["waste"]["idle_gt_30m"]:
        L.append("idle>30m    %s (%dm)" % (w["task"], w["idle_min"]))
    for w in rep["waste"]["review_rounds"]:
        L.append("review      %s (%d round(s))" % (w["task"], w["rounds"]))
    if not any(rep["waste"].values()):
        L.append("none")

    L.append("")
    L.append("== Meter (Go, /inference/go/v1/usage) ==")
    for win in ("rolling", "weekly", "monthly"):
        m = rep["meter"].get(win)
        if not m:
            continue
        extra = ""
        if "log_cost" in m:
            extra = " · log $ in window: $%.4f%s" % (m["log_cost"], "" if m["range_complete"] else " (range incomplete)")
        L.append("%-8s %s%% · resets %s%s" % (win, m["percent"], (m["resets_at"] or "?")[:16], extra))

    L.append("")
    L.append("== Machines (UA) ==")
    for m in rep["machines"]:
        L.append("%-34s %5d req · $%.4f%s" % (
            m["ua"], m["requests"], m["cost"], "  (local)" if m["local"] else "  [≠ local %s]" % (rep["local_version"] or "?")))

    if rep["notes"]:
        L.append("")
        L.append("== Notes ==")
        for n in rep["notes"]:
            L.append("- " + n)

    out = "\n".join(L)
    # tripwire: this script must never emit credentials
    if TOKEN_PAT.search(out):
        raise RuntimeError("refusing to print output that looks like a token")
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description="OpenCode usage audit (text-only)")
    ap.add_argument("--days", type=int, default=7, help="number of VN calendar days (default 7)")
    ap.add_argument("--json", action="store_true", help="emit JSON instead of text")
    ap.add_argument("--until", type=int, default=None, help="anchor unix ms (default: now)")
    ap.add_argument("--fixture-dir", default=None, help="offline fixture dir (no network)")
    ap.add_argument("--local-version", default=None, help="override local opencode version (offline)")
    ap.add_argument("--db", default=None, help="path to opencode.db (default ~/.local/share/opencode/opencode.db)")
    ap.add_argument("--repo", default=None, help="repo for git outcomes (default ~/quaqua)")
    ap.add_argument("--runs-dir", default=None, help="runs dir (default <repo>/runs)")
    args = ap.parse_args(argv)
    rep = build_report(args)
    if args.json:
        print(json.dumps(rep, indent=1, default=str))
    else:
        print(render_text(rep))
    return 0


if __name__ == "__main__":
    sys.exit(main())
