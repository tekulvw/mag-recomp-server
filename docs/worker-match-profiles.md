# Explicit gameplay worker match profiles

The gameplay worker supports one selected world at a time. Fixed-profile startup
continues to use the existing CLI resources. Formation mode is an explicit retail
opt-in with `--application-id 20231 --match-profiles /absolute/profiles.json`.
The existing required base resource arguments, including `--spawn-catalog`, remain
required and validated. The registry is an explicit override at acknowledged
selection time; a missing optional profile resource never reuses a base resource.

All resources are external operator inputs. The public tree contains source and
invented fixtures only. These schema checks do not prove native identities,
vehicle behavior, live compatibility or match performance.

## Manifest and resource validation

The manifest is a JSON object with format `mag_match_profiles_v1` and between one
and 64 rows. Every path, including the manifest path, must be absolute. Profile
identifiers are unique, 1–64 ASCII letters, digits, hyphens or underscores.
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
Sabotage (`game_type: 0`) and Acquisition (`game_type: 1`) with automatic team
assignment, two distinct factions, defender-owned map metadata and no bunkers.
Both sides must have enough authored squad capacity for the declared team
capacity. `maximum_players` is twice that capacity and cannot exceed 64.
All six ordered faction pairs for each mode can be listed; the registry does not
invent or fill missing pairs.

Objective map/mode/wire identities, squad assignments and forward spawns must
agree. Collision metadata must match the spawn map. Navmesh requires collision,
its exact source hash, matching map and coverage validation. Vehicles and
character damage rules retain their existing exact spawn-source hash checks.
Item catalogs, bootstrap equipment, combat/skill data and the map-independent
falling/incapacitation rules remain global. Bootstrap faction coverage is checked
for every loaded profile. Unswitchable bot population, leadership, bunker and
development-dummy combinations are rejected in registry mode. Their map-scoped
state must be explicitly implemented before those combinations can be supported.

## Bridge contract

A successful `hello` advertises capability `match_profiles_v1` and a
`match_profiles` array. Rows contain exactly:

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

## Selection lifetime and durability

Admission returns `match_profile_not_selected` until selection succeeds. An exact
formation/profile retry is idempotent, even with active actors. Reusing a live
formation with another profile returns `match_formation_profile_conflict`.
A different formation cannot replace an active world or bypass pending durable
rewards/loadout selections; it returns `match_profile_selection_locked`.
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
their previous reservation cleanup behavior.

Pending loadout selection receipts survive actor release/expiry in an outbox and
continue to appear in bridge responses until settled through the original
reservation/world/player/selection/revision binding. Pending rewards likewise
remain available for their existing acknowledgement flow. The next world cannot
be selected until both durable outboxes are clear. No worker selection permits
concurrent multi-world hosting, starts a service, or authorizes a release.
