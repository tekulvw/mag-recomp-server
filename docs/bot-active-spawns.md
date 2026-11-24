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
helicopter passenger points and unsupported points are never used by bots.

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
