# Acquisition Escort primary gunners

**Goal:** Enable the source-qualified hidden seat1 gunner on the four current
type9 Escorts in each Acquisition profile, preserving driver/capture/delivery
authority, passenger-only reinforcement and optional integration.

**Baseline:** fresh main `94773c1cafdf43b65d1cf7063f44c1ad63f64ed2` after the
mounted Ground fix. Both SDK ancestries were refreshed; unrelated state is
preserved. Releases remain manual, and native runs/deployments/accounts are
outside this implementation.

## Source and design

The current six mounted-area v6 profiles bind24 Escorts. Original Cougar,
Maultier and Bushmaster properties admit driver0, hidden turret0 operator1,
passengers2–9 and all-occupant cap8. The named seat resolves at model nodes9/2/3
with nonzero reference offsets. Its HMG is a single automatic hitscan primary
with no secondary; those item IDs are shared with unenabled type7 guns, making
exact role qualification essential. Reinforcement uses other APC allocations.

Reuse the existing family1 codec, independent owners, common grant fence, live
pose and recorded HP/reward publication. Add an explicit Acquisition type9
schema alongside the existing Domination APC schema, with exact source hashes,
three variants and six loaded profile pairs. Keep metadata scoped separately
as `ground_escort_primary`, and require the matching scope at the constructor.
Do not turn metadata membership into an ordinary/type7 weapon grant.

The direct Acquisition configuration and profile-registry loader both need the
optional rules path and item-bound qualification. Existing Domination config
and outputs remain supported. Any common projector refactor must preserve its
previous outputs under exact reproduction.

Delivery state8 is the additional lifecycle seam: revoke grant readiness inside
the recorded Ground candidate and clear owner6 in the same arrival delta as
driver/state changes. Existing driver-only capture and Arrival credit remain.
Hidden/rebuilt/recaptured allocations preserve the match-lifetime grant counter.

## Work and verification

- Runtime owner: policy scope, mode1/type9 qualification, optional config/registry,
  current pose, seat/lifecycle/area integration and delivery revocation.
- Combat owner: isolated Escort metadata and typed current-role construction,
  active-round/native framing, shared infantry and existing scoped Ground target
  transaction. Helo/Objective/secondary routes remain separate.
- Projector owner: owned-original Escort source/model/item projection, named-node
  lookup without APC node2 assumptions, exact six v6 profile pairs and tests.
- Root: combat selection extension, private staging, combined verification and
  focused source integration, then continue to impact rocket mechanics.

Observe focused behavioral RED/GREEN, source HMG armor/kill/credit, separate
driver/gunner controls, current creation and named pose, stale grants, capacity,
delivery unseat/field6/rollback and hospital/APC compatibility. Prove the connected
Capture→boarding→delivery cleanup path with current original resources before
source integration. Use critical checks, normal source CI and no release action.
