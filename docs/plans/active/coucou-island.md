# Plan — Coucou-like Media Island (GNOME 46 extension)

Input: `spec-raw/coucou-island-lock.md` (rev 2). Review: `runs/quansu-island.md`.
Repo: `~/Lab/coucou-island` (new; created in M0).
Builder/reviewer pairing: tieude builds → quansu reviews (different harness).

## M0 — Scaffold (tieude)
1. Repo + `metadata.json` (UUID, `shell-version: ["46"]`, `session-modes: ["user"]`), GSettings schema (`auto-peek`, `show-island`, `toggle-shortcut`), empty `extension.js` with reversible enable/disable owner.
2. `src/{adapter,domain,island,view}/` skeleton; headless test runner (gjs or node on pure reducers).
3. `make pack|install|enable|uninstall` wrapping `gnome-extensions`.
→ verify: `gnome-extensions pack` succeeds; reducer test suite runs (0 tests ok); install/uninstall leaves no files.

## M1 — PoC: compact pill + play/pause (tieude)
1. MPRIS fixture: controllable session-bus fake player (Python/GJS) for tests.
2. Adapter: own Player XML (incl. Position/Rate/Seeked/Can*), subscribe-before-snapshot discovery, generation tokens, normalized snapshots.
3. Domain reducer: registry + selection (Playing > Paused, transition-ranked, Stopped ineligible, owner-loss fallback).
4. View: bounded chrome pill under top bar on primary; compact (placeholder art, title, play/pause icon); hidden when no eligible player.
5. Lifecycle: full teardown + rollback; error model per lock.
→ verify (gate): headless adapter+reducer tests vs fixture green; anh iu on Wayland: pill shows with real player, play/pause works, no player → hidden, enable/disable ×10 no leak/stuck grab (`journalctl --user -g <uuid>` clean).

## M2 — Island state machine + expanded (tieude)
1. Island reducer: hidden/compact/peek/expanded, precedence table, cancellable timers, paused 5-min auto-hide.
2. Expanded card: metadata, prev/play/next gated by Can*, modal grab with Esc/outside-click, guaranteed popModal.
3. Auto-peek on track change (~3s, respects `auto-peek`, no focus steal); hover >300ms peek; `show-island` toggle.
4. Animations: `ease()` 200–250ms, honors `enable-animations`.
→ verify: reducer transition tests (all precedence cases); manual: expand/collapse/peek interrupt cases, reduced-motion on.

## M3 — Progress + artwork (tieude)
1. Position anchor + Rate extrapolation, re-anchor on Seeked/status/track/rate; timer only while expanded; hide when duration unknown.
2. Artwork loader: Soup 3 streamed + byte cap + deadline + cancellation; async file://; cache with eviction; placeholder fallback.
→ verify: fixture tests for seek/rate/track change; artwork tests (oversize, timeout, 404, offline → controls still work).

## M4 — Display + a11y hardening (tieude)
1. `monitors-changed` replacement, no-primary hide, fullscreen hide, scaling 100/125/150/200 + mixed.
2. A11y: focusable buttons, dynamic accessible names, toggle shortcut, focus/contrast styles, high-contrast + large-text.
→ verify: anh iu runs multi-monitor/scaling checklist; a11y checklist.

## M5 — v1 gate (quansu review → check)
1. DoD matrix: all 10 items with pass/fail evidence.
2. Install/uninstall/update docs; residual-risk section in README.
→ verify: `check` gate; quansu review of full diff; DoD matrix all pass.

## Open
- Spec `# Goal` section truncated — confirm nothing missing.

## V2 — beyond media (anh iu picked all 6, 2026-10-02; research `runs/sude-ideas.md`)
- M6 Activity model: pure reducer deciding which activity owns compact (agent > recording > timer > media), transient peek queue (notif/HUD/battery); bounded-adapter contract per source. + Timer/Pomodoro + privacy/recording indicators.
- M7 System peeks: volume/brightness HUD (observe only; OSD suppression setting default off), battery peeks (UPower), notification mirror peek (no banner suppression).
- M8 AI agent: read-only status via `herdr api snapshot` → then Claude Code hook socket + Allow/Deny/Always (hook settings backup+diff, never auto-install without owner OK).
→ verify each: pure tests + headless dbus-run-session run; never live session.
