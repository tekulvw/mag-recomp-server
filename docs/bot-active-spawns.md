# Bot spawns follow the active spawn set

Population bots used to start every life at their fixed population slot, so
spawns that open during a match (Sabotage forward parachute insertions,
Domination and Acquisition spawn changes) were used only by humans. Bots now
choose where each life starts from their squad's current spawn choices, under
the same rules humans are held to.

## Policy

Evaluated at every bot life start (population allocation and each respawn
wave). The result is deterministic for identical world state.

1. The squad's current spawn: a human member's shared squad spawn, else the
   runtime squad default. It counts only once it differs from the squad's
   authored default; until then the bot keeps its population source spawn, so
   match starts are unchanged.
2. Other points the squad gained at runtime, most recently added first.
3. The bot's population source spawn.
4. The squad's remaining authored points, in authored order.

A point is considered only when the squad may use it now, it is active, and its
kind is insertion, alternate, on-foot or parachute. Bunker, vehicle and
helicopter passenger points and unsupported points are never used by bots,
except for the defender hospitals below.

## Defending squads

A squad defends when the objective its bots work on (its lane: the Sabotage
progression or catalog assignment, the Acquisition catalog assignment, or the
Domination runtime assignment; before lanes exist, the authored initial
assignment) belongs to its own team. Defending bots do not keep their
population slot just because the squad's default is unchanged. Instead each
life ranks every point the squad may use now by the length of a complete
navmesh route (walking or certified drops) from the point to the lane
objective:

- the squad's usable points above, including the population source spawn;
- the squad's hospital (bunker passenger point) while its point is active and
  the hospital is operational and belongs to the squad's side. This covers
  every Domination hospital and an Acquisition field hospital whose profile
  binds its emplacement (see
  [Acquisition field hospitals](acquisition-gameplay.md#field-hospitals)).
  Bots arrive on foot at the hospital's authored exits, or a 1 m cell further
  out, instead of taking a seat;
- in Acquisition, the field hospital the structural objective routes the
  squad to while it stands (the squad's runtime default), when the spawn
  catalog still leaves it unsupported. Players fall back to the squad's
  category-zero point there; bots stand on routed ground within 6 m of the
  authored point. A catalog that makes the field hospitals selectable needs no
  fallback: the structural spawn scope then leaves them alone and they rank
  like any other point. A catalog that binds them as bunker passenger points
  uses the hospital rule above.

Forward spawns come first whenever they route to the objective: the hospital
or field hospital above, and any point the squad gained at runtime (a forward
spawn the match opened for it). The squad's other points follow. Within each
group the shorter route goes first, so a standing forward spawn is used even
when a rear point is nearer the objective (an Acquisition squad's prototype
sits in its rear base, behind its field hospital). Once the hospital falls,
its point goes inactive and the rear points take over.

A point whose placements do not route to the objective is skipped and reported
once. Points within 20 m of their group's shortest route are rotated per bot,
so a squad spreads over equally forward points. A human squad member's shared
spawn, once it differs from the authored default, goes first when it routes.
The population slot stays the final fallback. Attacking squads keep the order
above.

Routes respect the structures standing now
([bot-structure-collision.md](bot-structure-collision.md)), so every cached
qualification (and its receipt) is renewed whenever the navmesh installs gates
for a new structure snapshot: a destroyed gate opens a way, a repaired one
closes it.

## First lives

Bots of a side take their first lives at that side's first spawn wave. The
first human of a side to confirm a spawn starts the side's wave clock at its
authored initial spawn delay, and the side's bots wait for that wave (or for
one of its humans to be alive). A side with no human counts as having
confirmed when the other side's first human did, so it keeps its own initial
delay: with the retail delays (attackers 35 s, defenders 20 s) defending bots
come in 15 s before attacking ones, as retail defenders do, instead of with
the first human's side. Their wave clock starts at that first wave. A side
that has started refills at any time. Without a confirmed human the
population starts as before, when the first human reaches on foot, with every
side at once.

The source spawn always uses the bot's own population slot and waits while a
body occupies it, as before. Any other point uses placements that are:

- grounded and inside verified collision coverage (the bot body settles there);
- on the walking navmesh, in a component that a complete route from the bot's
  population slot reaches (spawns are often left through one-way drops, so
  routing back is not required);
- at least 0.71 m apart and free of other bodies at the moment of use.

The first free placement wins; otherwise the next point is tried. A point with
no such placement is skipped and reported once. If nothing else qualifies the
population slot is the fallback.

## Parachute points

The server has no airborne descent model for bot bodies. Bots arrive landed on
the first supporting surface below each authored drop-line position (the line
human parachutists are spaced along), then on a small ring around the first
landing.

## Scope

Populations with a fixed `regional_collision` scope or a `regional_team` keep
their bound slots. Slots without a `squad`/`source_spawn` binding also keep
their slot.

## Evidence receipts

- `bot_population_spawn` and `bot_respawn` carry `spawn_point_index`, `arrival`
  (`population_slot`, `authored_point` or `parachute_landing`) and `position`.
- `bot_spawn_point_qualified` (`placements`) or `bot_spawn_point_unusable`
  (`reason`) is written once per point and population source spawn.
- Defenders: `bot_lane_spawn_qualified` (`route_m`, `placements`, `arrival`)
  or `bot_lane_spawn_unusable` (`reason`), once per point, lane objective and
  whether the point is the bot's own population slot. Arrivals add
  `hospital_exit` and `field_hospital`.
