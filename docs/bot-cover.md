# Bot cover

World-owned bots that are shot at or meet an enemy can move to nearby
geometry that blocks the enemy's line of fire, fight from it and leave it when
it stops protecting them or when their objective needs them. Retail had no bot
AI; this is a replacement design written for this server. It is off by
default (see [Configuration](#configuration)).

## Cover spots

Cover is found once per collision world and navmesh, at load, on a worker
thread. Nothing is bundled.

- Every navmesh polygon edge that no link covers is a walkable boundary: a
  wall, a ledge or the edge of the map. Boundary edges are sampled every
  1.25 m, just inside the walkable surface.
- From each sample the static world is probed horizontally, away from the
  walkable side, up to 1 m, on the projectile query row (what bullets stop
  at), at five heights: 0.55 m, 0.75 m, 1.05 m, 1.45 m and 1.75 m above the
  feet. A hit only counts on a steep (wall-like) surface.
- A **high** spot is blocked at every height: it hides a standing body. A
  **low** spot is blocked at the two lowest heights and open above 1 m: it
  hides a crouched body, and a standing body fires over it. Anything else (a
  railing, a window, open ground, a drop-off) is not cover.
- A high spot needs a **peek spot**: a step 0.8 m to its left or right that is
  on the same walkable surface, reachable without crossing the cover, and
  open at standing height past the cover's edge. A high spot without one
  hides but cannot fight, so it is dropped.
- Spots outside verified physics regions are dropped, and nearby duplicates
  (same metre cell and facing) are merged.

Spots are indexed on an 8 m grid. The map is cached in the process by
collision world and navmesh geometry hash, so selecting a profile again
reuses it.

Placed structures (Acquisition and Domination bunkers, gates and other
objective structures; see [bot-structure-collision.md](bot-structure-collision.md))
also give spots. Whenever the standing structures change, a worker samples
around each structure's footprint, projects the samples onto the navmesh,
drops those inside a structure, and probes toward the structure against the
structure and the static world, as above.

`bot_cover_map` (spots, high spots, boundary edges, samples, build time) and
`bot_cover_structure_spots` record each build.

## When a bot takes cover

A bot considers cover when it reacts to a threat
([bot-perception.md](bot-perception.md)), in either of two cases:

- **walking:** perception has paused its ordinary route (it was hit, saw an
  enemy, or a bullet passed close);
- **standing:** its weapon holds an enemy, or it was hit or nearly hit.

It must also be free of every task that owns its body: a passage, queue,
holding spot, traffic yield, supply task, recovery or objective interaction,
and not grounded on another character. An attacker within 15 m of its
destination keeps pushing instead, and a bot that just left cover waits 5 s
before taking cover again (a hit lets a walking bot do it at once).

The threat is the enemy its weapon holds, else the one perception reacted to
(at its current position), else the point perception noticed.

## Choosing a spot

Spots within 16 m are filtered, then ranked by distance:

- the spot's protected direction must face the threat (within 60°), and the
  threat must be at least 5 m away;
- another bot must not hold, walk to or stand within 1.5 m of it;
- an attacker's spot may be at most 3 m farther from its destination than
  the bot is now, and nearer spots toward the destination rank better; a
  defender's spot must be within 18 m of its post;
- low spots are used only when crouching is on.

The four best are checked with the same sight-line test the weapon and
perception use (static world inside verified coverage, plus the mode's
placed structures, on the projectile row; see
[bot-structure-collision.md](bot-structure-collision.md)):

- **hidden:** the threat's torso cannot see the bot's torso at the spot,
  crouched behind low cover;
- **can fire back:** from a peek spot (high cover), or standing at the spot
  (low cover), the bot's torso sees the threat.

The first spot that passes both is taken if a complete walking route of at
most 24 m reaches it, without a drop. The bot sprints there. Its objective
task keeps its destination; its route is replanned when it leaves cover.

## Fighting from cover

Once there the bot cycles:

- **Hidden** for about 1 s (± 0.4 s per bot and cycle): behind high cover it
  stands at the spot; behind low cover it crouches. It reloads here, and a
  hurt bot (below 75% health) stays down up to two more phases while its
  self-healing works ([bot-healing.md](bot-healing.md)).
- **Exposed** for about 2.2 s: it steps out to the peek spot (high cover) or
  stands up (low cover) and the weapon controller fights as for any standing
  bot. It goes back early when its magazine runs dry.

While hidden it checks every 500 ms that the threat still cannot see it.

It leaves cover when:

- the threat is gone: nothing has stimulated it for perception's sight
  memory (4 s) (`threat_cleared`);
- the threat can see it while hidden: it was flanked (`flanked`), or the
  structure it used was destroyed or changed (`exposed_structure`). It
  searches again at once;
- objective planning gives it a new duty, an operator selection or a new
  objective, including a blast evacuation (`objective_changed`);
- any task above takes its body (`preempted`);
- it stayed 6 s (attackers) or 20 s (defenders) (`dwell`);
- a move into or out of cover fails (`move_failed`, `peek_unreachable`,
  `hide_unreachable`).

On leaving, a crouched bot stands, the route resumes, and perception does not
pause the route again for 6 s, except for a hit.

### Squads

- When an attacker's dwell ends while the threat is still live, it **bounds**:
  it takes the next spot that is at least 4 m closer to its destination
  (`bot_cover_bound`), or resumes its route when there is none.
- It waits up to 1.5 s to go while a squadmate within 25 m is in cover but
  none of them is firing; it goes at once when a squadmate is exposed and
  firing (covering it).
- Hidden and exposed phases are offset per bot, so squadmates sharing cover
  do not all peek at once.

## Crouching

With `crouch` on, a bot behind low cover crouches. The server publishes its
Character state with stance 1 (crouched) in an owner delta, and stance 0 when
it stands; late joiners get the current stance in the bot's snapshot. The
client's stance codes are 0 standing, 1 crouched, 2 prone and 3 changing
stance: a lab capture of a human client tapping and holding the stance
control sent 1 when crouched and 2 when prone, and the retail client's own
stance enumeration has the same order.

While crouched, the bot's body is 1.2 m tall: enemy bots aim at its torso at
0.72 m, its hit box ends at 1.2 m, and its own muzzle and eyes are at 0.72 m.
What clients see therefore matches what the server tests. Human Characters
are still treated as standing by bot weapons and sight.

## Cost

Cover runs once per world tick, after perception and before bot movement
(phase `cover` in headless timing). Per tick it spends at most 24 rays,
32,768 geometry work units and 4 route queries, shared by every bot, in
round-robin order; a bot that does not fit waits for a later tick. Episodes
are advanced first; a hidden bot spends one ray every 500 ms, and a step out
or back is one short route query. Searching scans the grid cells within
16 m, a distance check per spot.

`bot_cover_summary` records every 10 s: bots in each state, spot counts,
leave reasons and totals (episodes, arrivals, bounds, peeks, searches,
searches without a spot, rays, routes, rejected routes, deferred work, time
hidden, exposed and moving, busy time and the slowest tick).

## Evidence

- `bot_cover_start`: bot, life, trigger (`walking` or `standing`), attacker,
  position, spot (feet, protected direction, high or low, peek spots,
  structure), fire position, threat and enemy.
- `bot_cover_bound`: from and to.
- `bot_cover_end`: reason, state, attacker, structure, peek cycles, bounds,
  time in cover and in the episode.

## Configuration

The population file's `cover` object; omitted fields use the defaults:

```json
{
  "enabled": false,
  "crouch": false,
  "search_radius_m": 16,
  "max_route_m": 24,
  "min_threat_distance_m": 5,
  "facing_degrees": 60,
  "attacker_goal_slack_m": 3,
  "attacker_push_m": 15,
  "bound_gain_m": 4,
  "attacker_dwell_ms": 6000,
  "defender_dwell_ms": 20000,
  "defender_leash_m": 18,
  "hide_ms": 1000,
  "expose_ms": 2200,
  "jitter_ms": 400,
  "recheck_ms": 500,
  "retry_ms": 1500,
  "reengage_cooldown_ms": 5000,
  "perception_calm_ms": 6000,
  "move_timeout_ms": 8000,
  "spacing_m": 1.5,
  "max_candidates": 4,
  "max_rays_per_tick": 24,
  "geometry_work_per_tick": 32768,
  "max_routes_per_tick": 4
}
```

`mag-gameplay --bot-cover on|crouch|off` overrides every population,
including match-profile populations: `on` enables cover, `crouch` enables it
with low cover and crouching, and `off` disables it. Cover also stays inactive
whenever perception is inactive. Fixed-goal populations walk to their slot
goal again after cover; objective populations replan.

With cover off, no bot changes stance, body height, movement or receipts.

## Not implemented

- Crouched or prone human Characters: bots aim at a human's standing torso.
- Crouch walking, prone and leaning: a bot stands to move, crouches only
  while hidden behind low cover, and steps sideways to fire past high cover.
- Suppressive fire at a hidden enemy's position; bounding is coordinated only
  by waiting for a squadmate's fire.
