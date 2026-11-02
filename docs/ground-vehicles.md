# Retail ground-vehicle creation

`--ground-vehicles /absolute/VEHICLES.json` enables an explicit external catalog
for retail Acquisition. Loading requires the exact spawn catalog SHA256,
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

Driving, turret equipment/combat, damage/repair, destruction and moving exits
remain separate typed implementation work. A spawn row alone does not enable a
vehicle. Native game acceptance, deployment and a release require their own review.
