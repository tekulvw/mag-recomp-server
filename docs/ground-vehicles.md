# Retail ground-vehicle creation

`--ground-vehicles /absolute/VEHICLES.json` enables an explicit external catalog
for retail Acquisition or the qualified Domination profile scope. Loading requires the exact spawn catalog SHA256,
verified native spawn pool identities, matching spawn tokens, active points and
the configured side/faction. Missing or unverified bindings fail before startup.
No vehicle IDs, properties or placements are inferred from authored row ordinals.

The catalog has `format: "mag_ground_vehicles"`, `schema_version: 1`, a `source`
object and a `vehicles` array. `source` requires SHA256 strings named
`spawn_catalog_sha256`, `native_elf_sha256`, `native_binding_sha256` and
`vehicle_properties_sha256`. These identify externally reviewed inputs; syntax
validation is not independent certification of the underlying native evidence.

Each vehicle supplies separate `index` and `spawn_point_index` values,
`spawn_point_token`, `vehicle_type_token`, `faction`, `team`, `modifier`,
`position: {x,y,z}`, normalized `rotation: [x,y,z,w]` and
`health_quarter_points`. The registry allows32 unique ground-vehicle indices
and distinct active spawn bindings. It publishes at most16 creations per packet
within the existing transport budget.

The retail type1 creation path emits30 field-presence bits and the evidenced
initial active/owner/type/faction/state/turret-owner/damage/team/spawn/transform
fields. Position uses the shared49-bit codec, rotation uses44 bits and the
effective shared damage payload uses120 bits. Fresh operational health and no-hit
values are explicit server policy. Optional armed inventories and turret angle
state retain native defaults; they are not fabricated from incomplete equipment
information.

Initial delivery and late join each receive the configured vehicle snapshot.
Creation batches use ordinary replication history, sequence and NAK replay.
Batch progress commits only after successful evidence recording.

The [APC passenger insertion path](apc-insertion.md) adds explicit player-point
bindings, per-vehicle passenger occupancy, mounted respawn, snapshots and qualified
static exits. The maintained `project-retail-apc-insertion.py` joins initial-load
spawn bindings to original faction properties and independently qualified exits.
It supports each opposing attacker on each of the three Acquisition maps. Its
initial-load proof concerns a fresh/reset compiled load before dynamic creation;
it does not observe an existing process's pool occupancy. Vehicle network indices
remain an explicit local allocation policy, separate from native spawn indices.

Explicit driver catalogs connect qualified Acquisition Escorts and opted-in
Domination APCs to the shared manual seating and owner-motion path. A spawn row
alone does not grant driving or turret authority. Native game acceptance,
deployment and a release require their own review.

## Admitted-seat visibility

`project-retail-ground-seat-visibility.py` reads the original vehicle registry and the exact faction/modifier property member for each admitted seat. It expands authored physical seat counts and requires the runtime passenger/operator role to match the authored slot. The projection stores the original archive, registry and per-property member digests alongside an explicit `occupant_visible` boolean. It accepts the qualified Acquisition and Domination creation scopes; arbitrary scope names remain rejected.

```sh
python3 tools/extraction/project-retail-ground-seat-visibility.py \
  --ground-catalog /absolute/GROUND.json \
  --game-archive /absolute/game.psarc \
  --output /absolute/UPGRADED-GROUND.json
```

The output must be outside the source checkout. Repeat with `--check` to verify exact reproduction. For Domination, passenger APC allocations supply the admitted seats; motor-pool source allocations without admitted seats produce no visibility rows. Providing this property does not enable driver or gunner control.

A bound Structural objective with nonzero periodic area radius and damage requires complete visibility for every admitted ground seat before gameplay starts. The area scan can encounter any occupied seat before applying its distance test, so leaving one vehicle unqualified can interrupt processing even if it is far from the objective. Hidden passengers are skipped without damage while the scan cursor still advances. Maps with no enabled periodic Structural effect retain optional visibility compatibility.

Updating a Domination ground catalog changes its bytes. Regenerate the paired ground-lifecycle projection with those exact upgraded ground files and update the gameplay configuration's lifecycle path and SHA256; retain the matching spawn catalogs and active-ability sidecars. An old lifecycle digest cannot bind upgraded ground data. These source checks remain separate from native live-match review.

## Domination APC driver opt-in

The Domination ground projector can explicitly enable manual driver and passenger entry for the eight source-type3 attacker APCs. Vehicle indices are source allocations: select by kind rather than assuming that APCs occupy the lowest indices. Type7 motor-pool sources receive no new seating or control authority, and turret seat1 remains excluded.

```sh
python3 tools/extraction/project-retail-domination-ground.py \
  --spawn-catalog /absolute/POST-HOSPITAL-SPAWNS.json \
  --ground-sources /absolute/GROUND-SOURCES.json \
  --exits /absolute/QUALIFIED-EXITS.json \
  --enable-apc-driver --game-archive /absolute/game.psarc \
  --spawn-output /absolute/DRIVER-SPAWNS.json \
  --ground-output /absolute/DRIVER-GROUND.json
```

The explicit flag requires the original game archive. The projector verifies its digest, original registry, unique faction/modifier wheeled-class variant, exact property member, passenger layout, health and occupancy. It emits driver seat0 and immediately verifies original physical roles and visibility for every admitted driver/passenger slot. Outputs must be outside the public source checkout. Without the flag, the existing projection stays byte-identical and does not take an archive argument. Repeat with `--check` to verify reproduction. The enabled projection preserves the final spawn bytes, while ground/lifecycle/config hashes change.

The generated limits are conservative replacement policy:8m manual mount distance,750ms position age and25m/s reported driver speed. They are not extracted retail constants. Every original APC variant has an authored occupancy cap8; the Engine conservatively counts all occupants against it. The original can transfer ownership to passengers. This implementation grants motion only to the authenticated operator in seat0 and revokes that grant to server owner0x105 when the driver leaves or loses authority.

An opted-in Domination catalog requires exactly eight source-type3 driver rows, each at physical seat0 with passenger slots2–9 and cap8. Legacy no-driver catalogs retain their previous behavior. Admitted operator visibility is part of the same complete seat inventory required by periodic Structural area effects.

Driving changes dismount qualification for every occupant of that APC. The runtime rechecks the candidate exit against the current rotated vehicle pose, loaded map-matched static scene and current dynamic bodies. Failed clearance retains the seat and driver grant. Reproduce and test all initial poses and moved/rotated exits using the intended collision resources; a stationary source-exit receipt alone does not establish current-pose clearance. Keep each gameplay configuration's collision world and active-ability sidecar digests synchronized when staging data.

## Changing seats in place

The client's change-seat action sends an ordinary mount request for another
seat on the vehicle it already occupies. The client picks that seat itself. It
steps through the seats in index order, skips occupied seats and wraps around.
From the driver or gunner seat it can stop on a passenger seat. From a
passenger seat it skips the other passenger seats. On a vehicle with driver
seat0, gunner seat1 and passenger seats2–9, changing seats from the gunner seat
therefore lands in seat2 first, and changing again from there reaches the
driver seat. The server honours the requested seat and never substitutes
another one.

An in-place change is handled like a fresh mount of the new seat, without the
on-foot distance and freshness checks. The old seat is released in the same
update. Taking seat0 grants owner field1 to the new driver and starts a fresh
motion budget, exactly as a fresh driver mount does. Leaving seat0 revokes
field1 to server owner `0x105`, so nobody can drive a vehicle that has no driver.
Taking seat1 grants turret owner field6. Leaving seat1 sets field6 to `0xFFFF`
and clears the turret grant. A request for a seat that another occupant holds
is refused, and no seat or grant changes. Other clients learn who sits where
only from that character's state14 vehicle and seat user data; the vehicle has
no separate occupancy field. Late joiners receive the current field1 and
field6 owners in the creation row.
`in_place_seat_changes_grant_driving_exactly_like_a_fresh_driver_mount` and
`in_place_seat_change_into_a_held_driver_seat_is_refused` cover the Domination
APC and the Acquisition Escort.
