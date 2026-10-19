# evtprobe — herdr 0.9.5 events reliability probe (2026-10-17)

Nguồn: branch `quaqua/tieude-evtprobe` (commit `8aca836` add probe + `99e1b34` fix analyzer) — publish vào main ngày 18/10/2026 để dùng được cross-machine (raw/ + runs/ là local-only, git-ignored).

- `probe.py` — subscriber `events.subscribe` (pane-pinned) song song poll cross-check + `ps` snapshots → log `.jsonl`. Read-only.
- `analyze.py` — analyzer (đã fix pane lookup + đếm transition chỉ theo status).
- `REPORT-2026-10-17.md` — report đầy đủ: method, 8/8 transitions delivered (omp/opencode/claude), latency bounds ≤1.6s, coverage contract (pin pane set + state-read lúc connect + reconnect khi topology đổi), cảnh báo idle cuối turn, verdicts + lessons.

Chạy: xem header `probe.py` (env `EVT_DURATION`, `EVT_POLL_LIST`, …); cần herdr 0.9.5 + socket `$HERDR_SOCKET_PATH`.

Liên quan: issue #47 (T4 — event-driven monitor brainstorm).
