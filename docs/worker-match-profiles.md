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
defender-owned map metadata. Domination uses all 32 squad bindings, with up to
eight members per squad (the retail size). Domination and Acquisition may
supply the three per-profile hospital inputs together: `bunker_placement`,
`bunker_structure` and `bunker_allocation`. An Acquisition profile with them
also needs its `acquisition_gameplay` configuration, whose Structural objectives
own the field hospitals. Sabotage keeps its no-bunker profile restriction.
Both sides must have enough authored squad capacity for the declared team
capacity. `maximum_players` is twice that capacity: the world's character
capacity, humans and bots together. It cannot exceed 256, the client's
Character pool. See [World capacity and the human cap](#world-capacity-and-the-human-cap).
All six ordered faction pairs for each mode can be listed; the registry does not
invent or fill missing pairs.

Objective map/mode/wire identities, squad assignments and forward spawns must
agree. Collision metadata must match the spawn map. An optional
`collision_coverage` path supplies a [coverage sidecar](collision-coverage-sidecar.md)
for the row's `collision_world`; it requires collision and must name that world's
exact hash and map. An optional `collision_filter_rows` path (a JSON list of
the 32 retail collision filter rows, or a file with a `filter_rows` list, such as
an Acquisition gameplay configuration) requires collision. With it, static rays
skip triangles on layers their retail query row passes through. Without it,
every triangle blocks, as before. The direct worker takes the same file as
`--collision-filter-rows` (see [character movement and ray
layers](extraction.md#character-movement-layers)). Navmesh requires collision,
its exact source hash, matching map and coverage validation against the world's
regions and any sidecar regions. Vehicles and
character damage rules retain their existing exact spawn-source hash checks.
Item catalogs, bootstrap equipment, combat/skill data and the map-independent
falling/incapacitation rules remain global. Bootstrap faction coverage is checked
for every loaded profile. The worker-wide `--smoke-grenade-rules` flag gives
every row [smoke grenades](smoke-grenades.md), whatever its mode, bound to that
row's own objective catalog. Startup refuses it when a row has no objective
catalog, or when an Acquisition or Domination row has no gameplay
configuration. Gameplay configurations no longer name smoke rules. Registry workers can use native election with `--leadership-passives` and
`--leadership-role-policy`. Schema 2 enables squad vacancy filling. Schema 3
also enables platoon vacancy filling using its separately projected weights.
A platoon promotion can vacate a squad leadership slot, which the worker
refills in the same roster update. Both schemas elect from the worker's
assigned roster. Retail's separate pre-match squad reassignment is outside
that policy.
Registry mode rejects development role bindings, the global
`--bot-population`/`--bot-resupply-*` arguments, global bunker arguments and
development dummy settings. A bot population and its resupply providers are
bound to one map, so each profile supplies its own (see below). Per-profile
Domination and Acquisition hospitals are supported.

## Bot-filled profiles

A profile row can fill its world with bots. The optional row fields are:

| Field | Meaning |
| --- | --- |
| `bot_population` | `mag_bot_population_v1` file for this profile's map. Required by every other bot field. |
| `bot_fill_total` | Total world population (humans plus bots). Defaults to the worker's `--bot-fill-total`. |
| `bot_names` | Bot names for this profile. Defaults to the worker's `--bot-names` (or the synthetic names). |
| `bot_resupply_catalog` | Optional bot resupply providers for a Sabotage, Acquisition or Domination profile on a qualified map ([bot-static-resupply.md](bot-static-resupply.md)). |
| `bot_resupply_approach_policy` | Optional approach policy. Requires `bot_resupply_catalog` and the profile's navmesh. |

Paths resolve like the other row resources. Bots that route need the
profile's own `navmesh_world` (and its `collision_world`). An example row with
invented paths:

```json
{
  "profile_id": "synthetic-sabotage-a0-d1",
  "spawn_catalog": "worlds/synthetic-a0-d1/spawns.json",
  "objective_catalog": "worlds/synthetic-a0-d1/objectives.json",
  "collision_world": "worlds/synthetic-home1/collision.json",
  "navmesh_world": "worlds/synthetic-home1/navmesh.mag-nav",
  "bot_population": "worlds/synthetic-home1/bots.json",
  "bot_fill_total": 64,
  "bot_names": "names/bots.json"
}
```

Startup loads these files and then runs the ordinary bot population checks
for each profile on a scratch engine that holds only that profile's spawn
catalog, collision world, navmesh and bunkers plus the worker's items, wire
profiles, names and fall-damage rules. The check covers the population's map,
capacity, team and squad slots, spawn bindings, terrain support, navmesh
coverage, bot firearms and, when present, the resupply binding and approach
policy. Any failure refuses startup before logs or listeners open, with an
error of the form `match profile <profile_id>: <reason>`. `--validate-catalogs`
reports `profile_bot_fill_validated`. A worker-wide `--bot-fill-total` is
refused unless at least one row has a population. Populations that use
regional collision or a regional team are refused: they need a
coverage-qualified regional navmesh and live structure state that profile rows
cannot supply.

Selecting a profile installs that profile's validated population, names and
resupply inputs; selecting a profile without bots leaves bots disabled. Bots
start filling once a human is on foot, keep each team at no more than half of
the fill total, and yield seats to humans who join later. Bots pack squads
rather than spreading over them: they fill squads a human occupies first, then
squads whose spawns can reach an objective, then the fullest squad with a free
bound slot, then squads whose authored objectives (in the objective catalog)
include one of an occupied squad's on the other side, then the lowest squad
index, so platoons fill in order and both sides head for the same objectives.
Acquisition pairs each attacking platoon with a defending platoon on the same
objectives, so when a side's first platoon is demoted the other side follows it
to the paired platoon instead of fighting over objectives nobody defends. A squad
cannot reach an objective when one of its first four slots is on the navmesh
and none of them has a route (walking or through certified drops) to any
objective in the objective catalog; it is checked once per installed
population, recorded as `bot_population_unroutable_squads`, and still fills
once the other squads are full. Without a navmesh, fall rules or objective
catalog no squad is demoted. When the population shrinks, the bot in the
last-filled squad leaves first. A squad-bound population whose fill total is the
whole world must bind at least `squad_capacity` slots to every squad in the
spawn catalog (`bot_population_insufficient_squad_slots`), since any squad may
hold a human. Domination bots fight but do not capture
objectives. A Domination world with no entered human (for example after the
last release) does not tick its objectives or write `domination_gameplay`
records, matching Acquisition.

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
`match_profiles` array. A worker with at least one bot-filled profile also
advertises `bot_fill_v1`, and each such row adds `bot_fill_total`.
Standard rows contain:

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

### World capacity and the human cap

A world's character capacity and the number of humans it admits are separate.
Rows up to 64 characters (`team_capacity` up to 32) keep their original
meaning: every seat may be human, and the row is unchanged. A row above 64
describes a larger world that bots fill. Only such rows add two fields, and a
worker with at least one of them also advertises `world_capacity_v1`:

```json
{
  "profile_id": "synthetic-acquisition-a0-d1",
  "game_type": 1,
  "attacking_faction": 0,
  "defending_faction": 1,
  "maximum_players": 128,
  "team_capacity": 64,
  "human_capacity": 32,
  "human_team_capacity": 16
}
```

`maximum_players` and `team_capacity` still give the world's size. A larger
world admits at most `human_capacity` humans (32) and `human_team_capacity` per
side (16). A coordinator that does not understand `world_capacity_v1` must not
accept such a row. Selecting the row limits admission to the human cap; its
`configure_match` must use a `maximum_players` no larger than `human_capacity`
and per-side minima no larger than `human_team_capacity`, or it fails with
`invalid_match_limits` or `invalid_team_quorum`. Reservations and groups beyond
a side's human cap fail with `match_full` or `party_no_room`, however many bot
seats remain.

Horizon places humans on player IDs below the human cap, so humans use
Character indices 0-31. In a larger world, bots take every other index of the
256-index pool first, highest first, and borrow the human range only when the
world fills the whole pool (a 256-character world with fewer than 32 humans).
A human who joins on a borrowed index makes that bot yield it: the bot is
removed before the human's character is published, and the bot fill keeps the
world's total.

Each recipient's reliable shared-frame queue holds up to 512 frames, or
2 × characters + 256 above that (768 for a 256-character world). One
bridge reply still carries at most 128 messages and 24,000 payload bytes for
all recipients together. Frames beyond that stay queued, in order, for later
replies. The worker counts every such reply and reports them at most once a
second as `shared_replication_deferred`, with the queued frames, the deepest
queue and its bound. A recipient whose queue reaches its bound is released as
before (`replication_backlog_released`). A world above 64 characters with many
humans needs the bridge throughput work (Phase 2 of the large-lobbies plan)
before real use.

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

Selection of a profile without bots defaults to a two-human quorum with one on
each side, and `configure_match` rejects a lower total minimum or a zero
minimum on either side. Selection of a bot-filled profile defaults to one human
and no per-side minimum; `configure_match` then also accepts a total minimum of
one and zero per-side minima. This opt-in deliberately lets one human start a
match whose sides are otherwise bots. The coordinator then configures the
selected match's human minimum/capacity with `configure_match` and admits the
cohort. Late joins use the existing selected formation and its two sides.
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
claimable or sending their descriptors. An exact `release_party` for the same binding rolls back
an entirely unclaimed cohort. It may also fence a never-admitted exact cohort
with an empty ID list. Success has `status: "released"` and the ordered IDs
when applicable. It omits team and squad. The worker retains this
acknowledgment through empty-world cleanup until the next selection commits.
Use ordinary release for individual departures after publication.

The same worker also advertises `party_late_entry_v1`. Once any current
member has claimed, the selected match is running and `reserve_party`
admits the group late with the same binding (the running round's
`admission_epoch`). The worker still validates and records the whole cohort
once before installing anyone. It picks one squad on the group's side with
a free human seat for every member; bots never count as occupants. With no
such squad it splits the group across the fewest squads on that side,
emptiest first, and returns the leader's squad. A forming match keeps the
one-squad rule. After the record, bots holding a member's player ID,
overfilling the chosen squads or exceeding the side's team capacity retire
immediately; the regular bot reconciliation then keeps the profile's fill
total. Refusals name their cause: `party_no_room` (world, side or squad
room), `party_faction_mismatch` (a member's faction differs or is not in the
selected pair), `party_match_ending` (the selected match is terminal),
`party_member_conflict`, `party_formation_stale` and the existing scope
errors. A refusal changes nothing.

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
That includes the bot population and resupply providers, which come only from
the selected profile.
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
