# Bots respond to armed objectives

Before this change, defenders ignored a planted charge unless their squad's lane
already pointed at it. Attackers did nothing in particular to protect one they
had armed. Defender bots also never deactivated a Sabotage A/B capture. Bots now
take an objective-level duty while an objective is armed.

## When an objective counts as armed

- A demolition objective (Sabotage C, Acquisition and Domination structural
  charges) in `Activated` (fuse running) or `Deactivating` (being defused),
  when its rule has `can_defuse`.
- A Sabotage capture point in `Activated` (inside its completion window) or
  `Deactivating`, when its rule has `can_deactivate`. Defender bots may now
  perform that deactivation, as humans already could
  ([sabotage-defender-control.md](sabotage-defender-control.md)).

An objective the defenders cannot reverse is not armed, and no duty is
assigned for it. Retaking an already `Completed` capture point is not
attempted by bots.

## Duties

| Team | Duty | Goal |
| --- | --- | --- |
| Defenders | `armed_converge` | ring 3–7 m around the objective |
| Attackers | `armed_guard` | ring 8–14 m around the objective; a Domination point's ring, 4–12 m by default ([bot-assault.md](bot-assault.md#points)) |

A bot already standing inside its duty's outer radius keeps its spot instead
of walking to a ring place, so it can keep firing. While a defender holds a
disarm, guards prefer that defender when they pick a target and drop another
target at their next perception check. Guards do not walk in to hunt the
defender: a moving bot cannot fire, and the hold lasts only a few seconds.

Defenders clear the objective before they disarm it. The usual operator
selection picks the nearest eligible defender (converging, or already on that
lane) to defuse or deactivate. It starts a new disarm only when no living
attacker (bot or human) is within `contest_radius_m` of the objective, or when
at most `late_disarm_ms` of the window or fuse remains. A disarm already under
way is kept.

Selection is per objective and team: squads in order of their closest
available member, then members by distance, with ties broken by squad and
object index. At most `defender_cap` defenders and `attacker_cap` attackers
join, and only bots within `reach_m`. Bots that are dead or servicing supply
cannot join, and neither can a bot holding an action on an objective that is
not armed (planting a charge, taking a capture). Drafting it would cancel that
action at once and flip its objective back to where it was. A bot keeps its duty, and its ring place, for the rest of its
life or until the objective stops being armed. Places freed by a death are
refilled. Ring places are spread by slot. A place whose ground is unsupported
or unreachable is rotated to the next candidate.

When the objective is defused, deactivated or completes (detonates), or the
match ends, every duty ends. The bots then replan to their squad's normal lane.

Duty goals are ordinary objective routes through the shared bounded planner
(`candidate_checks_per_tick`, `route_queries_per_tick`, `planning_slice_us`,
wave limits). Anything that pauses an ordinary objective walk, such as a combat
reaction, also pauses and resumes a duty walk. The selection itself compares
distances for each armed objective and bot.

## Attackers keep out of the blast

An armed demolition charge explodes with its authored instantaneous blast
when its fuse ends. The blast is centred on the trigger where the charge was
planted. Its hot zone is a circle around the objective's position: the
distance to its farthest trigger, plus the authored outer blast radius, plus
`blast.margin_m`. Only objectives whose mode runtime supplies the blast
(Domination and Acquisition structural objectives) have a zone. While the
objective is armed, defusable or not:

- Every living attacking bot inside the zone takes an `armed_guard` duty on it
  if it has none there, outside the usual cap and reach. The planter is one of
  them: once the charge is armed it has no action left there and walks out.
- The attackers' guard ring is pushed outside the zone and keeps its width.
  A guard keeps its current spot only when that spot is outside the zone.
- An attacking bot's route that would enter a zone it does not start in is
  refused (`bot_route_blast_zone`), and the bot plans again later. By then the
  charge has gone off or been disarmed.

Defenders are not moved. They may stay to defend the objective or disarm it
as before.

## Configuration

`objective_policy.armed` in the population file. If omitted, these defaults
apply:

```json
"armed": {"enabled": true, "defender_cap": 8, "attacker_cap": 8, "reach_m": 160.0,
  "converge_radius_m": [3.0, 7.0], "guard_radius_m": [8.0, 14.0],
  "contest_radius_m": 15.0, "late_disarm_ms": 8000}
```

`"enabled": false` turns off the duties and the clear-first rule. Bots still
deactivate a Sabotage capture when their lane leads there. Defusing and
deactivating always need `interactions_enabled`.

The population file's `blast` object sets the hot zone's margin. With
`"enabled": false`, attackers no longer leave an armed charge's zone and their
guard ring and routes ignore it:

```json
"blast": {"enabled": true, "margin_m": 4.0}
```

## Evidence receipts

- `bot_blast_evacuation`: an attacker inside an armed charge's zone was sent
  out (`objective`, `radius_m`, `position`).
- `bot_armed_objective_duty`: `joined` true/false, `objective`, `role`
  (`defend`/`attack`), `slot`, `duty`.
- `bot_objective_task`: `destination_role` is the duty code for duty goals.
  A bot that keeps its spot gets `"held_position": true`.
