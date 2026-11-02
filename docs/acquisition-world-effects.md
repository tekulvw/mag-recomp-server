# Acquisition world queries and lifecycle consumers

The world provider performs actual convex contacts and capsule sweeps against
static triangles and current dynamic roots. Existing-vehicle respawn and
ordinary character effects now have concrete Engine transactions. The full
objective callback remains uninstalled; this source checkpoint is not a claim
that Acquisition matches can run to completion.

## Query inputs and behavior

Static `mag_collision_world` v1 instances optionally carry explicit `collision`:

```json
{
  "filter_info": 0,
  "convex_radius": 0.0,
  "child_filter_policy": "uniform",
  "provenance": "Invented example; actual inputs require source evidence"
}
```

These invented example values are not native defaults. Existing ray/AABB loaders
remain compatible with files lacking this metadata, but body casts reject those
files. A body query requires complete static coverage or a verified physics
region containing its entire swept bounds, including margins. Missing root
filters, child policy, margins or regional coverage return an error.

`acquisition_world::Filters` supplies the current full32-row table and every
dynamic root's full filter word. Same nonzero system groups use subgroup/exclude
comparisons instead of the layer table. Owned replacement grouping assigns one
distinct group per character/vehicle, shared by its roots and separate from
static groups. Group numbers are internal identities, not native allocation
number parity. Mine owner grouping remains unsupported.

The provider refines convex-core distance and, for intersecting cores, support
hull penetration depth and escape normal. Radii are added separately. Sweeps use
conservative advancement with explicit replacement tolerances and bounded work.
Separating initial contact can escape; inward initial contact can block at0.
Only the exact moving root and spawning root are excluded. Another root sharing
the spawning vehicle identity can still block. Numerical/broadphase parity is
not claimed, and unresolved geometry never becomes a fabricated no-hit.

## Existing vehicle respawn

`prepare_existing_acquisition_respawn` reuses an exact current ground allocation
and qualified source point/final placed pose. It temporarily stages mutable
ground authority, collects actual pairs and resolves each in order. Character
correction/damage is consumed immediately on the isolated candidate so a death's
filter change affects later casts. Ground negative damage changes model health
and vaporize/pending-death state; it does not synchronously delete the vehicle.

Preparation restores live authority before returning. The record method checks
Engine origin/revision/round, native inputs and unchanged ground state, records
the complete effect/output data once, then installs without fallible checks.
Source counters increment even for healthy no-op respawns. Hidden geometry can
serve the spawning contact query without registering world roots early. The
ordinary world-tick registration phase adds healthy state2 pending roots.

Runtime type1 deltas use only state5, damage18, position25 and quaternion26 fields.
Late joins replay current pose/health/state; NAK replay preserves exact remembered
bytes. Mount/passenger admission requires a healthy registered state2 vehicle.

This method does not synthesize objective state-entry effects or acknowledge the
whole FSP callback. Callers still own source-point activation, objective/stat
bookkeeping and the complete surrounding transaction.

## Ordinary character damage and owner corrections

The character candidate reuses armor, resistance, friendly-fire modifiers,
incapacitation/death, cleanup, statistics, XP and reliable notifications. It
supports the qualified raw1000 volume request and null-context vehicle-death
occupant request. Collector duplicates run against the already modified
candidate. Missing required native state or damage resources rejects preparation.
Candidates are fenced to their Engine/revision/round/native inputs/topology.

Forced positions use Character field5, including the owner, without respawn or
life reset. Tracking holds the correction and queues newer owner reports.
Prototype8 / retail9 correction ACKs identify server sequence, client sequence,
object type/index and field. Unlock requires the authenticated owner, type3,
that character, field5 and exact current correction server sequence. The lock
binds after recording and recipient sequence rewriting, not to a placeholder
queue sequence. Transport delivery ACKs do not unlock it. The owned monotonic
counter profile explicitly handles ACK client sequence0 without freezing at
UINT_MAX. It does not claim all native counter-wrap behavior.

Native transitive OnCharacterHit and character-associated objective/supply-point
callbacks remain outside this consumer's claimed closure. Current Engine
objective invalidation is retained; no synchronous native callback is invented.

## First playable mode and remaining work

Prioritize capture/unlock, Escort movement/delivery/destruction, scoring and
match end. Exact solver bits, native traversal order and quaternion low bits are
fidelity limits rather than additional gameplay gates.

Concrete remaining requirements include qualified static FSP query inputs,
objective capture/entry integration, vehicle driving admission, full destruction
explosion/occupant/FSP/hulk consumers, scoring/results and global assignments.
The old private APC-exit witnesses do not cover FSP source centers and cannot
certify those query regions. Do not promote provisional metadata or invent root
filters to enable callbacks. Mines require owned admission and lasting effects
before they can be enabled.

No game/deployment acceptance follows from synthetic or offline checks. Releases
remain manual after human live-match review.
