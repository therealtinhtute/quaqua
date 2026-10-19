# Team event-driven monitor — quaqua watch + status (central log/metrics cho CoS & Lead)

Status: **draft brainstorm** (chưa giao worker) · CoS 2026-10-17 · gom từ phiên CoS `985f4d82` + bằng chứng `runs/tieude-evtprobe.md`

## Vấn đề (retro audit, có số)
CoS/Lead **không có bảng trạng thái chung** cho team → anh iu luôn là người phát hiện trước:
- Prompt #26 "có ai chạy đâu nhỉ" · #29 "nó có chạy đâu" · #41/#43 "nó chưa chạy kìa" · #118 "đâu còn cái nào chạy đâu" · #136 "team intake đã xong chưa, sao chưa dọn" (phiên CoS `985f4d82`).
- Phiên đó: 137 prompt, 509 tool call, 11 compact, 21 tool lỗi. Trạng thái (`done-not-cleaned`, `idle-unprompted`, `orphan`) đều **tính được** từ `herdr agent list` + `runs/*.md` + `runs/*.state` — nhưng không ai nhìn.
- Watchdog/pipeline vẫn **poll 30s** (`poll 30s, stall 600s, idle 120s`); exit 3 giả (agent done without run file, false idle) là churn lớn nhất 3 ngày qua (13 watchdog exit, 21 dòng `blocked` — `harness-effort-cost-audit.md` §2.3).

## Nền tảng đã có (không cần service mới)
- herdr 0.9.5 đã là **event bus**: `events.subscribe` / `events.wait`; events `pane.agent_status_changed`, `pane.agent_detected`, `pane.exited`, `pane.closed`, `tab.created/closed`, `pane.output_matched`; CLI `herdr agent wait --until idle|done|blocked` (block theo event).
- Cả 4 harness tự báo trạng thái qua integration plugin (claude v11, omp v10, opencode v13, pi v9 — `herdr integration status`).
- Bằng chứng độ tin cậy (`raw/2026-10-17-tieude-evtprobe.md`): **8/8 transition delivered, 0 mất, 0 sai, 0 trùng** (gồm omp + opencode); latency ≤1.6s, thường ≤500ms; event luôn đến trước poll kế tiếp.
- **Contract bắt buộc khi làm subscriber**: subscription là per-pane (không wildcard), không có snapshot lúc connect, 1 subscribe/connection → phải (a) pin pane set, (b) đọc state 1 lần lúc connect, (c) reconnect + re-sync khi topology đổi (`pane.agent_detected`).
- **Cảnh báo**: `idle` cuối turn với background shell là idle thật theo plugin (`session.idle→idle`) → watcher **không bao giờ** coi status event là bằng chứng hoàn thành. **Report file (`runs/<name>.md`) vẫn là nguồn chính.**

## Thiết kế đề xuất (từ thought #8171)
1. **`quaqua watch <names…>`** — chặn trên `events.wait`: khi `agent_status_changed → idle|done|blocked` hoặc `pane.exited`, kiểm tra `runs/<name>.md` rồi in **1 dòng** và thoát. CoS/Lead chạy nền → bị đánh thức đúng lúc thay vì sleep-poll. Có `--timeout`, tự reconnect khi socket rớt (pre-mortem).
2. **`quaqua status`** — bảng tổng: mỗi dòng = 1 Lead/worker: trạng thái agent · report có/chưa · phase/prompted trong pipeline · thời gian idle · cột cảnh báo (`DONE-NOT-CLEANED`, `IDLE-UNPROMPTED`, `ORPHAN`). Nguồn: `herdr api snapshot` + `runs/*`. In text-only (không dashboard GUI; popup herdr theo `docs/research/herdr-ui-ux-notes.md` là optional sau).
3. **Central log/metrics** (mục tiêu "log metric chung"): nối vào `runs/telemetry.jsonl` (pipeline đã ghi `{ts,task,builder,reviewer,rounds,status,minutes}`) + cost/token lấy từ opencode API local (`/api/session/<id>` — có `cost`, `tokens`, `variant`) cho opencode; omp/claude để sau (W2 đang treo vì lý do này). Cùng một writer, một file, CoS + Lead cùng đọc.

Rejected: daemon/Kafka/blackboard service thường trực — herdr server đã giữ vai đó; thêm tiến trình = thêm thứ phải sống/dọn.

## 3 phase đề xuất
- **P1 — `quaqua watch`** (subcommand nhỏ, Rust): dựa hợp đồng evtprobe (pin + resync + report-file-ưu-tiên). Test: fixture herdr shim + 1 live smoke với 1 worker thật.
- **P2 — `quaqua status`**: bảng + cột cảnh báo; thêm 1 dòng Protocol vào AGENTS.md: "CoS chạy `quaqua status` đầu mỗi lượt và sau mỗi lần được đánh thức".
- **P3 — metrics chung**: gộp `telemetry.jsonl` + cost per session (opencode trước); dashboard popup herdr = optional.
- Điều kiện tiên quyết: `sude-guard` (W7 kind routing) merge + binary mới — vì transfer Claude-out đang dùng cùng binary.

## Decisions (anh iu, 2026-10-19 · issue #47)
- **P1 `watch` + P2 `status` song song**: 2 worker, mỗi worker một file area riêng (`status` / `watch`); CoS điều phối trực tiếp, không cần Lead (<3 worker).
- **P2 thêm cờ `WAITER-LOST`**: orchestrator đang idle trong khi worker của nó còn chạy hoặc đã nộp `runs/<name>.md` mà chưa ai đọc. Ca thật 2026-10-19: lệnh chờ nền của mu-lead chết theo session, worker P4 vẫn chạy, không ai biết cho tới khi CoS tự kiểm tra. `watch` không chữa được ca này vì nó cũng chết theo session.
- **Trigger**: CoS chạy `quaqua status` đầu mỗi lượt (thêm 1 dòng vào Protocol trong AGENTS.md) + Auditor cron báo cờ đỏ về CoS khi không có session nào mở.
- **P3 metrics bị tách khỏi #47**, chuyển sang #48 (cost đa harness).
- **Text-only.** Dashboard popup herdr để lại làm improve sau (xem `docs/research/herdr-ui-ux-notes.md`), không nằm trong scope #47.
