# Bot perception and goal interruption

World-owned bots notice enemies that cross their view while they are busy
walking to a goal, hear gunfire, explosions, footsteps and near misses, and may
pause an ordinary route to react before resuming it. This is a replacement
design written for this server; it does not reproduce the original game's AI.

## Previous behaviour

The weapon controller (`bot_firearms`) only runs for a bot that is standing
still: not relocating, not sprinting, not an objective operator and not in any
passage, queue, supply or traffic task. It acquires the nearest enemy inside a
120° cone around the published facing, within the AI and weapon range, with
static and body line of sight. A bot walking to an objective therefore ignored
an enemy in front of it until it arrived, and a standing bot never turned
toward anything behind it.

## Perception state

Each bot life has a small perception record (`bot_perception::Mind`), reset
when its index starts another allocation or life:

- **Threat**: the point the bot is reacting to, the enemy (when seen), the
  cause, when it may react, when it was last stimulated and how long it
  lingers without a fresh stimulus.
- **Hold**: whether the bot's route is paused, and since when.
- **Cooldown**: after a resume, sounds alone cannot interrupt for a while; a
  hard cooldown after the maximum hold blocks every interruption.

### Sight

- A walking bot looks along a 110° horizontal cone around its published
  facing, up to 25 m. Any enemy within 4 m is noticed in every direction.
  Farther enemies do not stop a walking bot; once it stands, the weapon
  controller's own 60 m acquisition applies.
- A standing bot keeps the weapon controller's own acquisition; perception
  only adds the 4 m all-round check for it.
- Line of sight uses the static collision world inside verified physics
  coverage. Bodies do not hide a person from view; the weapon controller still
  checks body blocking before a shot.
- Each bot is checked at most every 250 ms, nearest two candidates first, in
  round-robin order. A tick spends at most 32 rays and 32,768 geometry work
  units; a bot that does not fit waits for the next tick.

### Hearing

Sounds are positions with a class and the source's team. A bot ignores its
own team and itself. Hearing radii:

| Sound | Radius |
| --- | --- |
| Firearm | 70 m |
| Sniper rifle (authored category) | 110 m |
| Suppressed (muzzle attachment slot populated) | 18 m |
| Explosion (reported projectile detonation) | 90 m |
| Sprinting footsteps | 15 m |
| Walking footsteps | 6 m |

A shot also carries its bullet path. A path that passes within 3 m of a bot
(that is not the shooter's own position) is a **near miss**, regardless of how
loud the weapon is.

Sources:

- Bot shots are reported by the weapon controller when it fires.
- A human's shot is inferred when the reported magazine count of the same
  equipped weapon falls without a reload; its path follows the reported yaw
  and pitch.
- Footsteps come from walking bots, and from humans whose reported position
  moves faster than 1 m/s (walk) or 5 m/s (sprint), at most twice a second.
- Explosions come from human projectiles that reported a detonation point.

No acoustic occlusion is modelled. Vehicles, mounted weapons and rockets are
not sound sources yet.

### Reaction

A bot first notices a threat and reacts after a delay drawn once per life from
200–600 ms by its index and life generation (deterministic). A near miss uses
the minimum delay. After reacting, the threat's point becomes the bot's
attention: without a weapon target, the weapon controller turns toward it at
its normal turn rate and publishes the facing, so the ordinary acquisition
cone can then find the enemy.

A seen enemy keeps the threat alive while it is seen or held by the weapon;
afterwards its last known position is watched for 4 s. A sound keeps the bot
alert and facing its source for 2.5 s.

## Arbitration above goals

Priority, highest first:

1. A visible enemy (or the enemy the weapon holds).
2. A near miss, or a sound within 35% of its hearing radius (for example
   gunfire within 24.5 m, suppressed fire within 6.3 m).
3. The bot's objective or fixed goal.

A farther audible sound turns a standing bot but never interrupts a route.

Only **ordinary route following** may be interrupted: the bot is relocating
on a grounded, non-sliding body, and has no passage, drop queue, holding spot,
retreat, egress, reroute, traffic yield, supply task or ground recovery, and
is not within the drop admission queue distance (plus 2 m) of a certified
drop on its route. An
objective operator that is still walking to its objective may be interrupted;
an operator holding an interaction is never relocating and is not.

An interruption pauses the route in place. The route, its corridor and the
objective task are kept; no replanning happens. The paused time is not
counted against the move deadline, the planner sees the task as waiting
rather than stalled, and the body stops sprinting and recovers stamina. While
paused the bot may fire like a standing bot.

The route resumes when the threat expires (`threat_cleared`), when the bot
stops being an ordinary route follower (`route_unavailable`, released at once
so passages and queues are never disturbed), or after 4 s (`max_hold`).
Hysteresis avoids oscillation:

- the hold only ends after the threat has lingered without stimulus, so a
  target that flickers in and out of view keeps one continuous hold;
- for 3 s after the threat clears, sounds alone cannot interrupt again; a
  visible enemy still can, after a fresh reaction delay;
- after `route_unavailable` nothing interrupts for 3 s, and after `max_hold`
  nothing interrupts for 12 s, so the bot makes real progress on its goal.

A paused bot also refreshes its objective and local-traffic progress, so
neither the planner nor the traffic arbiter treats it as stuck.

The short hold and long cap cooldown are deliberate. In headless bot-only
Sabotage runs, longer holds (10–20 s) left bots trading fire at range until
the cap and delayed objective completion several times over; with these
defaults a bot in sustained contact still spends most of its time advancing.

## Cost

Perception runs once per world tick before bot movement. Sight is bounded by
the ray and geometry budgets above and by the per-bot interval; hearing is at
most 64 sounds per tick against each live bot (a distance check, plus a
segment distance for shots). The synthetic 64-bot check confirms the ray
budget per tick and that the round robin reaches every bot.

`bot_perception_summary` records cumulative interrupts, resumes, sounds, near
misses, sight rays, deferred checks and perception CPU time every 10 s.

## Evidence

- `bot_perception_interrupt`: bot, life, cause (`sight`, `near_miss`,
  `{"heard": kind}`), source point, enemy, objective and operator flag.
- `bot_perception_resume`: bot, life, reason and how long the route was
  paused.
- `bot_perception_summary`: periodic totals, as above.

## Configuration

The population file's `perception` object; omitted fields use the defaults:

```json
{
  "enabled": true,
  "fov_degrees": 110,
  "view_distance_m": 25,
  "proximity_m": 4,
  "reaction_min_ms": 200,
  "reaction_max_ms": 600,
  "sight_interval_ms": 250,
  "sight_memory_ms": 4000,
  "sound_alert_ms": 2500,
  "max_hold_ms": 4000,
  "resume_cooldown_ms": 3000,
  "max_hold_cooldown_ms": 12000,
  "interrupt_fraction": 0.35,
  "hearing": {
    "firearm_m": 70, "sniper_m": 110, "suppressed_m": 18,
    "explosion_m": 90, "sprint_m": 15, "walk_m": 6, "near_miss_m": 3
  },
  "max_sight_rays_per_tick": 32,
  "geometry_work_per_tick": 32768,
  "max_sounds_per_tick": 64
}
```

`mag-gameplay --bot-perception off` disables perception for every population,
including match-profile populations, and restores the previous behaviour:
no route is paused, no facing changes without a weapon target, and the weapon
controller sees no perception input. `--bot-perception on` forces it on even
when a population file disables it. Perception also stays inactive whenever
the population's `firearms` policy is disabled.

The layer is mode-neutral: it reads bot routes and tasks, not objective
kinds, so it applies to Sabotage, Acquisition and Domination populations.

## Not implemented

- Walking to investigate a last known position, taking cover and strafing.
  A bot watches the last known position from where it paused instead.
- Acoustic occlusion and vehicle, rocket and mounted-weapon sounds.
- Acceptance against live clients: whether retail clients report magazine
  counts on every shot, as human gunfire detection assumes, is unverified.
