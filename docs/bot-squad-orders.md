# Bots follow their squad's FRAGO and lane

Population bots used to work only on their squad's lane objective, and they
ignored the squad leader's tagged objective (FRAGO). A live Acquisition match
showed the result. A human squad leader tagged a defender structure outside
the lane, and the squad's six bots kept working on the lane capture until the
structure fell. Squads without a human leader carried no tag at all, so a
human member of a bot-led squad saw no order. This is a replacement design
written for this server.

## What retail decides

The native squad assignment refresh keeps two fields per squad. The primary
objective is the first valid entry of the squad's authored objective list
(its lane). The tagged object is the leader's manual tag, or else the first
valid entry of the squad's authored FRAGO list
([squad-manual-orders.md](squad-manual-orders.md)). Retail authors empty FRAGO
lists in every Acquisition and Domination map, so in retail only a human squad
leader's tag ever names a FRAGO objective. Retail has no bots and no automatic
leader orders. The automatic tag below is this server's own policy.

## Which objective a squad works on

Each squad with bots works on one objective, chosen every bot tick:

1. **The FRAGO.** A squad whose tag belongs to a human leader works on the
   tagged objective while it is open for the squad's team. It may be inside or
   outside the lane.
2. **The lane.** Otherwise the squad works on its lane objective: the native
   primary in Domination, and in Acquisition and Sabotage the first authored
   objective that is not inactive, completed or locked. A completed objective
   therefore moves the squad on to the next uncompleted objective of its lane.
   A Domination lane stays on its secured point while its next points are
   still inactive; holding that point is the native primary.
   While the lane's defences stand, the lane objective is the defence the
   squad breaks first ([bot-breach-order.md](bot-breach-order.md)).
3. **The nearest active objective.** A squad whose lane is exhausted helps the
   objective, among those other squads of its team work on, nearest to the
   mean position of its living bots. Ties go to the lower objective index.

An objective is open for a team while the team has an action there (arm,
capture, secure, defuse, deactivate or retake), or while it is neither
inactive, completed nor locked (the team's own point to defend, or a charge
that is already armed). A completed or locked FRAGO is never followed, and its
squad returns to its lane. Tags that name a vehicle, helicopter or hospital,
or an objective without a place bots can reach, are not followed either: the
squad stays on its lane.

Duties still come first for the bots that hold them, in their existing order:
armed-objective duties and blast evacuation, repair, demolition teams, then
the squad's objective. Resupply runs pause objective work as before. When a
duty ends, the bot returns to its squad's objective. Bot lives of a squad on
a FRAGO, or defending its objective, start at the points nearest this
objective by route ([bot-active-spawns.md](bot-active-spawns.md)). Demolition team scoring still
measures from the lane objective.

## Automatic orders for bot-led squads

A squad without a human leader keeps its tag on the objective its bots work
on. When that objective completes and the squad moves on, the tag moves with
it. The tag goes out through the ordinary squad order publication, so human
members see it as their squad's FRAGO, and FRAGO rewards use it as they use a
human tag. An automatic tag does not stamp the shared order gate and needs no
leader authority. It is written only while the match is playing and only when
squad orders are configured (`--squad-orders`).

A human who becomes the squad leader inherits the automatic tag, and the
server keeps moving it with the lane. Once the leader sets a tag of their
own, or clears it, the order belongs to the leader until the leader role
changes hands again.

## Configuration

The population file's `squad_orders` object. Omitted fields use the defaults:

```json
"squad_orders": {"follow_orders": true, "automatic_orders": true,
  "spawn_at_current_objective": true}
```

- `"follow_orders": false` restores the previous lane-only selection, without
  the nearest-objective fallback.
- `"automatic_orders": false` leaves bot-led squads untagged, as before.
- `"spawn_at_current_objective": false` ranks only defenders' spawns, toward
  their lane objective, ignoring the FRAGO, as before
  ([bot-active-spawns.md](bot-active-spawns.md)).

## Evidence

- `bot_squad_orders`: automatic tags written this tick, per squad:
  `previous_tag`, `tag`, the `objective`, its `source` (`lane`,
  `nearest_lane` or `frago`) and whether it `applied` (`error` when the mode
  refused it). A refused tag is not requested again for that squad until
  another tag applies.
- `bot_objective_task` and `bot_objective_waiting`: the bot's `objective`, as
  before.
- `squad_orders_published`: the tag that reached each recipient.

Retail spawn choice does not read the tag. A human squad's shared spawn is
the leader's choice among the squad's active points, and the mode moves the
squad default only through objective effects (Sabotage forward links,
Domination and Acquisition spawn effects)
([worker-match-profiles.md](worker-match-profiles.md#shared-squad-spawn-selection)).
Ranking bot spawns toward the FRAGO is this server's own policy.

## Not implemented

- Following tagged vehicles, helicopters or hospitals.
- Waypoints. Bots do not walk to the leader's map waypoint.
- Platoon and OIC orders. These have no squad override natively either.
- Bot squad leaders choosing a FRAGO other than their lane objective.
