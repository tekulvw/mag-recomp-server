# Acquisition dynamic collision and lifecycle source support

The optional `--dynamic-bodies /absolute/BODIES.json` resource enables an actual
convex penetration provider and read-only Engine body collection. It does not
install the Acquisition atomic callback backend. Original-derived shape inputs,
native instruction evidence and review profiles remain private.

## Geometry and completeness

`dynamic_collision::Catalog` accepts source-bound capsules, spheres, rounded
boxes and convex hull point sets. A template contains every separately registered
root. A ground vehicle's chassis and wheel are two roots with the same target;
both hits are retained. Local root offsets rotate with the parent entity, and
the convex radius is added to the unchanged shape core. Shape resources are
immutable, reference counted and validated before use.

`Snapshot::capture` compares instantiated rows against an independently supplied
complete entity roster. Missing, extra, duplicate or mismatched identities and
generations reject the snapshot. It retains the supplied entity/root traversal
order, without sorting or deduplicating hits. This is explicit replacement
authority; native broadphase traversal equivalence is unproven.

Queries calculate convex-core distance bounds and compare them with the sum of
shape radii. Work and iteration limits, unsupported geometry and uncertain
boundary contacts return an error for the entire query. No partial recipient
list is returned. Native hull welding, solver tolerances, numerical bit parity
and exact tangency classification are not claimed. Query rotation must come
from the qualified caller; this module does not invent a native Euler matrix.

The caller supplies the **current** raw filter6 mask. The query applies it to
each body's current low five layer bits. A source's initial filter table is not
assumed to remain current forever.

## Actual Engine collection

`Engine::dynamic_collision_snapshot` collects the current owned human, bot,
development-target and ground-vehicle objects. Initial unspawned characters have
the qualified zero-position constructor body. Dead characters retain a body on
the dead layer; incapacitated characters remain on the live layer. Mounted
characters remain present and follow their ground vehicle's position. An
unavailable authoritative pose or template fails the snapshot.

Ground poses, health, incarnation and registration now come from the Engine's
owned mutable ground runtime. The catalog supplies immutable source/model/seat
policy. State2 can await registration after respawn; the ordinary registration
phase adds roots, and state0 retains the allocated entity without roots. Late
join, mount distance, passenger insertion and exits use current authority.
Vehicle driving admission and complete death-effect application remain open.
There is also no owned mine admission or
mine registry: accepting mine templates does not create live mines.

Snapshots carry Engine origin, world incarnation and recorded-event revision.
Before committing effects, the callback owner must recheck snapshot freshness
while serializing authority and separately recheck the external filter source.
Another Engine's otherwise identical snapshot is rejected.

Resource installation is explicit, idle-only and restricted to retail20231
Acquisition. CLI offline validation checks the base vehicles and every preloaded
match profile before opening logs or listeners. Builder installation and profile
selection repeat those compatibility checks.

## Initial ground population

Ground creation supports separately qualified hidden state0 entries and native
vehicle source descriptors. A descriptor joins source family/index/token,
vehicle type, team, spawn type and authored availability to the exact creation.
It is independent of the player spawn catalog's availability: an active vehicle
source must not enable an unsupported player spawn point.

Private source extraction accounts for four compiled insertion APCs, four
initially hidden FSP Escorts and eight active Trucks per Acquisition map. These
are distinct objects. Requested free client/network indices preserve the four
APC slots; selected extra indices are explicit owned free-slot policy, not a
claim about original Sim allocation order. Native vehicle-type compatibility
aliases require their own source evidence. Unknown tokens are rejected.

Private offline review compositions validate all six Acquisition faction
pairings through the actual Registry, catalog and Engine constructors. They
preserve existing human spawn rows and released bundle inputs. Exported source
rotation representatives preserve the native matrix/sign convention; exact VMX
normalization low bits and quaternion packet bit equivalence remain unproven.

## Lifecycle handlers and remaining integration

`acquisition_spawn_blockers` implements the qualified bounded character push
control flow: at most two outer pushes and two casts per push, shallow-contact
limits, partial movement, slide projection and final contact verification. Its
required query provider must include static geometry, current filters, closest
contacts, tolerances and the exact spawning-body exclusion. Dynamic entity
penetrations alone cannot implement those world casts. An incomplete provider
must return an error.

`acquisition_damage_metadata::vehicle_death_occupant` constructs the native
null-context ordinary damage request. Seat policy has three category flags and
a separate optional damage request pointer. Attacker selection and fallback to
occupant self precede teamkill compensation. This request preserves ordinary
protection/modifier/armor handling and never forces health to zero.

The concrete [world and lifecycle consumers](acquisition-world-effects.md) now
provide contact/sweep queries, staged ordinary character damage/corrections and
existing-vehicle respawn. Complete ground death/hulk effects, mine lasting radial
effects, FSP objective integration and assignment sweeps remain required.
Native FSP activation reuses the existing
Escort; it must not allocate another APC, fabricate a timer, or treat an
intermediate state request as committed script execution. The complete atomic
callback remains disabled until these consumers exist and are checked. See
[effect qualification](acquisition-effect-qualification.md).

Releases remain manual after human live-match review.
