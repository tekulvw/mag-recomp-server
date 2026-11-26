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

A structure can enclose a room that no opening leaves, such as a closed
field-hospital bunker. A life started on foot inside one (a hospital spawn
whose profile has no bunker seating) would otherwise be trapped. Each
structure's enclosed rooms are found once per mesh: these are the cells of
its base level where a standing body fits, but which cannot reach the mesh's
outer bounds without crossing it. Each room also takes in its walls, up to
1.5 m from its free space. A body inside one is not held by that structure:
it walks out through the walls, and its routes are planned on the static mesh
alone. Bodies outside still cannot enter.

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

That rule alone cannot tell which side of a wall crossing the entry edge a
route is on, so it would also seal a route running beside such a wall (past
a bunker whose shell crosses a polygon edge, say). The search is therefore
first run with the ambiguous entry allowed on either side, and its corridor is
then followed region by region: the route starts in the regions at its start,
crosses each shared edge into the regions whose free cells meet its own along
that edge (an ungated polygon is one region), and must end in a region holding
its goal. A corridor that gets through is kept. A crossing it cannot pass is
refused to the next search, up to four searches; a crossing sealed outright
ends them early. Only then is the conservative rule above used.

The string-pulled path can still cut across a structure inside one polygon,
for example past a door jamb. Each straight leg the lifted body cannot follow
is replaced by a shortest grid path over free cells inside the corridor's
polygons, then shortened to the fewest straight legs. Legs keep 0.3 m more
room from a structure than the body needs, so bots crossing next to a wall
can pass each other. A passage narrower than that is still used at the
body's own width. A leg that no grid path
replaces is kept as it was, and body physics still stops at the structure.

The gates are rebuilt with the snapshot, so a destroyed gate opens its road
again and a repaired one closes it. A polygon whose structures are unchanged
(same placement, mesh and pose) keeps its previous gate.

## Cost

The world tick only takes the new snapshot, which it indexes on a 16 m
horizontal grid. The enclosed-room analysis and the polygon gates are built
on a worker thread. Until they are installed, routes keep the previous gates
and newly placed structures show no rooms. Sweeps test each structure only
where the swept body is near its bounds, in legs of at most 2 m.

## Evidence and limits

`bot_structure_obstacles` is recorded on every new snapshot, with each
obstacle's label and world bounds, the movement row and the time taken on the
tick. `bot_structure_analysis` is recorded when the worker finishes, with how
many meshes were analysed for rooms, how many polygons were gated and of them
reused, any gate error (routes then fall back to the static mesh), and the
time taken. `MAG_SIM_SNAPSHOTS=1` headless runs mark each bot whose standing
body penetrates a structure with `in_structure`, and each bot inside an
enclosed room or its walls with `leaving_room` (see
[headless-simulation.md](headless-simulation.md)).

Gating assumes a structure stands on the polygon surface beneath it. Free
cells are tested at that surface's height, and steering heights are
interpolated along the leg. Sabotage and development worlds have no placed
structure collision, so they are unchanged.
