# Bots attack demolition structures

Population bots used to work only on their squad's lane objective. In
Domination that is always a secure-and-hold point, so attacking bots never
touched the defenders' demolition structures: field hospitals (forward
spawns), anti-air guns, artillery, sensor arrays, motor pools, gates,
barricades and bridges. Defender bots already repaired destroyed structures
([bot-repair.md](bot-repair.md)), but nothing ever destroyed one. Attacking
squads now send a small demolition team to the structure that matters most
near their lane. This is a replacement design written for this server.

## What a structure is worth

A demolition structure is a structural objective with a demolition rule: the
side that does not own it arms a charge in one of its triggers, the charge
detonates when its fuse ends and destroys (completes) the structure, and the
owners may disarm the charge or repair the wreck later. What a destroyed
structure changes is authored data, which the mode runtime reads:

| Class | Read from | Default value |
| --- | --- | --- |
| `spawn` | a forward-spawn link that closes an owner squad spawn while the structure is destroyed, when a living owner started its life at that spawn or a human owner's squad spawns there | 100 |
| `idle_spawn` | such a link to a spawn no current owner life uses | 40 |
| `support` | completion rules that switch command abilities, or a link that opens a spawn of the attacking side | 60 |
| `vehicle_spawn` | a forward-spawn link that closes an owner vehicle source (no squad routing) | 30 |
| `other` | anything else (the structure's own geometry: gates, barricades, bridges) | 20 |

A structure takes the highest class that applies. Only Domination supplies
these links so far; elsewhere every structure is `other`.

## Teams

- Each squad keeps at most one team of up to `team_size` bots, sent to one
  structure instead of the squad's lane objective.
- A squad without a team scores every structure the other side owns and its
  own side may arm now: not armed, not destroyed and not held by a human. The
  score is the structure's value less `distance_weight` per metre between it
  and the squad's lane objective. It picks the best positive score within
  `reach_m` of the lane that fewer than `squads_per_target` other squads have
  taken.
- A side keeps at most `teams_per_side` teams. After one of its teams ends,
  the side waits `rest_ms` before it sends another, so structures fall over the
  course of a match rather than all at once.
- Members are the squad's nearest available bots. A bot is not available while
  it has an armed-objective or repair duty, a supply task, or holds the action
  of another objective (for example its lane capture). A member keeps its place
  for its life, and empty places are refilled.
- Duty priority is armed-objective duty, then repair duty, then demolition,
  then the lane.
- The team's nearest member becomes the structure's operator through the
  ordinary operator selection. It walks into an authored trigger and arms the
  charge with the same START a player sends
  ([domination-runtime.md](domination-runtime.md#bot-objective-operators)).
  The other members take attack roles around the structure.
- When the charge is armed, the structure stops being a candidate and the team
  ends. The armed-objective duties then take over: attackers guard, defenders
  converge and disarm ([bot-armed-objectives.md](bot-armed-objectives.md)). A
  team that has not armed its structure within `give_up_ms` gives it up, and
  the squad does not pick it again for `retry_ms`.
- A structure the defenders have repaired may be armed again.

What destroying a structure does is unchanged. The mode runtime applies the
authored effects: the linked forward spawn closes and its squads fall back to
their category-zero point, abilities switch, and sequences swap geometry. A
repair reopens the spawn.

## Configuration

The population file's `demolition` object; omitted fields use the defaults:

```json
"demolition": {"enabled": true, "game_types": [2], "team_size": 1,
  "squads_per_target": 1, "teams_per_side": 1, "rest_ms": 90000,
  "reach_m": 150.0, "distance_weight": 0.5,
  "values": {"spawn": 100.0, "idle_spawn": 40.0, "support": 60.0, "vehicle_spawn": 30.0,
    "other": 20.0},
  "give_up_ms": 90000, "retry_ms": 60000}
```

`"enabled": false` restores the previous behaviour, with every bot on its
lane. `game_types` lists the match game types it runs in (2 is Domination).
Teams also need `objective_policy.interactions_enabled`.

## Evidence

- `bot_demolition_team`: a squad picked a structure (`picked: true`, `score`)
  or its team ended (`picked: false`, `reason`: `unavailable` when the
  structure is armed, destroyed or held, `squad_unavailable`, `gave_up` or
  `disabled`).
- `bot_demolition_duty`: a bot joined or left a team.
- `bot_objective_action`: the operator's arming request and its result.

## Not implemented

- Attacking structures with weapons. Bots only plant charges.
- Using the structure's own geometry when it changes. Bot movement and the
  navmesh use static collision only, so a standing gate does not block bots and
  a destroyed one opens nothing for them.
- Acquisition and Sabotage values. Their structures are all `other`, and the
  default `game_types` leaves those modes off.
