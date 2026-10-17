#!/usr/bin/env python3
"""Offline test for scripts/audit/usage.py — no network, fixture only.

Run:  python3 scripts/audit/tests/test_usage.py
"""

import datetime
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(os.path.dirname(HERE), "usage.py")
FIXTURES = os.path.join(HERE, "fixtures")
VN = datetime.timezone(datetime.timedelta(hours=7))
UNTIL_MS = 1792148400000  # 2026-10-16T18:00:00+07:00


def vn_ms(s):
    return int(datetime.datetime.fromisoformat(s).replace(tzinfo=VN).timestamp() * 1000)


def build_db(path):
    con = sqlite3.connect(path)
    con.execute(
        "CREATE TABLE session_v2 (id TEXT PRIMARY KEY, directory TEXT, title TEXT,"
        " cost REAL, tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER,"
        " tokens_cache_read INTEGER, time_created INTEGER, time_updated INTEGER, idle_outcome TEXT)"
    )
    con.execute(
        "CREATE TABLE session_message (id TEXT, session_id TEXT, type TEXT, seq INTEGER,"
        " time_created INTEGER, time_updated INTEGER, data TEXT)"
    )
    con.executemany(
        "INSERT INTO session_v2 VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        [
            ("ses_a", "proj/.wt/tieude-alpha", "alpha work", 0.5, 1, 1, 1, 1, 0, 0, "succeeded"),
            ("ses_b", "proj/.wt/tieude-beta", "beta work", 0.2, 1, 1, 1, 1, 0, 0, "failed"),
            ("ses_c", "proj/.wt/tieude-gamma", "no traffic", 0.0, 0, 0, 0, 0, 0, 0, None),
        ],
    )
    msgs = [
        (vn_ms("2026-10-16T09:00:00"), "ses_a",
         [{"type": "tool", "name": "shell", "state": {"input": {"command": "cargo test --locked -p quaqua"}}}]),
        (vn_ms("2026-10-16T09:05:00"), "ses_a",
         [{"type": "tool", "name": "read", "state": {"input": {"filePath": "crates/quaqua/src/lib.rs"}}}]),
        (vn_ms("2026-10-16T09:45:00"), "ses_a",
         [{"type": "tool", "name": "edit", "state": {"input": {"filePath": "~/quaqua/runs/tieude-alpha.md"}}}]),
        (vn_ms("2026-10-15T17:00:00"), "ses_b",
         [{"type": "tool", "name": "shell", "state": {"input": {"command": "git status"}}}]),
        (vn_ms("2026-10-15T17:10:00"), "ses_b",
         [{"type": "tool", "name": "read", "state": {"input": {"filePath": "rules/_all/brief.md"}}}]),
    ]
    con.executemany(
        "INSERT INTO session_message VALUES (?,?,?,?,?,?,?)",
        [
            ("m%d" % i, sid, "assistant", i, t, t, json.dumps({"time": {"created": t}, "content": content}))
            for i, (t, sid, content) in enumerate(msgs)
        ],
    )
    con.commit()
    con.close()


def make_fixture_dir(tmp):
    for fn in ("request_logs.json", "meter.json"):
        shutil.copy(os.path.join(FIXTURES, fn), os.path.join(tmp, fn))
    shutil.copytree(os.path.join(FIXTURES, "runs"), os.path.join(tmp, "runs"))
    build_db(os.path.join(tmp, "opencode.db"))
    return tmp


def run_script(*extra):
    cmd = [sys.executable, SCRIPT, "--fixture-dir", FIXTURE_DIR,
           "--local-version", "2.0.32", "--until", str(UNTIL_MS)] + list(extra)
    return subprocess.run(cmd, capture_output=True, text=True, timeout=60)


FIXTURE_DIR = None


class UsageAuditTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        global FIXTURE_DIR
        tmp = tempfile.mkdtemp(prefix="usage-audit-test-")
        FIXTURE_DIR = make_fixture_dir(tmp)
        cls.tmp = tmp

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def test_text_report(self):
        r = run_script("--days", "3")
        self.assertEqual(r.returncode, 0, r.stderr)
        out = r.stdout
        # totals and paging: all 5 rows across 2 pages, $0.40
        self.assertIn("5 requests", out)
        self.assertIn("$0.4000", out)
        self.assertIn("14/10", out)
        self.assertIn("16/10", out)
        # warn: foreign UA version
        self.assertIn("[WARN]", out)
        self.assertIn("9.9.9", out)
        # tasks
        self.assertIn("tieude-alpha", out)
        self.assertIn("tieude-beta", out)
        # waste flags
        self.assertIn("respawn     tieude-beta → tieude-beta2", out)
        self.assertIn("killed      tieude-beta", out)
        self.assertIn("idle>30m    tieude-alpha (40m)", out)
        self.assertIn("review      tieude-alpha (1 round(s))", out)
        # product/process split: 2 product / 2 process / 1 other calls; $ .22/.145/.035
        self.assertIn("40% calls", out)
        self.assertIn("$0.2200", out)
        # meter: rolling 7%, log $ in its window = r5
        self.assertIn("rolling  7%", out)
        self.assertIn("log $ in window: $0.0300", out)
        # never a token
        self.assertNotRegex(out, r"st_|rt_")

    def test_json_report(self):
        r = run_script("--days", "3", "--json")
        self.assertEqual(r.returncode, 0, r.stderr)
        rep = json.loads(r.stdout)
        self.assertEqual(rep["totals"]["requests"], 5)
        self.assertAlmostEqual(rep["totals"]["cost"], 0.40, places=4)
        self.assertTrue(rep["warn_other_machine"])
        self.assertEqual(rep["local_version"], "2.0.32")
        tasks = {t["task"]: t for t in rep["tasks"]}
        self.assertEqual(set(tasks), {"tieude-alpha", "tieude-beta"})
        self.assertAlmostEqual(tasks["tieude-alpha"]["cost"], 0.33, places=4)
        self.assertAlmostEqual(tasks["tieude-beta"]["cost"], 0.07, places=4)
        self.assertEqual(tasks["tieude-beta"]["outcome"], "killed(failed); report")
        self.assertAlmostEqual(rep["split"]["product"]["cost"], 0.22, places=4)
        self.assertAlmostEqual(rep["split"]["product"]["callsPct"], 40.0, places=1)
        self.assertAlmostEqual(rep["split"]["process"]["cost"], 0.145, places=4)
        self.assertAlmostEqual(rep["meter"]["rolling"]["percent"], 7)
        self.assertAlmostEqual(rep["meter"]["rolling"]["log_cost"], 0.03, places=4)
        self.assertNotRegex(r.stdout, r"st_|rt_")

    def test_days_window(self):
        r = run_script("--days", "1")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("2 requests", r.stdout)
        self.assertIn("· $0.0500 ·", r.stdout)

    def test_json_matches_text_totals(self):
        text = run_script("--days", "3").stdout
        rep = json.loads(run_script("--days", "3", "--json").stdout)
        self.assertIn("$%.4f" % rep["totals"]["cost"], text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
