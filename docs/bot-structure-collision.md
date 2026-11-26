# Bot movement and routing around placed structures

A mode's placed structures (Acquisition and Domination bunkers, gates,
fortifications, emplacements, Domination field hospitals and their hulks) are
not part of the static map world. Each one is collision the mode places, swaps
and removes as the structure changes state, held with its pose in the mode's
sequence collision. Bot sight and shots already stop at it
([bot-perception.md](bot-perception.md)). Bot movement and routing now do too.

## Movement

Once per world tick, before bots perceive and move, the worker takes a
snapshot of the structures standing now. It lists every placed surface that
the character movement filter row (query layer 1) collides with:

- structural sequence packs whose pack layer (and child layer 0) the row
  collides with; a pack without a qualified filter binding is solid;
- hospital hulks (layer 17) and operational hospitals (layer 22), when the
  row includes their layer.

Helicopters are left out. The snapshot is rebuilt only when the structures
change. Clones share their geometry.

Every bot body sweep tests the snapshot after the static world, and so does
every downward ground ray. That covers locomotion, step-up and fall sweeps,
`can_traverse` and its edge checks, local grid plans, idle physics and
recovery. So:

- an intact gate, a bunker wall or any other standing structure stops a bot
  the way static geometry does;
- a bunker roof or floor carries one;
- once a structure is destroyed, its intact collision is gone, and only
  what its destroyed state placed (if anything) remains.

A body a structure appears on is not trapped. A moving sweep ignores the
triangles the body already penetrates at its start, and still stops at every
other triangle.

## Routing

The navmesh is baked from the static world, so its polygons run through
structures. Polygons cannot be split at runtime. Instead, each polygon a
standing structure touches is described by where a bot can stand in it:

- the polygon is sampled on a 0.25 m grid (coarser for very large polygons);
- a cell is free when the bot body standing on the polygon surface there,
  lifted by its step height so floors and sills do not count, does not
  penetrate a structure;
- free cells form connected regions;
- each link to a neighbouring polygon records the regions touching their
  shared edge, and among them the through regions, which also touch another
  link.

A route may cross the polygon from one link (or its start point) to another
link (or its goal) only when the two share a region. Suppose several through
regions touch the entry edge, as when a wall crosses the edge itself. The
route could have arrived in any of them, so each of them must reach the exit.

The search gives a sealed crossing a prohibitive cost. Routes therefore go
around through any opening: another gate, a gap, or a bunker door. When
nothing else reaches the goal, the route is cut at the sealed crossing and
ends, as a partial route, at the point of its last reachable region nearest
the goal, in front of the gate.

The string-pulled path can still cut across a structure inside one polygon,
for example past a door jamb. Each straight leg the lifted body cannot follow
is replaced by a shortest grid path over free cells inside the corridor's
polygons, then shortened to the fewest straight legs. A leg that no grid path
replaces is kept as it was, and body physics still stops at the structure.

The gates are rebuilt with the snapshot, so a destroyed gate opens its road
again and a repaired one closes it.

## Evidence and limits

`bot_structure_obstacles` is recorded on every rebuild. It carries the number
of obstacles, the movement row, the number of gated polygons, a gate error (if
any; routes then fall back to the static mesh), and the build time.
`MAG_SIM_SNAPSHOTS=1` headless runs mark each bot whose standing body
penetrates a structure with `in_structure` (see
[headless-simulation.md](headless-simulation.md)).

Gating assumes a structure stands on the polygon surface beneath it. Free
cells are tested at that surface's height, and steering heights are
interpolated along the leg. Sabotage and development worlds have no placed
structure collision, so they are unchanged.
