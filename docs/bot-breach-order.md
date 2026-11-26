# Attacking bots break a lane's defences before its capture

Attacking population bots used to go straight for their lane's capture. In a
live Acquisition match on 0.17.1, every attacker objective task went to the two
stage-one vehicle captures. Not one went to a gate, bunker or anti-air gun,
and no structure fell. Domination attackers went to their secure-and-hold
point too. There, only the small demolition teams
([bot-demolition.md](bot-demolition.md)) touched the structures in front of
it. Bots now break each lane's defences first, outermost first, and then take
the capture. This is a replacement design written for this server.

## What retail decides

Neither mode locks a capture behind the defenders' structures:

- In Acquisition, each platoon's first vehicle capture is active from mission
  start. The authored controller links only captures and deliveries: a
  delivered first vehicle locks its chain and enables the second capture.
  Squad objective lists name only the vehicle captures and deliveries.
- In Domination, the controller links points to stage tokens, and each squad's
  native primary is a point.

The structures still decide how a lane plays:

- Gates and barricades are standing collision on the approach. Bot routing
  treats them as walls ([bot-structure-collision.md](bot-structure-collision.md)).
- A bunker is a defenders' forward spawn (in Acquisition a field hospital with
  its turret). Destroying it closes that spawn.
- Destroying an anti-air structure opens the attacking platoon's parachute
  drop zones and switches on its air strikes. A repair closes them again
  ([acquisition-gameplay.md](acquisition-gameplay.md#parachute-drops)).

Mortars, sensors, other command-ability structures and motor pools sit
beside or behind the captures. They help, but they are not in the way.

## A lane and its defences

- **Acquisition.** A lane is one attacking platoon's pair of vehicle chains.
  It is seen from the platoon's first delivery point, which is on the
  attackers' side.
- **Domination.** A lane is a point that attacking squads with bots share as
  their native primary while they may capture it. It is seen from the mean of
  those squads' home spawns. A secured point is held, not breached.

A lane's defences are the defending side's demolition structures that stand
nearer the lane's origin than its nearest capture, within the mode's reach of
a capture. Of those, only these count:

| Kind | Read from | Lane |
| --- | --- | --- |
| bunker | the structure is the forward spawn of an on-foot squad point of the owners | the lane of the owner squad that spawns there |
| anti-air | its destruction opens spawn points of the other side (Acquisition: its authored anti-air hook and the drop zones it opens) | the lane of the attacking squads whose drop zones it opens, else the nearest |
| obstacle | no authored links: only its own geometry (gates, barricades) | the lane with the nearest capture |

Structures with any other link (mortar, sensor, command abilities, motor
pools and vehicle sources) are support, and are never lane defences. A
structure the attackers own is never a defence. Defences are worked in order
of their distance from the lane's origin, outermost first.

## Who works which defence

The squads are re-planned every bot tick, while the match is playing:

1. While some of a lane's defences stand, are not broken, and the attackers
   may capture the lane's objective now, each attacking squad of the lane
   that has bots and whose lane objective is that capture works a defence
   instead. Squads are ranked by squad index, and the n-th takes the n-th
   standing defence, so a lane's squads work its outermost defences side by
   side. The last `attacker_reserve_squads` of them stay on the capture.
2. The lane's defending squads spread the same way over the defences the
   attackers work. The last `defender_reserve_squads` (default one) stay on
   the capture.
3. A defence is broken once it has been destroyed. A repair does not send the
   lane back. It also counts as broken once attacking squads have worked it
   for `give_up_ms` without destroying it, so a lane that cannot break a
   defence still reaches its capture. When the last defence breaks, the
   squads move on to the capture.

The defence a squad works replaces its lane objective everywhere the lane is
read: objective tasks, the bot-led squad's automatic FRAGO
([bot-squad-orders.md](bot-squad-orders.md)), spawn selection and demolition
team reach. A human leader's FRAGO still comes first. A human member of a
bot-led squad therefore sees the gate or bunker its bots attack as the
squad's order. The squad's current objective (FRAGO, then defence or lane,
then the nearest objective of the team) is available to other bot
subsystems.

Duties still come first for the bots that hold them, in their existing order:
armed-objective duties and blast evacuation, repair, demolition teams, then
the squad's objective. Demolition teams leave out the defences that whole
squads work now and take other structures. Their team size, team count, rest
and give-up settings are unchanged.

## Configuration

The population file's `breach` object. Omitted fields use these defaults:

```json
"breach": {"enabled": true, "game_types": [1, 2], "give_up_ms": 300000,
  "attacker_reserve_squads": 0, "defender_reserve_squads": 1,
  "acquisition_reach_m": 300.0, "domination_reach_m": 100.0}
```

- `"enabled": false` restores the previous behaviour: every squad on its
  lane objective, capture first.
- `game_types` lists the match game types it runs in (1 is Acquisition, 2 is
  Domination). Other modes are unaffected.

## Evidence

- `bot_breach_plan`: each lane's captures, defending side and defences
  (objective and kind) in order. Recorded whenever the plan is rebuilt: once
  per match in Acquisition, and when the attackers' points change in
  Domination.
- `bot_breach`: recorded when the squads' defences change or a defence
  breaks. It carries the defences that broke this tick, with `reason`
  `destroyed` or `gave_up`, each lane's standing defences, and each squad's
  `lane_objective` and the defence `objective` it works instead.
- `bot_objective_task` and `bot_squad_orders` show the defence as the
  squad's objective.

## Not implemented

- Telling gates from barricades. Both are obstacles.
- Attacking structures with weapons. Bots only plant charges.
- Breaking defences in Sabotage.
