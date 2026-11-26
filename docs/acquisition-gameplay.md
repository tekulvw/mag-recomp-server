# Acquisition gameplay

Acquisition is an explicit runtime opt-in. The backend connects timed capture,
the ordered controller, existing Escort activation, driver ownership and motion,
delivery/destruction, scores, results, and a fresh round. Existing profiles stay
unchanged unless they supply the gameplay configuration. Source and synthetic
Engine checks are separate from native live-match acceptance.

## Configuration

For a standalone worker, add `--acquisition-gameplay /absolute/GAMEPLAY.json` to
the explicit retail mode1 startup inputs. For faction matchmaking, put an
absolute or registry-relative `acquisition_gameplay` path in each enabled
match-profile entry instead of supplying that standalone flag. The registry retains fixed character
factions, longest-waiting eligible faction priority and the selected pair's roles.

The strict `mag_acquisition_gameplay` configuration binds the selected map/spawn
catalog, controller, initial objective identity evidence, startup rules and game
rules. It supplies ordered objective/squad inputs, four FSP joins, actual damage
and Arrival geometry, current filter rows, vehicle capabilities/armor and
reinforcement intervals. The controller and Arrival static paths in a file can
be absolute or relative to that configuration's directory. The inputs stay
external to the public source repository. Hashes identify supplied inputs; they do not certify
their semantics independently.

Enabled configurations require all four distinct Escort allocations and their
source-matched driver policies, actual compiled map collision, retail combat
metadata, character damage rules, dynamic body/hulk templates and source-bound
static partitions. Missing gameplay damage or Arrival resources reject the
configuration. Startup checks run before logs/listeners; registry checks run
before profile advertisement/selection. `--validate-catalogs` performs offline
configuration checks without opening logs or serving a match.

## Structural objective assets

New source-projected profiles can include the `structural` member in their
Acquisition gameplay configuration. Paths resolve relative to that configuration
file.

```json
{
  "structural": {
    "bindings": "structural-bindings.json",
    "effects": "structural-effects.json",
    "collision_assets": "structural-collision",
    "terrain_index": "terrain-materials.json",
    "terrain_materials": "terrain-materials.bin",
    "repair_catalog": "retail-structures.json"
  }
}
```

Use `project-retail-acquisition-structural.py` with the selected map's original
disc, metadata, objective and spawn catalogs, and shared game archive. Use
`--mode GameType128` on `project-retail-domination-structural-effects.py` and
`project-retail-domination-terrain-materials.py` to prepare the matching effects,
collision assets and terrain materials. Keep the projected inputs outside the
public checkout. When `structural` is present, its linked inputs and original
motor-pool allocations must validate before the profile loads. Existing
configurations can omit this member.

The v2 bindings enable ordinary Structural arming, defusing, fuse detonation,
destruction and repair using the extracted rules. Ordered trigger bindings retain
the selected demolition box and its authored explosion position. The authenticated
interaction route requires a live, ready, on-foot actor; TCP repair also requires
the currently equipped source repair tool. Repair tuning comes from the supplied
repair catalog. Legacy v1 bindings retain their passive behavior.

The provider runs the sourced startup entries and native Structural updates
in family-12 order, including Precision transitions. It applies linked insertion points, motor-pool hooks,
tactical command availability, Structural explosions and area damage, and
sequence collision. Committed sequence state is included in late-join
replication. Bound native infantry spawn types 4, 5 and 8 use the authored
category-zero point for local character insertion until their physical routes
are qualified.

### Controller objective sequences

Capture and Arrival objectives are run by the ordered controller, not by the
Structural provider, but their definitions can still name one authored
sequence per entered state. A capture objective typically names one sequence
for its deactivated state, which builds the closed enclosure around the
Escort, and another for its completed state, which rebuilds that enclosure and
plays its opening. Clients only change an objective's sequence when the
objective's sequence fields (fields 9 and 10: a signed index and the game time
it started) change, so the server must publish them.

With `--mode GameType128`, `project-retail-domination-structural-effects.py`
adds a `published_sequences` list to the effects catalog. Each row names a
non-Structural objective whose definition has a non-empty sequence list: its
class, definition hash, the sequence index for each of the twelve states (-2
keeps the current sequence, -1 stops it) and, for each sequence, its source
hash and event kinds. Definitions without sequences are left out.

At load, every published objective must bind to a controller objective that
has no Structural row. Each time the controller commits a state for such an
objective, the server looks up that state's index. It publishes nothing for
-2, for a state with no authored slot, or when that index is already selected;
otherwise it selects the index, stamps it with the commit's game time and
publishes fields 9 and 10 with that tick's Structural sequence updates. The
selection joins the Structural sequences in late-join replication, so a client
that joins after a capture receives the completed sequence with its original
start time. A new round clears it. Catalogs without `published_sequences` keep
the previous behaviour.

The server only publishes these sequences; it does not replay them. It adds no
collision for the objects they create and models none of their animation. The
enclosure's collision and its opening exist only on clients, and the server's
collision world never holds the enclosure. Nothing on the server therefore
keeps the Escort inside, and the server has nothing to remove when the
enclosure opens. Server-side bot and projectile queries still pass through the
enclosure walls, as before.

Profiles with Structural area damage require complete authored occupant visibility
for every admitted ground seat. Run `project-retail-ground-seat-visibility.py`
against the original game archive and the selected ground catalog to add that
source-bound inventory. Hidden occupants are excluded before the radius query;
visible occupants use the current vehicle root position. Missing visibility
rejects an enabled area-damage profile at startup. This does not qualify mounted
firearm or radial-blast body coverage.

## Automatic vehicle repair

The scheduler consumes effective `kCharModAutoRepair` for living, ready
occupants of source-type-9 Escorts and source-type-3 APCs. Qualified passengers,
drivers and gunners use their authoritative occupied seat and the current vehicle
root for leadership proximity. Gunners need a current seat grant and can receive
the passive before initializing their primary. The passive uses neither an
equipped repair tool nor a client repair request and grants no repair work or XP.

Each character life owns its repair timer. An elapsed interval strictly greater
than 1.5 seconds produces at most one attempt per scheduler update, carrying the
remainder. Loss of association or an effectively zero modifier resets the timer.
A due attempt on a full or currently unrepairable associated vehicle still
consumes its interval. Repair uses a percentage of the loaded maximum HP,
retains float HP, updates health conditions and preserves prior damage
attribution. Timer, leadership state, HP and field18 publication commit after
recording succeeds. Cadence-only observations produce no empty evidence record.
Current and late-join recipients see the resulting health.

This first consumer admits operational, living, noncritical Escorts. Other
Ground source types, field-hospital emplacements, aircraft and destruction/rebuild repair remain
separate work. Preserving the previous published source field is an explicit
bounded policy; the native outgoing builder's transient source-field provenance
and exact native publication timing remain unqualified. Automatic character
healing is a separate consumer. Native match acceptance remains pending.

## Owned gameplay

The objective tick clones objective, ground, character, round and vehicle-wave
authority. Entry effects run before the ordered controller; target requests do
not change the observations used by later branches. Pending state entries and
assignments use the same candidate. One durable record precedes installation
and network release. A failed query, effect or record leaves live authority
unchanged. Completed and subsequent Locked states retain their publication order;
capture holders receive the existing same-generation exit handshake.

Capture activates the exact existing Escort source and clears spawn blockers
through real convex queries. The owner grant uses the authenticated connection
index. Vehicle dynamics require the currently held driver seat, a fresh owner
sequence, operational life and the configured movement budget. Exits at moved
poses use current static support, clearance and dynamic occupancy.

Arrival requires the linked Escort, an admitted current driver, actual chassis
overlap and a positive upward five-metre raw24 ray. A static source witness or
current dynamic root can establish that positive. Lack of a witness is not a
claim about a complete native-world miss. Every occupant needs a supported,
distinct exit before delivery commits. Delivery revokes ownership, enters
vehicle state8 with the configured inert timer, and applies the source objective
award once. The A/B awards in the qualified profiles are600/400 against1000
defending points. Timer expiry moves the delivered vehicle to inert7.

Smoke grenades are not part of this configuration. Pass the worker's
`--smoke-grenade-rules` flag instead (see [smoke grenades](smoke-grenades.md)).
A configuration that still sets `smoke_grenades` is refused at startup.

### Bots after a capture

The objective catalog has no interaction for an arrival objective, so the bot
planner takes its place from this configuration: the centre of the arrival
trigger the captured vehicle must reach. The side that owned the capture
defends it, holding guard places around the trigger; the other side heads
there on its attack places. Bots do not drive the captured vehicle.

A bot whose objective still has no place (no interaction, demolition or
arrival trigger) backs off for the planner's stall interval and reports
`bot_objective_waiting` with reason `bot_objective_unplaceable` once per task.
It does not count towards the spawn-wave backlog, so it can neither hold the
shared planning budget nor keep the raised wave budget on.

Authenticated firearm reports and grenade detonations use source equipment and
vehicle armor. Fractional HP persists across hits. Critical vehicles decay using
the configured rate. Destruction applies ordinary occupant damage, then the
radial blast, FSP target notification and body/hulk changes. Source-type9 Escort
replacement requires recapture; it does not use the generic reinforcement timer.
APCs(type3) and motor-pool trucks(type7) queue the same allocations at an advanced
vehicle-class wave, enter state6, then respawn at the next due wave. A new match
clears that queue, clock and live vehicle state.

Scores use the native-bound per-update bleed primitive. Depletion publishes the
result once, advances the end deadline and uses the shared terminal/release
path. Late joins receive committed scores, objectives and current vehicle state.

## Scope and verification

Synthetic Engine scenarios cover both deliveries, victory/countdown, release and
recapture; destruction/occupants/recapture; APC/truck reinforcement and restored
human insertion; late joins; rejected routes; and failed-record rollback. Private
offline consumers qualify all six faction pairings across the three maps. These
checks do not establish a native rendered match, throughput or frame rate.
Connected Structural scenarios also cover selected-trigger arming, defusing and
release, interruption by an actor life change, TCP repair through quiet frames,
rearming, late-join state, reset, and failed-record rollback.

Exact Havok solver/traversal/quaternion bits, native material penetration and
dynamic blast occlusion remain fidelity work. Radial destruction uses an explicit
replacement hard-static-LOS policy. The admitted ground seats are source-bound
closed seats protected from the radial callback; their separate death-occupant
request still applies. Native medals, full objective/vehicle statistics and
cosmetic fades remain deferred. Mines, vehicle-mounted weapon operation and
additional ability effects require their own admitted mechanics. Supporting a
template or decoding a field does not enable those features.

Releases remain manual after the maintainer's live-match review.

## Optional APC crew

Acquisition can opt into the four source-bound insertion APCs' driver and
primary gunner seats. Use the v2 Ground catalog and set `apc_turrets` in the
Acquisition gameplay config alongside its existing `ground_turrets` Escort
source. The loader checks both turret documents against the selected map and
exact Spawn and v2 Ground bytes before installing their disjoint policies.
Existing v1 Ground catalogs continue to use the Escort-only driver scope.
The offline worker reports `acquisition_apc_crew_validated` after binding.

`tools/extraction/project-acquisition-apc-crew.py` composes new candidates from
an owner-qualified Acquisition registry and crew qualification, the retail
APC turret projection, and the matching Acquisition Escort turret projection.
It writes six new Ground documents, a new APC turret document and an Escort
document rebound to those new Ground bytes. Keep the inputs and prior output
directories for reproduction. The projector consumes qualified external
evidence and does not extract retail vehicle properties itself.

Seat 0 uses the existing Ground driver authority and bounded motion policy.
Seat 1 uses turret-zero primary authority. Reinforcement still selects from
passenger seats 2–9, and all occupants count toward the single cap of eight.
Moving exits use the current pose and collision checks. Destruction ejects
occupants and revokes grants before the existing Ground reinforcement cycle.
The stock infantry rocket and ordinary frag paths can damage operational APCs
when their source-backed resources are active. The Acquisition direct bullet
target and manual repair scopes continue to cover Escort only.

## Field hospitals

Each defending squad's field hospital ("FORWARD SPAWN: Bunker", spawn type 8,
category 2) is the forward spawn of one Structural objective: the bunker the
attackers destroy. In the original game the point carries a mountable
defensive emplacement, the same vehicle type as a Domination hospital. A life
due there is seated in it and leaves through its authored exits by the doors.

A profile that supplies the three hospital inputs (`bunker_placement`,
`bunker_structure` and `bunker_allocation`, see
[worker match profiles](worker-match-profiles.md)) binds those emplacements, and
its spawn catalog lists the field hospitals as bunker passenger points. The
runtime then uses the Domination hospital insertion unchanged:

- a respawn at the point takes the highest-numbered free passenger seat of that
  hospital's emplacement, and the client receives the emplacement with the
  other initial objects (late joiners get its current state);
- a life that cannot be seated (destroyed hospital, wrong side, full) starts on
  foot at the squad's alternate insertion, else at the point, as described in
  [apc-insertion.md](apc-insertion.md#when-the-apc-cannot-seat-a-new-life);
- the player leaves by a clear authored exit, nearest the camera heading first;
  clearance also checks the current Structural sequence bodies;
- bots arrive on foot at the same exits
  ([bot-active-spawns.md](bot-active-spawns.md#defending-squads)).

The emplacement follows its Structural objective's forward spawn, the way a
Domination hospital follows its FSP. When the bunker is destroyed, its point
goes inactive and the squads routed there fall back to their category-zero
point. In the same transaction the emplacement is destroyed, its seated
occupants die through the shared vehicle-occupant death rule, and a family-13
update reaches every client that has the emplacement
(`acquisition_hospital_cycle`). The FSP activation that follows a repair
restores the body. No hulk or destruction blast is bound for Acquisition; the
Structural objective's own sequence supplies the wreck. Direct hits on the
emplacement do not damage it in Acquisition: the Structural objective's damage
governs the bunker. Hospital turrets (seat 0) are not bound for Acquisition.

Without the three inputs every point behaves as before, and profile loading
refuses hospital inputs without the Acquisition gameplay configuration or for a
bound hospital that is not a type-8 Structural forward spawn.

`tools/extraction/project-retail-domination-hospitals.py --mode acquisition`
writes the inputs from an Acquisition spawn catalog, the structure catalog,
externally qualified field-hospital exit rows (format
`private_acquisition_field_hospital_exits_v1`), qualified Domination hospital
placements on the same structure catalog (for each emplacement model's mounts,
seats and passenger pose binding) and the conditional fresh-load allocation
model. Each field hospital takes the lowest free family-13 index in spawn
order. It also writes the promoted spawn catalog; rebind every resource that
pins the old catalog's SHA-256. The client's acceptance of these emplacements
(seating and exits on a live client) remains a lab check.
