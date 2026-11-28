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

## Ranking toward the current objective

A squad ranks its points when it defends the objective it works on now, or
when that objective is not its lane objective: its human leader's open FRAGO,
or once its lane is exhausted the nearest objective of its team
([bot-squad-orders.md](bot-squad-orders.md)). The lane objective is the
Sabotage progression or catalog assignment, the Acquisition catalog
assignment, or the Domination runtime assignment, or while the lane's
defences stand the defence the squad breaks first
([bot-breach-order.md](bot-breach-order.md)). A squad defends an objective its
own team owns. Before the first bot tick, and before lanes exist, the lane
objective or the authored initial assignment stands in. An attacking squad on
its lane keeps the active-spawn order above, which already follows the spawns
the mode opens for it.

Each life of a ranking squad, at its first life and at every respawn wave,
ranks the points the squad may use now by the length of a complete navmesh
route (walking or certified drops) from the point to that objective. The
candidates are:

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
or field hospital above, any point the squad gained at runtime (a forward
spawn the match opened for it), and the squad's runtime default once the mode
moved it off the authored default (an Acquisition LALO drop zone, which is
active only while its platoon's anti-aircraft is down). The squad's other
points follow. Within each group the shorter route goes first, so a standing
forward spawn is used even when a rear point is nearer the objective (an
Acquisition squad's prototype sits in its rear base, behind its field
hospital). Once the hospital falls, its point goes inactive and the rear
points take over.

A point whose placements do not route to the objective is skipped and reported
once. Points within 20 m of their group's shortest route are rotated per bot,
so a squad spreads over equally forward points. A human squad member's shared
spawn, once it differs from the authored default, goes first when it routes.
The population slot stays the final fallback.

When no point routes to the current objective (a FRAGO off the walking
network, say), a defended lane objective is ranked toward instead: the lane
defence the squad holds, then the mode's own lane objective. When nothing
routes to those either, or the squad attacks its lane, the active-spawn order
above applies. A new FRAGO ranks toward the new objective at the next life;
route qualifications are cached per point and objective, so changing back
reuses them.

The population file's `squad_orders.spawn_at_current_objective` (default
`true`) controls this. `false` restores the earlier rule: only defending
squads (whose lane objective their own team owns) rank their points, toward
the lane objective, ignoring the FRAGO and without the moved-default forward
spawn.

Routes respect the structures standing now
([bot-structure-collision.md](bot-structure-collision.md)), so every cached
qualification (and its receipt) is renewed whenever the navmesh installs gates
for a new structure snapshot: a destroyed gate opens a way, a repaired one
closes it.

## Route checks run off the world tick

Proving that a point routes can be expensive. A point that does not route
searches the navmesh once per placement tried, and on a large map that takes
tens of milliseconds each. A side's first wave asks for dozens of points at
once. Run inline, a defender wave on Baku held the world for 7 s on a laptop
and about 16 s on a cluster node: longer than the 10 s world-stream frame
deadline, so every client was dropped.

So the server never routes for a spawn choice inside the world tick:

- A point qualification or defender lane qualification that is not cached is
  queued to a worker thread. The tick only gathers the point's placements
  (ground probes), once per point and collision world: placements depend
  only on static collision, the authored point and a hospital's authored
  exits, so a structure gate change qualifies the routes again but keeps the
  placements. Until the answer arrives the point counts as pending: it
  is skipped for this life, is not cached and has no receipt, and the bot uses
  its next point or its population slot. The answer and its receipt are
  stored on a later tick, provided the navmesh and its structure gates are
  unchanged. Otherwise the point is queued again.
- A structure gate change does not discard answers. Every point and lane
  qualification from earlier gates, usable or not, stays in use, marked
  stale, and is routed again at most once every 15 s per key, by the
  prefetch or by a spawn choice that uses it. Gates change between almost
  every wave of a Domination match (breaching and repairs), and routing every
  qualification again after each change kept the worker busy, mostly with
  lane checks that fail and search the whole navmesh. Once world ticks took
  the navmesh first, the worker answered about a tenth of the lane checks it
  had before. A stale answer can be up to about 15 s out of date after a gate
  change: a life may use a point a structure has just walled off, or skip one
  that has just opened. Routes already queued when the gates change still
  answer, through the gates they were planned with, and their answers are
  kept the same way instead of being dropped and routed once more. Synthetic
  tests that route inline still route again at once.
- The defender lane points every population slot will ask for are queued
  ahead, at most once a second: while humans wait for their side's first
  wave, so the first defender lives normally already have their forward
  points, and again after structure gate changes. One pass probes the
  placements of at most two points it has not probed before (a parachute
  point's drop line searches 400 m down per jumper, a few milliseconds per
  point); a pass that meets more stops there and continues on the next tick.
  Before this, a 256-player Domination simulation probed about 45 defender
  points in one tick (200–360 ms), and again after every gate change.
- The same pass queues the point routes of squads without ranked goals
  (attackers on their lane): every point their active-spawn order tries
  before the population slot, such as an open parachute drop zone or a human
  leader's squad spawn, under the same probe budget. Before, these were
  queued only when a life asked at its wave. In live Domination the answer
  was still pending then, and the next structure gate change cleared it
  again, so attacker lives used their population slot at every wave and
  never the open drop zones.
- The check behind squad fill order (whether a squad's population slots
  reach any objective; see `bot_population_unroutable_squads`) runs once per
  population on its own worker thread, started at the same time. Until it
  answers, squads rank as routable, as they do without navigation data.

The workers and the tick search the same navmesh at once. The loaded Detour
mesh is only read after it loads, so every query takes its own native query
context (Detour's node pools and open list, about 0.6 MiB at 16,384 search
nodes), reusing an idle one or making one, and a world tick never waits for a
worker search. Structure gates are immutable sets: the gate worker builds a
new set and swaps it in whole, and each route uses the set it started with.

Before that (0.19.0), the workers and the tick shared one native query
object, and a worker search held it for the whole search. A world tick locks
it at least once per moving bot, so every one of those locks could wait for a
whole worker search. Late in a live 256-character Domination match
(2026-11-28) the spawn-route worker had used 212 s of CPU, and the world spent
up to 28% of the time in ticks of 0.5–1 s; every client event, hits included,
waited behind them. A turn order fixed the stalls: while a world tick runs, a
worker waits before it starts a navmesh query (at most 200 ms, so a world
that never stops ticking cannot starve it). But workers then searched only
between ticks, and a 256-character simulation qualified about a tenth as many
lane spawns. `MAG_NAVMESH_SHARED_QUERY=1` restores that one shared query
object and the turn order, for comparison.

Synthetic unit tests keep the routes inline so their expectations are
unchanged. The headless simulation queues as live, then waits for the answers
outside its timed phases, so a run stays deterministic. With
`MAG_SIM_CLOCK=wall` (see [headless-simulation.md](headless-simulation.md))
it lets the workers run beside the tick instead, as live.

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

A bot's landing spots are the first supporting surface below each authored
drop-line position (the line human parachutists are spaced along), then a
small ring around the first landing.

A bot life that arrives at a drop zone comes down under its parachute, as a
human does (`gameplay/src/bot_parachute.rs`):

- Its respawn carries character state 17 (the state of a human parachute
  respawn) and the drop zone's parachute yaw. Its position is straight above
  its landing spot, at the drop zone's authored airborne height.
- It descends at a steady 6.4 m/s. Human drops in live Domination came down
  39-41 m in 5.8-6.9 s (5.9-6.7 m/s). Every 250 ms it publishes its height
  to the recipients that hold its snapshot. A recipient that takes its
  snapshot mid-descent (a joiner, or one waiting for a paced snapshot batch)
  gets state 17 and the current height.
- When the descent time is up it publishes standing state 1 on its landing
  spot, and its life goes on from there.
- While in the air it does not walk, fire, operate objectives, take cover or
  react to threats. Other bots neither see nor aim at it. Its body is reserved
  at its landing spot from the start, so no other life lands there. Damage
  still applies. A bot hit in the air goes down or dies on its landing spot,
  and its next life at an open drop zone descends again.
- The descent is vertical: bots do not steer the way human parachutists drift
  forward. A drop zone less than 1 m above the landing spot is arrived at
  standing.

`bot_respawn` adds `descent_ms`, the time to touchdown, and
`bot_parachute_landed` (`position`, `descent_ms`) marks the touchdown.

## Scope

Populations with a fixed `regional_collision` scope or a `regional_team` keep
their bound slots. Slots without a `squad`/`source_spawn` binding also keep
their slot.

## Evidence receipts

- `bot_population_spawn` and `bot_respawn` carry `spawn_point_index`, `arrival`
  (`population_slot`, `authored_point` or `parachute_landing`) and `position`.
- `bot_spawn_point_qualified` (`placements`) or `bot_spawn_point_unusable`
  (`reason`) is written once per point and population source spawn.
- Ranked lives: `bot_lane_spawn_qualified` (`route_m`, `placements`,
  `arrival`) or `bot_lane_spawn_unusable` (`reason`), once per point,
  `objective` and whether the point is the bot's own population slot. Arrivals
  add `hospital_exit` and `field_hospital`. `bot_population_spawn` and
  `bot_respawn` add the `objective` the life was ranked toward and the chosen
  point's `objective_route_m` (both `null` when unranked).
