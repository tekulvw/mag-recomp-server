# Explicit gameplay worker match profiles

The gameplay worker supports one selected world at a time. Fixed-profile startup
continues to use the existing CLI resources. Formation mode is an explicit retail
opt-in with `--application-id 20231 --match-profiles /absolute/profiles.json`.
The existing required base resource arguments, including `--spawn-catalog`, remain
required and validated. The registry is an explicit override at acknowledged
selection time; a missing optional profile resource never reuses a base resource.
The base remains a supported no-bunker profile, even when registry rows supply
Domination hospitals. A noninteractive initial-load Domination profile can serve
as that base; supply its matching initial objective catalog. The worker refuses
admission until an acknowledged registry selection replaces that base world.

All resources are external operator inputs. The public tree contains source and
invented fixtures only. These schema checks do not prove native identities,
vehicle behavior, live compatibility or match performance.

## Manifest and resource validation

The manifest is a JSON object with format `mag_match_profiles_v1` and between one
and 64 rows. Pass the manifest's absolute path to the worker. Resource paths
inside it can be absolute or relative to the manifest's directory. Relative
paths contain ordinary, nonempty components. Profile identifiers are unique,
1–64 ASCII letters, digits, hyphens or underscores.
Unknown fields fail validation. A manifest example using invented paths is:

```json
{
  "format": "mag_match_profiles_v1",
  "profiles": [
    {
      "profile_id": "synthetic-sabotage-a0-d1",
      "spawn_catalog": "/operator/worlds/synthetic-a0-d1/spawns.json",
      "objective_catalog": "/operator/worlds/synthetic-a0-d1/objectives.json",
      "collision_world": "/operator/worlds/synthetic-home1/collision.json",
      "navmesh_world": "/operator/worlds/synthetic-home1/navmesh.mag-nav",
      "ground_vehicles": "/operator/worlds/synthetic-a0-d1/vehicles.json",
      "character_damage_rules": "/operator/worlds/synthetic-a0-d1/damage.json"
    }
  ]
}
```

Only `profile_id` and `spawn_catalog` are mandatory row fields. Omitted optional
resources mean absent features, subject to the selected spawn policy's existing
requirements. In particular, objective-enabled policy requires a valid objective
catalog and assignments for each selected squad. All files are loaded and
validated before logs/listeners open. Selection performs no file I/O, so changing
a resource file after startup does not change the preloaded world. Repeated
collision paths are loaded once and immutable triangle/BVH buffers are shared
across profiles and selections.

The spawn catalog determines the summary, sides and map. Registry mode supports
Sabotage (`game_type: 0`), Acquisition (`game_type: 1`) and Domination
(`game_type: 2`) with automatic team assignment, two distinct factions and
defender-owned map metadata. Domination uses all 32 squad bindings and may supply
the three per-profile hospital inputs together: `bunker_placement`,
`bunker_structure` and `bunker_allocation`. Other modes retain their existing
no-bunker profile restriction.
Both sides must have enough authored squad capacity for the declared team
capacity. `maximum_players` is twice that capacity and cannot exceed 64.
All six ordered faction pairs for each mode can be listed; the registry does not
invent or fill missing pairs.

Objective map/mode/wire identities, squad assignments and forward spawns must
agree. Collision metadata must match the spawn map. An optional
`collision_coverage` path supplies a [coverage sidecar](collision-coverage-sidecar.md)
for the row's `collision_world`; it requires collision and must name that world's
exact hash and map. Navmesh requires collision,
its exact source hash, matching map and coverage validation against the world's
regions and any sidecar regions. Vehicles and
character damage rules retain their existing exact spawn-source hash checks.
Item catalogs, bootstrap equipment, combat/skill data and the map-independent
falling/incapacitation rules remain global. Bootstrap faction coverage is checked
for every loaded profile. Registry workers can use native election with `--leadership-passives` and
`--leadership-role-policy`. Schema 2 enables squad vacancy filling. Schema 3
also enables platoon vacancy filling using its separately projected weights.
A platoon promotion can vacate a squad leadership slot, which the worker
refills in the same roster update. Both schemas elect from the worker's
assigned roster. Retail's separate pre-match squad reassignment is outside
that policy.
Registry mode rejects development role bindings, global bot population, global
bunker arguments and development dummy settings. Per-profile Domination
hospitals are supported.

An interactive Domination profile additionally supplies `domination_round` and
`domination_gameplay`, ground vehicles, character damage rules and collision.
Its worker also needs dynamic body bindings and a qualified static partition for
each map. Startup preloads and validates the complete backend before advertising
the profile; selection installs a fresh clone and an exact formation retry keeps
the current world. See [Domination configuration](domination-runtime.md) for the
explicit backend inputs and current insertion scope.

## Active squad commands

Profiles with an Acquisition or Domination gameplay provider enable squad
ClusterBomb and Sensor where the native mode and role make them available.
By default the worker loads `<map_name>--active-abilities.json` from the
gameplay configuration's directory. To relocate a manifest, set the profile's
`active_abilities` field to its absolute or registry-relative path. A fixed
Acquisition worker can use `--active-abilities` for the same override. Startup requires native squad
election without development bindings and validates the manifest against the
profile's collision world and objective catalog. Sabotage grants no active
squad commands.

The setup manifest accepts absolute paths and paths relative to its own directory.

```json
{
  "format": "mag_active_abilities_v1",
  "rules": "/operator/rules/ability-rules.json",
  "effects": "/operator/rules/ability-effects.json",
  "map_bindings": "/operator/worlds/synthetic-map/ability-map-bindings.json",
  "static_ray_receipt": "/operator/worlds/synthetic-map/static-ray-receipt.json"
}
```

The effects projector reads a supplied retail game archive. The static-ray
projector adds root and child filter metadata to a collision assembly and emits
a receipt containing the world hash and authored Y bounds. Use the projected
world as the profile's `collision_world` and the receipt as
`static_ray_receipt`. The ability map bindings identify the exact gameplay
objective catalog and level source. Startup checks those identities against
the loaded world and objective files. Generate the binding for the exact worker
catalog and retail map. [Map binding projection](active-ability-map-bindings.md)

Rebuild any attached navmesh against the
projected world hash. A new formation constructs a fresh command runtime from
the preloaded setup, while a repeated identical selection preserves it.

## Bridge contract

A successful `hello` advertises capability `match_profiles_v1` and a
`match_profiles` array. Standard rows contain:

```json
{
  "profile_id": "synthetic-sabotage-a0-d1",
  "game_type": 0,
  "attacking_faction": 0,
  "defending_faction": 1,
  "maximum_players": 64,
  "team_capacity": 32
}
```

Before any reservation or client GameInfo, the coordinator must select an idle
worker profile and require an exact successful acknowledgement:

```json
{
  "v": 2,
  "request_id": "synthetic-request-1",
  "op": "select_match_profile",
  "formation_id": "synthetic-formation-1",
  "profile_id": "synthetic-sabotage-a0-d1"
}
```

A successful normal bridge response includes `selected_match`:

```json
{
  "formation_id": "synthetic-formation-1",
  "profile_id": "synthetic-sabotage-a0-d1",
  "game_type": 0,
  "attacking_faction": 0,
  "defending_faction": 1
}
```

Selection defaults to a two-human quorum with one on each side. Registry
configuration rejects a lower total minimum or a zero minimum on either side.
The coordinator then configures the selected match's human minimum/capacity with
`configure_match`, including at least one human per selected faction, and admits
the cohort. Late joins use the existing selected formation and its two sides.
The worker never changes a saved character's faction.

## Group reservation opt-in

A retail profile worker started with `--party-reservations` advertises
`party_reservation_v1`. Its eligible profile rows add `squad_capacity` and
`squad_counts`, the two authored squad counts in attacker/defender order.
The coordinator still checks the team's human limit and the whole roster's
fit in one actual squad. The selected-match acknowledgment includes the
prospective `admission_epoch`. Use the value returned by the worker on both
group requests. An identical selection retry preserves the epoch.

The v2 `reserve_party` request carries `formation_id`, `profile_id`,
`admission_epoch`, `world_id`, `cohort_id` and ordered `members`. Each member
has the same reservation ID, player ID, account ID, session key, access key
and character fields as an individual `reserve`. The worker validates the
entire cohort before recording once and installing any members. Success
returns `party_reservation` with the same binding fields, `status: "reserved"`,
the selected `team` and `squad`, and the original ordered `reservation_ids`.
An exact retry can use a new correlation request ID.

Complete each formation's group reservations before making any members
claimable or sending their descriptors. A new group cannot enter after any
current member claims. An exact `release_party` for the same binding rolls back
an entirely unclaimed cohort. It may also fence a never-admitted exact cohort
with an empty ID list. Success has `status: "released"` and the ordered IDs
when applicable. It omits team and squad. The worker retains this
acknowledgment through empty-world cleanup until the next selection commits.
Use ordinary release for individual departures after publication.

## Selection lifetime and durability

Admission returns `match_profile_not_selected` until selection succeeds. An exact
formation/profile retry is idempotent, even with active actors. Reusing a live
formation with another profile returns `match_formation_profile_conflict`.
A different formation cannot replace an unfinished active world; it returns
`match_profile_selection_locked`. With `independent_match_cohorts_v1`, a terminal
world moves into bounded results-only retention while the successor is selected.
Pending durable rewards/loadout selections stay in their original outboxes and
do not block the next formation.
Unknown profiles fail without changing the current selection or world.

A new idle selection allocates/validates its next state before recording the
selection journal entry. Failed writes or log limits do not install or
acknowledge the new world. A successful switch resets objective/spawn/round
state, world and match clocks, roster/turret/repair/parachute state and bot runtime,
and replaces every world resource, including clearing omitted optional resources.
Global item/skill resources, reward receipts and persistence acknowledgement
history remain intact.

Last-actor release/expiry or other world cleanup retires the selected formation
and closes admission again. It also retires old reservation IDs in registry mode;
new cohorts require new formation and reservation identities. A retired formation
cannot be selected again. At 4096 retired formations, further new selections fail
with `match_formation_history_full`; an operator can drain pending durable work
and restart the worker to bound session history. Fixed-profile workers retain
a bounded history of 512 retired reservation IDs across rounds.

Pending loadout selection receipts survive actor release/expiry in an outbox and
continue to appear in bridge responses until settled through the original
reservation/world/player/selection/revision binding. Pending rewards likewise
remain available for their existing acknowledgement flow. Both outboxes survive
independent cohort rollover. See [result cohorts and requeue](match-cohorts.md)
for bounded retention, ordered delivery and exact identity rules. No worker
selection permits concurrent live multi-world hosting, starts a service, or
authorizes a release.

## Shared squad spawn selection

A ready client can change its squad's shared spawn only after its initial objects
exist. With native leadership enabled, the sender must hold the squad-leader
role for its current reservation and squad. Legacy profiles without native
election use the first squad member.

Selection uses the squad's currently supported, available and active spawn
points, including points activated by the mode's progression. Supporting other
retail points requires authoritative runtime point ownership and a physical
spawn path for their type. Members following the previous shared point follow
the new one, while individual selections are retained.
