# Coucou Island M13 — Idle Clock + Agent Off — Brainstorm Lock (2026-10-07)

Base: `spec-raw/coucou-island-lock.md` (all prior constraints still hold). Target: GNOME Shell 50.1, Wayland.
Research: Apple HIG Live Activities (compact / minimal / expanded); WWDC23 "Design dynamic Live Activities".

## Goal
Island replaces GNOME's center clock when idle, uses Apple-style size classes for multiple sources, and the Claude agent features are switched off and hidden for now.

## Decisions
1. **Idle = collapsed clock.** No active peek → island is a small pill showing `HH:MM` (follow `org.gnome.desktop.interface clock-format`). It sits where GNOME's center clock was.
2. **dateMenu.** Hide only the dateMenu clock label (do not destroy dateMenu). Click on the idle island → open the original dateMenu (calendar + notifications). Disable → label restored exactly.
3. **`idle-hide-minutes` is ignored while the island replaces the clock** — the top bar must never lose its clock.
4. **Size classes.** 1 source → compact (leading icon/name, trailing value/progress). ≥2 → minimal: main pill + max 2 detached dots that still carry state (color/progress ring), not just a logo.
5. **Priority.** Newest peek takes the main pill for `auto-close-seconds`, then returns. Playing media is the default background.
6. **Expanded list.** Hover + delay opens it: new key `expand-hover-delay-ms` (default 400). Pointer leave → collapse after 300 ms (no flicker). List ordered by priority, one row per source, max N rows then scroll.
7. **Click while active** → open the source app (e.g. media player). Click while idle → dateMenu (decision 2).
8. **Agent Claude off.** `watch-agents` and `agent-approvals` default `false`; hide the Agent section in prefs. Agent code stays (`agents.js`, `agent-status.js`, `approvals.js`, `agent-card.js`, `permission-rule.js`), re-enable via gsettings. Owner machine: `gsettings reset` both keys if previously set.

## Hard constraints
- No new GTK in `extension.js`; no reliance on Shell internals beyond hiding/showing the dateMenu label actor and opening the menu.
- Enable/disable is fully reversible (top bar identical after disable).
- Reducers stay pure + headless-tested: idle/compact/minimal/expanded transitions, priority, hover-delay timing.
- All existing tests (636) keep passing.

## Done when
- Unit tests cover new island states + priority + hover delay + agent defaults off.
- Owner nested pass (devkit): idle clock in center, click → dateMenu, 2 sources → minimal dots, hover → list, disable restores GNOME clock, no Agent section in prefs.

## Open
- None.
