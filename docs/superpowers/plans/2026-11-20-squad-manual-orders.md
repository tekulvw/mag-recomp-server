# Manual squad orders implementation plan

User continuation: complete OIC after platoon, then continue gameplay TODOs.
The preceding source increment merged as PR90. This feature starts from freshly
fetched main d9638b6 in the existing isolated worktree; native SDK mains were
also fetched and inspected. Releases remain manual. No live run or deployment.

## Design

Use authenticated squad-local held leadership for retail tagged-object and
waypoint requests. The client supplies only the target or XYZ plus clear bit.
Reuse existing Acquisition/Domination assignment refresh and reliable squad
publication. Keep manual tag distinct from the effective tagged object and
primary objective. Waypoints retain coordinates when cleared. One round-owned
timestamp gates successful sets; clears and leader removal preserve it.
External projected character gameplay settings supply the cooldown. Missing
configuration must deny rather than invent tuning or enable incomplete orders.
Full target IDs resolve against actual current inventories before acceptance.
Original inputs and native evidence remain private.

## Tasks and ownership

1. Protocol worker: typed0206/021a decoding, exact payload handling and type8
   fields13/14/15 encoding using the existing49-bit position codec. Write
   meaningful failing wire tests before implementation.
2. Mode worker: expose cloned single-squad manual assignment refresh for
   Acquisition and Domination. Publish effective tag changes alongside primary
   assignments without bypassing native skip gates. Preserve rollback and current
   automatic assignment behavior.
3. Configuration worker: neutral source-bound order cooldown projection and
   typed round state/gate tests. Preserve wrapping arithmetic, shared stamp,
   clear behavior and retained waypoint coordinates.
4. Root: Engine configuration, authenticated event candidate, concrete target
   resolver, held-role removal cleanup, round reset and reliable late/current
   recipient publication. Connected tests cover unauthorized identity, shared
   cooldown, clear/replacement, automatic fallback, record failure and late join.
5. Run critical checks, source profiles and public boundary. Integrate the
   focused PR when green and checkpoint private proof/TODO. Reward parity and
   durable leadership advancement follow separately; assigning an order grants
   no immediate XP.

## Pinned interfaces

`protocol::TaggedObject { target_id: u32 }`;
`protocol::SquadWaypoint { position: CharacterPosition, clear: bool }`.
Runtime authority derives the squad and reservation-bound leader from the
connection. Set gate uses the verified unsigned native arithmetic and the
projected cooldown; full-ID inventory admission precedes installation. Successful
sets stamp server milliseconds. Initial/live fields are effective tag13,
waypoint position14 and waypoint active15; existing primary field12 is separate.
Reliable baselines install only after recording, and a full FIFO defers updates.
