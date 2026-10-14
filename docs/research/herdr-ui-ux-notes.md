# Herdr SLP Visual UX Notes

## Constraints
- No native non-terminal plugin UI in Herdr v1.
- Use tabs, panes, agent names/states, prompts, and popups only.
- Prefer lowercase labels and distinctive glyphs.
- Do not depend on workspace color support that is not documented.

## Visual language
- `peach cos` (`peach` icon before lowercase role).
- `torch lead`
- `peer`
- `workers grid`
- `cos`
- `lead-grid`
- `peer-sude`
- pane title should derive from role, not terminal `cwd`.

## Role signals
- Cos terminal prompt colored peach/pink.
- Lead terminal prompt colored blue.
- Peer terminal prompt neutral worker blue-gray.
- Sidebar agent state comes from Herdr agent detection only.
- Agent names and status text come from config.

## Tab ownership
- `cos`
- `leads`
- `peers`
- `pipeline`
- `stage`

## Popup dashboard
- Use `placement = overlay` or `popup`.
- Tail `state/telemetry.json`.
- Show concise text only:
  - role count
  - deferred status
  - pending ops count
  - last reconcile
  - foreign panes warning

## Action identities
- `quaqua.slp_verify_and_reconcile`
- `quaqua.slp_status`
- `quaqua.slp_spawn_worker`
- `quaqua.slp_shutdown`
