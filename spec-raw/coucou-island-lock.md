# Coucou-like Media Island — Brainstorm Lock (2026-10-02, rev 2 after quansu review)

Source spec: `spec-raw/Prompt — Plan Coucou-like Media Island for Ubuntu GNOME.md` (Goal section truncated at `...`).
Review: `runs/quansu-island.md` (2 blocker, 10 major, 3 minor — all addressed below).
Target: Ubuntu 26.04, GNOME Shell 50.1 (Mutter 50.1, GJS 1.88, Soup 3.6), Wayland.

## Architecture
- GNOME Shell extension only (GJS ESM, St/Clutter). No daemon, no GTK app, no .deb/Flatpak.
- Reason: Mutter 50 has no wlr-layer-shell; a standalone xdg-toplevel cannot self-position top-center or stay on top.
- Support: GNOME 50 only, `shell-version: ["50"]`. Add later majors only after their compatibility gate passes.
- No GTK import in `extension.js`; do not depend on Shell internal UI classes (e.g. `MprisPlayer`/`MessageList.Source`) — use them as reference only.

## Module boundaries (spec §Architecture)
1. **MPRIS adapter** — Gio D-Bus; emits normalized player snapshots/events. Owns proxies, subscriptions, cancellables.
2. **Domain reducer** (pure) — player registry + active-player selection.
3. **Island reducer** (pure) — UI state machine + transitions.
4. **Shell view / lifecycle owner** — St actors, chrome, input, timers, keybinding, artwork loader.
St/Shell objects never enter domain state. Reducers are testable headless.

## Lifecycle contract (blocker B2)
- One lifecycle owner; `enable()` is reversible and rolls back on partial failure; `disable()` removes every proxy, subscription, signal handler, timer, animation, keybinding, chrome actor, grab, cancellable.
- Every async callback checks a generation token; stale callbacks after disable/owner change are dropped.
- User session only (`session-modes: ["user"]`); lock screen → teardown via disable.

## Error model (blocker B1, DoD #9)
- DoD #9 means: recover from specified adapter/UI errors (malformed metadata, D-Bus method errors, owner vanish, artwork failure, timeout) without affecting other players or Shell.
- Every promise awaited or given a rejection handler; all async I/O bounded (deadline + cancellation).
- Per-player fault: drop/re-add that player only; island hides only if no eligible player remains.
- **Residual risk (accepted, documented):** extension shares gnome-shell process; main-loop blocking, native crash, OOM cannot be contained by JS.
- Diagnostics: rate-limited `console` logs tagged with UUID.

## MPRIS
- Discovery: subscribe `NameOwnerChanged` **before** `ListNames` snapshot; reconcile by owner generation.
- Declare own Player interface XML incl. `Position`, `Rate`, `Seeked`, `Can*` (Shell's bundled XML lacks them).
- Selection: rank by last playback/track transition only (not volume/artwork); Playing > Paused; startup tie-break by bus name order; Stopped = ineligible; owner loss → fall back to next ranked.
- Metadata normalized + length-bounded with type validation; missing title/artist/art → placeholders.
- Controls gated by `CanControl` + `CanPlay/CanPause/CanGoPrevious/CanGoNext` + current state; method errors logged, no optimistic success.
- Progress: async read `Position` on attach/selection/expanded entry; monotonic anchor extrapolated with `Rate`; re-anchor on `Seeked`, status, track, rate change; timer only while expanded; hide progress if duration unknown. Timers never poll tracks.

## Artwork
- `gi://Soup?version=3.0`, `send_async` streamed read with byte cap, finite deadline, redirect limit, per-track cancellation + stale-result guard.
- `file://` read async with same size cap; unsupported URI → placeholder.
- Cache `~/.cache/<uuid>/` with size-based eviction. Controls work fully offline. M1 may use placeholder only.

## Display / input
- Bounded chrome actor (pill size only, no full-screen wrapper), no strut, under top bar, centered on primary monitor.
- Placement from current primary geometry + measured panel allocation; recompute on `monitors-changed`; no primary → hide.
- CSS px already scaled by St (integer scale factor) — never multiply again; test 100/125/150/200% and mixed-scale hotplug.
- Fullscreen app on primary → hide (Shell fullscreen tracking).
- Expanded only: `Main.pushModal()` grab for Esc/outside-click; guaranteed `popModal()` on collapse/disable.

## UI/UX
- States: hidden → compact (~200×32: mini art, truncated title, play/pause icon) → peek (~3s auto on track change; hover >300ms) → expanded (art, metadata, progress, prev/play/next).
- Compact: **icon = play/pause, body click = expand.**
- Transition precedence: disabled/locked/no-active/fullscreen > explicit expansion > auto peek > compact. Auto-peek never collapses an explicit expansion; timers cancellable; peek never takes focus.
- Paused: pill stays visible, auto-hides after 5 min paused.
- Fixed black pill, full radius; explicit foreground/disabled/focus styles; high-contrast + large-text checked.
- Clutter `ease()` 200–250ms; honor `enable-animations` (never `animationRequired: true`).
- A11y: configurable toggle shortcut (non-conflicting default), standard focusable St buttons, dynamic Play/Pause accessible names.
- GSettings (no prefs UI in M1): `auto-peek` (bool), `show-island` (bool), `toggle-shortcut`.

## Packaging
- Fixed UUID + GSettings schema id. `gnome-extensions pack` → `install` → `enable`; uninstall removes extension; cache removed, settings retained (documented).
- Starts with desktop once enabled; no extra autostart. Wayland reload = log out/in (or nested shell for dev), never Alt-F2 `r`.
- Rejected: .deb (no gain over zip for single user), Flatpak (cannot render Shell chrome), user service/daemon (process split not needed).

## Known risks
- In-process extension (see Error model residual).
- Ubuntu-patched Shell vs upstream API differences; per-major breakage.
- Pill overlaps top of maximized windows (accepted, mitigated by small compact + `show-island`).
- Auto-peek may expose track info during screen share (mitigated by `auto-peek` toggle; no detection in v1).

## Acceptance
- **M1 gate:** headless reducer tests + controllable session-bus MPRIS fixture; owner-run GNOME 50 Wayland check: compact shows, play/pause works, no player → hidden, repeated enable/disable leaves no leaks/grabs.
- **v1 gate:** test matrix covering all 10 DoD items incl. multi-monitor hotplug/primary switch and scaling.

## Next
`/to-plan` → milestones starting with M1 PoC: compact pill + play/pause on real MPRIS.
