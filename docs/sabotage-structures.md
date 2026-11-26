# Sabotage objective collision

Some collision on the Sabotage maps is missing from the static worlds:

- A Sabotage objective places its own objects when it enters a state. The A and B
  consoles get one console per state. The C server racks stand intact until C is
  destroyed and then turn into wreckage. A fire-and-forget dependent sequence of
  C's raises an antenna above it, and on destruction a second dependent places
  the wreck and stops the first. None of this is compiled static geometry.
- The animated props on Remote Comms are left out of its static world
  (`--exclude-animated-parented`), because parts of them are attached to moving
  nodes. Their root-attached bodies do not move.

Without this collision, bots and the navmesh treated these places as open ground.
The Sabotage maps have no gate or door entities. Their satellite dishes only turn
(presentation sequences) and are already in the static worlds.

## Projection

```sh
python3 tools/collision/retail_compiled.py ... --exclude-animated-parented \
  --animated-root-packs ROOTS_DIRECTORY        # Remote Comms only; the world is unchanged
python3 tools/extraction/project-retail-sabotage-structures.py \
  --level-archive LEVEL_PSARC [--animated-root-packs ROOTS_DIRECTORY] --output OUTPUT_DIRECTORY
```

`--animated-root-packs` writes each excluded instance's packs that have no parent
node. Each pack is written as its own world-coordinate mesh at the instance's
compiled placement, with an `index.json` (`mag_animated_root_packs_v1`). Attached
packs are still left out, and `world.json` stays byte-identical.

The projector follows the authored sequence slots of every Demolition Structural
and SecureAndHold objective on one of the three pinned Sabotage levels. It also
follows every dependent sequence those slots start. An objective whose sequences
place no collision, such as a satellite dish, is skipped. It writes
`OUTPUT_DIRECTORY/index.json` (`mag_sabotage_objective_collision_v1`) and one
neutral mesh per collision pack, named by its own SHA-256:

- Each objective gives its token, class, definition hash, the slot of each of the
  twelve authored states (`-1` means no sequence and `-2` keeps the current one),
  and, for each sequence, the packs it places itself and the dependents it starts.
- Each dependent gives its packs, the dependents it starts and the dependents it
  stops.
- `standing` lists the animated props' root packs.

Every pack is posed in world coordinates. An objective's own packs compose the
objective's authored transform, the creation event's position and rotation, and
the pack's parent-node reference pose. A dependent must be fire-and-forget and must
not use its parent's transform, so its packs compose only the event's pose. Only
rotations about the vertical axis are accepted, because those mean the same under
every Euler order. A dependent's delayed creation keeps its `start_seconds` in the
index. Presentation events (print, particle, sound, voice, animation, node
rotation) are dropped, and any other event class is refused.

The output holds converted game geometry, so keep it in private bundles.

## Runtime

A match profile row opts in with `sabotage_structures` (the index path). The row
also needs a Sabotage objective catalog and a collision world, and every objective
in the index must bind to the catalog by token. Meshes load from the index's
directory and are checked against their own hash and their packfile provenance.
`--validate-catalogs` reports `sabotage_structures_validated`.

On every world tick, before bots move, the engine compares each bound objective's
current state with the slot it has placed:

- When the state's slot differs, the objects the previous sequence placed itself
  are removed and the new sequence's are placed.
- Dependents the new sequence starts (and the dependents they start) are placed,
  and the dependents they stop are removed. A dependent outlives the sequence that
  started it, so C's wreck replaces the antenna only through its stop.
- A delayed creation is placed at once, in its end state.
- When the objective world starts a fresh round, everything placed is removed and
  then placed again from the new states.
- Standing packs are placed once for the whole match.

Each change is recorded as `sabotage_structures`, with `object_index`, `state`, `slot` and
`packs` (the number of packs placed).

The packs go into a `SequenceCollisionState`, the same one Acquisition and Domination structures use. So bot
body sweeps, ground rays and navmesh route gating
([`bot_structure_collision`](../gameplay/src/bot_structure_collision.rs)) respect
them. Each pack keeps its collision tag, and movement collides with it only when
the character movement row does. That row is the profile's filter row 1 when
collision filter rows are loaded, and otherwise the navmesh's
`character_movement_layers`.

Profiles without `sabotage_structures` are unchanged. Shots, sight and blasts do
not yet test these packs.
