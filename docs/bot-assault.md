# Domination attackers fight for their points as one force

Attacking bots in Domination took every first-stage point within about a
minute, then stalled. In headless 256-character matches on the live bundle,
no platoon got past its second stage. The defenders stopped almost every
second-stage capture inside its completion window, and the platoon's other
point was taken back while it waited. Attacking bots now hold their point
from inside it, keep their stage together and push in waves. This is a
replacement design written for this server.

## What retail decides

The server's capture rules are retail's, read from the authored data:

- A point is a secure-and-hold objective with a small trigger box (8 m by
  8 m on Baku). One attacker holds the action for 3 s (`activate_ms`). The
  point then stays in its completion window for 20 s (`complete_ms`), and
  any defender who holds the action for 3 s (`deactivate_ms`) in that window
  reverses it. A secured point can be retaken: a 3 s defender hold starts a
  20 s window that an attacker cancels with a 3 s hold. Capture does not
  speed up with more attackers inside. What numbers decide is who survives
  near the point long enough to hold.
- A platoon's stage advances only while both of its points are secured at
  the same time. The controller rule locks both and opens the next pair 5 s
  later. The final stage has eight points. Each one the attackers hold costs
  the defenders 8 of their 1000 points every 15 s. The attackers lose 1 of
  their 1800 points every second, which is the match's time limit (30 min).

## What the bots did

- Attacking squads stood on the general attack roles: two bots 3 m from the
  point and six 18 to 34 m back, towards their spawn. In the completion
  window only the armed-objective guards came closer, to 8 to 14 m. The
  defenders' 15 m rule ("clear the point before a disarm") rarely found an
  attacker inside it. Defenders converged to 3 to 7 m and deactivated most
  second-stage captures within a few seconds.
- A lane that secured its point stayed there with both squads while the
  platoon's other point kept failing. The secured point went through repeated
  retakes and cancellations, so the stage never locked.
- Squads walked in as their bots arrived, one or two at a time.

## Points

Attacking bots whose squad works on a secure-and-hold point stand on a ring
around its trigger, `hold_radius_m` (default 2.5 to 7 m), inside the trigger
box or at its edge. This holds in every state of the point. The operator
still walks into a trigger and holds the action. In the completion window
the attackers' armed-objective guards use the same ring
([bot-armed-objectives.md](bot-armed-objectives.md)). With attackers inside
`contest_radius_m`, defenders start a deactivation only in the last
`late_disarm_ms` of the window, as before. Defenders are unchanged. Cover
already keeps an attacker near its destination: a spot may be at most 3 m
farther from it ([bot-cover.md](bot-cover.md)).

## Stages

The stages are read from the authored controller: a rule that needs several
objectives completed at once and then locks them all is a stage, and those
objectives are its points.

While a point of a stage is secured (completed, or being retaken or
cancelled) and another point of the stage is still open, only the first
`stage_hold_squads` (default 1) attacking squads on the secured point, by
squad index, stay to hold it. The others reinforce the open point that has
the fewest squads (ties go to the lower objective index). When the defenders
take the secured point back, its reinforcements return to it. This
reinforcement is the squad's current objective
([bot-squad-orders.md](bot-squad-orders.md)): it is published as the squad's
automatic tag, and bot lives spawn towards it. A human leader's FRAGO still
comes first.

## Waves

A squad whose open point (state `Deactivated`) has none of its living bots
within `rally_back_m` (default 35 m) gathers first. Its bots wait at a rally
place `rally_back_m` before the point, towards the squad's current spawn,
within `rally_radius_m / 2`. The push starts when `rally_share` (default 60%)
of its living bots are within `rally_radius_m` of the rally place or already
inside `rally_back_m`, or after `rally_wait_ms` (default 20 s). A squad that
already has a bot forward pushes at once. A push lasts at least
`rally_wait_ms`, and then until the squad has no living bot within
`rally_back_m + rally_radius_m` of the point. Squad members on another duty
(armed objective, repair, demolition) do not count. A rallying bot is never
chosen as an operator. Once the point is being taken or held, there is no
rally: everybody goes in.

## Configuration

The population file's `assault` object. Omitted fields use these defaults:

```json
"assault": {"enabled": true, "game_types": [2], "hold_radius_m": [2.5, 7.0],
  "stage_hold_squads": 1, "rally_back_m": 35.0, "rally_radius_m": 12.0,
  "rally_share": 0.6, "rally_wait_ms": 20000}
```

- `"enabled": false` restores the previous behaviour: attack roles, every
  squad on its own lane point, no rallies.
- `"rally_back_m": 0` turns rallies off only.
- It runs only in Domination matches with interactions enabled.

## Evidence

- `bot_objective_task`: `destination_role` is `point_ring` or `rally` for
  these destinations.
- `bot_squad_orders`: `source` `stage` for a reinforcing squad.
- `bot_assault_wave`: a squad's wave changed (`rally`, `push` or `none`),
  with its `objective` and the number of its `living` bots on it.
- `domination_objective_states`: every point change.

## Measurements

See the pull request for the A/B runs on Baku (Raven and Valor) and Canal.

## Not implemented

- Vehicles. Bots do not spawn in or ride APCs and helicopters. They already
  use the forward points the mode opens for their squad, including parachute
  points once the anti-air falls, ranked by route to the squad's current
  objective ([bot-active-spawns.md](bot-active-spawns.md)), so a reinforcing
  squad spawns towards the point it reinforces.
- Attacking a stage's two points with the whole platoon at once.
