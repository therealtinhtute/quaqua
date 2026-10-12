# Self-learning cho Quả Quả — SUPERSEDED (2026-10-08)

Superseded by **telemetry** (lock `spec-raw/quaqua-slp-slim-lock.md` §2): each pipeline appends one line to `runs/telemetry.jsonl` (`ts, task, repo, builder, reviewer, review, rounds, review_changed_outcome, status, minutes`); after ~10 pipelines CoS reviews it — a reviewer that rarely changes outcomes defaults off.

History: the self-built wiki + phase-1 code (wiki.py, worker-outcome.sh) was removed 2026-10-05; the Jev-judge lesson-mining plan below never resumed. Do not restart it without anh iu.

## Original plan (paused, kept for reference)
Goal: harness học từ outcome + lời sửa của anh iu; Jev làm giám khảo, Claude rút bài học, anh iu duyệt luật hành vi.
- Phase 1: `scripts/worker-outcome.sh` ledger + `wiki.py agreement` (Claude vs Jev shadow) + Jev `target` question.
- Phase 2: `wiki.py learn <session.jsonl|runs md>` → candidate lessons → Jev scores target+confidence → wiki auto ≥0.8 khi agreement ≥85%, còn lại vào queue `/wiki review`.
- Phase 3: SessionEnd hook tự chạy; routing alert khi model fail ≥2 lần; spawn-worker gắn top claims.
Gate (still true): luật hành vi (`memory/rules.md`, `rules/`, `models.md`) luôn qua anh iu.
