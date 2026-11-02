# Explicit APC passenger insertion

Retail Acquisition can bind a player spawn point to an explicitly configured
static ground vehicle. This extends the existing `--ground-vehicles` catalog;
all native identities, model properties and placements remain external inputs.
The generic implementation contains no faction-specific vehicle tokens, map
positions, passenger seat lists, occupancy limits or exit offsets.

`tools/extraction/project-retail-apc-insertion.py` produces both catalogs from
the original pre-APC spawn catalog and external evidence. Supply `--disc`,
`--game-archive`, `--native-elf`, `--spawn-catalog`, `--bindings`,
`--binding-proof`, `--exits`, `--spawn-output` and `--ground-output`. `--check`
requires byte-identical existing outputs. The CLI checks original archive/ELF
fingerprints and the supplied proof's actual bytes. It validates joins and
qualification labels; those checks do not themselves prove native semantics or
ground clearance. Normalized bindings require a complete token/index bijection,
compiled position and unit quaternion, matching level hashes and the explicit
`native_initial_load` qualification. This means successful fresh/reset compiled
load before dynamic creation, without a live occupancy claim. Every location,
squad/default/available/active reference follows the same remapping.

The projector reads the selected attacking faction's modifier-zero vehicle
variant and properties. It expands physical seat counts in authored order and
keeps maximum occupancy separate, applies the placement offset once using f32,
and converts externally settled exit feet into model offsets. It restores only
active player APC insertions; other unsupported kinds remain unavailable. Manual
remount policies remain absent unless separately qualified and configured.

A spawn location whose `kind` is `vehicle_passenger` requires a corresponding
entry in the ground catalog's optional `passenger_spawns` array:

```json
{
  "passenger_spawns": [
    {"spawn_point_index": 0, "spawn_point_token": 1000, "vehicle_index": 0}
  ]
}
```

These invented values illustrate the schema only. `spawn_point_index` identifies
the player insertion point in the verified spawn pool; `vehicle_index` separately
identifies the type1 network vehicle. The vehicle retains its own independently
validated `spawn_point_index` and `spawn_point_token` creation binding. There is
no conversion from an authored row ordinal to either native identity. Duplicate,
missing, inactive, unknown or token-mismatched player bindings fail validation.
The existing source SHA256 and native pool verification requirements still apply.

Each referenced vehicle must supply these additional model-qualified properties:

```json
{
  "passenger_seats": [2, 3, 4, 5, 6, 7, 8, 9],
  "maximum_occupancy": 8,
  "passenger_exit_candidates": [
    {"ranking_offset": [1, 0, 0], "position_offset": [3, 0, 0]}
  ]
}
```

This is also synthetic example data. Production seat lists, capacity and model
exit offsets must come from the qualified external projection. Seat lists must
be unique and within the supported physical-seat bounds; capacity must be
positive and cannot exceed the configured passenger seat count. Unreferenced
creation-only vehicles may omit all three fields. A passenger binding requires
all three, including a validated nonempty exit policy. A syntactically valid
provenance hash does not independently certify the evidence it names.

Respawn keeps the existing selected point, team, faction, squad, wave deadline,
selected loadout and new-life generation flow. The recipient first receives all
configured type1 creation batches. The server then chooses the greatest available
configured passenger seat on the bound vehicle, subject to that vehicle's
configured capacity. Separate vehicles have independent occupancy. A full
vehicle, missing policy or wrong actor side leaves the pending spawn intact.
The state14 Character payload is `[1, vehicle_index, physical_seat]`; field9
accompanies the ordinary respawn. Passenger insertion grants no vehicle owner,
driver or turret authority. The static vehicles retain the existing server policy
of precreation in operational state2; this does not implement or claim the
native on-demand available-to-spawn6/spawning1 lifecycle.

APC occupancy uses a separate typed `(vehicle_index, seat)` state. The legacy
Alpha bunker `mounted_seat` behavior is preserved. Common mounted checks exclude
APC passengers from on-foot movement authority, damage targets, weapon use,
healing, objective interactions, development movement and leadership claims.
Actor snapshots retain the current APC identity and physical seat. Shared FIFO
updates wait until that recipient's ground creations have been durably recorded,
so late join cannot receive a mounted reference ahead of its vehicle creation.
Ordinary sequence history and NAK replay preserve exact encoded bytes.

The existing mount0216 and dismount0217 requests resolve ground IDs by their type1
network identity. Mount requires the authenticated character, a living ready
actor, matching vehicle side/faction, completed creation delivery, a configured
passenger seat, available capacity and an exit policy. Exact repeated mounts and occupied seats are denied. A same-vehicle switch to a
free configured passenger seat retains its occupied capacity slot. Moving from
another vehicle requires a prior exit. On-foot remount additionally requires
explicit finite positive `maximum_mount_distance_m` and positive
`maximum_mount_position_age_ms` on that vehicle. Both default to absent, which
denies manual remount while preserving authorized spawn insertion and exit.
The server checks distance from the current tracked position to the current
static vehicle transform; missing, future, expired or stale-sequence position
samples deny the request. There is no inferred native distance or timeout. Dismount requires that exact owned vehicle and physical seat.
It takes the first externally prequalified candidate in supplied native order,
transforms its settled offset by that vehicle's current static position/rotation,
and publishes a character state/position/yaw correction. No Alpha object ID or
bunker placement participates. Exact repeated dismount and NAK requests replay
the committed correction. Ground clearance and model-offset qualification belong
to the external policy producer; the server does not infer terrain clearance.
Dynamic-vehicle exit clearance is not implemented. Camera facing determines
final yaw and does not change the ground exit candidate position.

All seat changes commit after evidence recording. Release/expiry removes the
actor-held occupancy, terminal damage cleanup clears it, and round reset clears
remaining APC seat state. The unsupported mounted self-death wire transition
remains explicitly denied. Driving, vehicle damage/repair/destruction, passenger
weapon use, turret combat, dynamic transforms and native live-match acceptance
remain outside this insertion path.

The synthetic tests in `ground_vehicle_runtime_tests.rs` exercise all three
faction values, owner-packet respawn/exit and exact replay, independently decoded
mounted payloads, current-state snapshots, external seat/capacity selection,
occupied/repeated/invalid/wrong-side mounts, late-join creation ordering, release,
terminal damage, round reset and evidence-failure rollback. They contain no
private vehicle properties, native map data, assets or captured packets.
