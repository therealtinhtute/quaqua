# coucou-island — review lessons (bug classes quansu found in M9/M10)

Check every change against these before handing off. Full evidence: `~/quaqua/runs/quansu-m9vol.md`, `~/quaqua/runs/quansu-m10mic.md`.

1. **Local write ≠ platform truth.** A Gvc writable property (`volume`, mute) mutates locally and notifies synchronously, before `push_*`. Never treat a re-read of the stream (setter notify, rescan, `notify::description`/`name`) as confirmation. Check `push_*`/write return values; on false/throw roll back to the last confirmed value and log once.
2. **Arm suppression/pending before the write**, not after. Pending must be visible in the view and part of paint dedup.
3. **Fakes must match the real API.** Notifying setters, sync signals, boolean returns, GNOME 46 properties only (verify in local typelib/upstream 46 source; e.g. no monitor property on GNOME 46 Gvc). A fake that emits later hides ordering bugs.
4. **Focus:** `grab_key_focus()` returns void; never branch on its result. The expanded shape owns one Tab scope (card + dots); focus the first control on entry/page change; update dots in place, don't rebuild them.
5. **Unmap mid-interaction:** hiding a card must cancel drags/grabs idempotently; motion without an active button never writes.
6. **Page stickiness:** activity changes (incl. last activity removed) never yank the user off Controls/Calendar; only collapse/environment blocks reset.
7. **Per-control deadlines:** one control's commands must not postpone another's overdue deadline. Device/source switches must not emit false transition peeks.
8. **Keyboard/range math uses the spec values,** not the amplified slider max (PageUp = 10 pts, End = 100%).
9. **Every new signal/handler** is recorded and disconnected on switch/destroy; subscribe to every field you snapshot/dedupe.
- Re-check test fakes against every new platform major before trusting green tests — 2026-10-07, runs/truonglao-g50.md
- In tests/support/view-stubs.js stub templates, never put backticks in comments (breaks the mirror parse) — 2026-10-07, runs/sude-g50.md
- Any manual-unzip dev path must run glib-compile-schemas --strict itself (pack ships only .gschema.xml) — 2026-10-07, runs/sude-nestfix.md
- GNOME 50 PanelMenu uses ClickGesture recognize-on-press on the ancestor button: a clickable child inside it must stop the press and call menu.open() explicitly, not toggle() — 2026-10-07, runs/sude-m13c.md
- A view test that throws before view.destroy() leaks the shape and modal stack into later tests; keep fixtures side-effect-free (autoExpand:false) or destroy in finally — 2026-10-07, runs/sude-m13wire.md
