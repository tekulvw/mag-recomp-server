# Default squad abilities and strike damage to structures

The human explicitly requested: “Make it the default, then proceed with structure
damage.” Supported source-gameplay Acquisition and Domination configurations must
therefore install the completed ClusterBomb/Sensor runtime automatically. This
supersedes the earlier default-off choice for these abilities. Unrelated rendering
features, native game/deployment holds and manual-only releases retain their scope.

## Approach

Keep runtime data external and source-bound. Resolve one deterministic ability
manifest from the selected gameplay provider/map when no explicit path override
was supplied. Missing or mismatched inputs fail startup rather than silently
removing a gameplay mechanic. Preserve authored empty Sabotage availability and
shell/diagnostic configurations that have no concrete gameplay provider. Require
the native leadership policy; enabling defaults must not invent leader grants.

Reuse the normal recorded combat and world lifecycle for structure damage. Qualify
retail collider user-data identities separately from network object types, follow
the actual damage receiver and its guards, and extract any missing inputs with
neutral tools. A network or script objective must not become damageable merely
because it has a position. Existing hospital, objective, current collision and
vehicle lifecycle consumers are reused where the retail mapping proves them.

Alternative considered: a Boolean enable flag or permissive missing-input fallback
would leave the mechanic optional and contradict the request. Writing structure
health directly would bypass native damage guards, armor, attribution and deferred
lifecycle transitions. Neither is the selected implementation approach.

## Required behavior

- Real source-gameplay profiles load abilities by default and preserve explicit
  path overrides. All twelve source faction profiles validate through that path.
- Default input resolution is deterministic for directories containing multiple
  maps; it does not depend on filesystem probing order or private machine paths.
- Structure targets come from source-qualified live collider/body identity and
  shape coverage. Network type alone cannot establish admission or immunity.
- Supported strike damage uses authored armor/friendly-fire/metadata and the normal
  destruction, occupant, collision, respawn and objective callbacks where applicable.
- Unsupported or immune native target classes remain faithful to their native
  receiver instead of receiving invented generic blast health damage.
- All updates remain transactional. A failed query, model, or durable record
  preserves damage state, pending strike ordinals, publication and rewards.
- Assets, populated catalogs and decompilation remain private. Public tests use
  invented data, and public builds work independently of the private workspace.

## Verification and boundaries

Use meaningful default-loading and timed damage regressions, affected lifecycle
checks and the complete standard suites before source integration. Reproduce the
native receiver/mapping proof and load all private source profiles offline. Source
PRs may integrate after checks under standing authorization. Preserve the released
private gitlink and unrelated indices, working files and services. No game launch,
deployment, release PR merge, tag or release publication is part of this increment.
Native-client live-match acceptance remains a separate human review.
