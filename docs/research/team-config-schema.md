# Team Configuration Schema

## Goal

Make worker model allocation, counts, and behavior configurable per project with fallback defaults. Values should not be hardcoded in scripts or jq planning.

## Required fields

- `schema_version`
- `team.defaults.cos.kind`
- `team.defaults.cos.model`
- `team.defaults.lead.kind`
- `team.defaults.lead.model`
- `team.defaults.peer.kind`
- `team.defaults.peer.model`
- `team.defaults.max_peers_per_orchestrator`
- `strategy.builder_reviewer_must_differ`

## Optional fields

- `team.projects.<slug>.override`
- `team.defaults.peer.effort`
- `strategy.name`
- `strategy.move_only_if_no_working_deferred`
- `features.shadow_mode`
- `features.candidate_token_priority`

## Example

```yaml
schema_version: 1
team:
  defaults:
    cos:
      kind: tieude
      model: google-antigravity/gemini-3.8-flash
    lead:
      kind: ni
      model: claude-sonnet-5.5
    peer:
      kind: tieude
      model: google-antigravity/gemini-3.8-flash
    max_peers_per_orchestrator: 4
    builder_reviewer_must_differ: true
  projects:
    zimage:
      override:
        peer:
          kind: suhuynh
          model: claude-sonnet-5.5
        max_peers_per_orchestrator: 3
```

## Resolution rule

1. If project override exists, use it.
2. Fill missing keys from defaults.
3. If unresolved, fallback to SLP defaults in AGENTS.
4. Reject unsafe config at startup (peers over cap, builder/reviewer same, unapproved model drift).

## Herdr token requirements

- `grid_role`: cos | lead | peer
- `grid_owner`: team owner
- `grid_group`: project group
- `grid_ready`: 1 or empty
- `grid_model`: resolved model
