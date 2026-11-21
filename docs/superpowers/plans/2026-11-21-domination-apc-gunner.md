# Indexed Domination APC primary gunner implementation

**Goal:** Admit an independent physical seat1 operator on each of the eight
existing source-type3 APCs per profile, with authored primary hitscan damage to
characters. Continue autonomous gameplay work after checked integration.

**Baseline:** freshly fetched public main `bcd4ac6c551ac8442bbd8cdf4de0b550dd8e0091`.
Both SDK main ancestries were refreshed; their tracked states and unrelated
private submodule change are preserved. Releases remain manual. No native game,
deployment or account operation is part of this increment.

## Qualified scope

Original Ground family1 has32 allocations and30 fields:25 inherited Vehicle
fields plus5 dynamics fields. Gunner owner6 is independent of common owner1.
Turret0 aim, definition and equipment use the already qualified common retail
widths. All three source-type3 variants have one authored hitscan primary and
an absent secondary. Physical seat1 uses the named model node2 with a nonzero
reference offset, composed with current vehicle pose; hospital's zero-local
fallback does not apply.

Driver0, passengers2–9, reinforcement selection and the existing all-occupant
cap8 remain. Type7 and legacy profiles remain separate. Common owner1 remains
server-owned without an authenticated driver in this replacement policy; native
first-occupant fallback is recorded but does not grant motion to a gunner.
Secondary rockets and armored/Objective target damage are follow-on increments.

## Implementation

- Add optional source-qualified APC turret rules through Domination configuration
  and registry loading. Bind exact original identities, faction/model/seat/weapon
  variants and all six loaded spawn/Ground profile pairs. Expose admission and
  hidden-seat visibility only after successful qualification.
- Reuse the common retail turret codec and grant fence where practical. One
  family1 decoder must consume turret fields and dynamics together while
  authorizing each component against its actual seat. Preserve existing driver
  and hospital behavior. Grant versions are not reused within match lifetime;
  exact equipment readiness is required after every new grant.
- Mount, switch, dismount, life changes, release, destruction and respawn must
  preserve independent driver/gunner roles. Current seat position derives from
  the named model offset and live body transform. Keep recorded publication,
  FIFO/creation order, bounded bootstrap and rollback guarantees.
- Add separate mounted-only APC combat metadata and a current-role constructor.
  Only authenticated active-round primary hits against characters enter the
  existing armor/life/kill/reward transaction. Reject ordinary-kit, foreign-seat,
  stale-grant, secondary and vehicle-target attempts.
- Maintain owned-original extraction and deterministic reproduction. Stage the
  six current profiles with their OIC, collision and hospital bindings intact.
  Observe focused RED/GREEN, then full affected checks, normal startup and a
  connected source-only two-role replay before source integration.

## Work ownership

Runtime owner handles rules, admission/lifecycle/config/registry and common grant
reuse. Wire/extraction owner handles pure family1 common fields plus the maintained
projector and its synthetic tests. Combat owner handles isolated metadata and
damage constructors. Coordinate shared source sections and serialize Cargo runs.
Root handles interfaces, combined checks, private staging and source integration.
