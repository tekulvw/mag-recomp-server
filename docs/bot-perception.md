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
  facing, as far as its weapon's sight distance (below). Any enemy within 4 m
  is noticed in every direction.
- The sight distance follows the equipped primary weapon's retail AI usage
  band (`ai_usage` in the item catalog's weapon parameters): near (shotguns,
  side arms) 30 m, medium (assault rifles, machine guns, submachine guns and
  most marksman rifles) 60 m, far (long-range sniper rifles) 100 m. The
  distances themselves are server tunables, not retail values. The same
  distance is the weapon controller's engagement distance and the end of its
  accuracy falloff, replacing the firearm policy's `max_engagement_distance_m`
  while perception is on; the weapon's own maximum range still applies.
- An enemy first seen at the edge of the sight distance takes 600 ms longer to
  react to than one at point-blank range (scaled linearly by distance), so
  distant enemies are not noticed instantly.
- A standing bot keeps the weapon controller's own acquisition; perception
  only adds the 4 m all-round check for it.
- Line of sight uses the static collision world inside verified physics
  coverage and, in Acquisition and Domination, the mode's placed structures
  (bunkers, field hospitals, helicopters), both on the projectile query row.
  A structure without a qualified filter binding is solid. Bodies do not hide
  a person from view; the weapon controller still checks body blocking before
  a shot. The weapon controller makes the same static and structure checks
  for every shot, and a shot a structure stops reports `bot_firearm_blocked`
  with reason `structure_blocked`. Movement and routing respect the same
  structures ([bot-structure-collision.md](bot-structure-collision.md)).
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

### Damage

A bot notices every hit it takes. When the damage record names an enemy
Character as the last attacker (a human or another bot), the attacker's current
position becomes the threat point and the attacker becomes the threat's enemy.
This needs no inference from gunfire, so it also covers suppressed weapons,
long-range shots and clients whose magazine reports are missing. Damage from
vehicles, the world or a teammate gives no point to turn to.

### Reaction

A bot first notices a threat and reacts after a delay drawn once per life from
200–600 ms by its index and life generation (deterministic). A near miss or a
hit uses the minimum delay. After reacting, the threat's point becomes the
bot's attention: without a weapon target, the weapon controller turns toward it
at 240°/s (`attention_turn_degrees_per_second`, never slower than the weapon
turn rate) and publishes the facing, so the ordinary acquisition cone can then
find the enemy. A new acquisition prefers the threat's enemy.

A bot that has already reacted is primed: the weapon's first shot at a newly
acquired enemy waits `primed_reaction_ms` (250 ms) instead of the firearm
policy's `reaction_ms`. An enemy acquired without any prior stimulus keeps the
full firearm reaction time.

A seen enemy or an attacker keeps the threat alive while it is seen or held by
the weapon; afterwards its last known position is watched for 4 s. A sound keeps the bot
alert and facing its source for 2.5 s.

## Arbitration above goals

Priority, highest first:

1. Damage from a known attacker.
2. A visible enemy (or the enemy the weapon holds).
3. A near miss, or a sound within 35% of its hearing radius (for example
   gunfire within 24.5 m, suppressed fire within 6.3 m).
4. The bot's objective or fixed goal.

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
so passages and queues are never disturbed), or after 4 s (`max_hold`). A bot
still trading fire, because its weapon holds an enemy or it was hit within the
last 2 s, keeps the hold for up to 15 s (`max_engaged_hold_ms`) before the
same `max_hold` release.
Hysteresis avoids oscillation:

- the hold only ends after the threat has lingered without stimulus, so a
  target that flickers in and out of view keeps one continuous hold;
- for 3 s after the threat clears, sounds alone cannot interrupt again; a
  visible enemy still can, after a fresh reaction delay;
- after `route_unavailable` nothing interrupts for 3 s, and after `max_hold`
  nothing interrupts for 12 s, so the bot makes real progress on its goal;
- a hit is the exception: a bot that is shot always may stop and fight back,
  even during either cooldown.

A paused bot also refreshes its objective and local-traffic progress, so
neither the planner nor the traffic arbiter treats it as stuck.

A bot standing still while it waits its turn (in a drop admission queue, at a
holding spot or during a traffic yield) may fire like any standing bot; walking
to those spots, holding an objective interaction and taking a passage still
never fire.

The short hold and long cap cooldown are deliberate. In headless bot-only
Sabotage runs, longer holds (10–20 s) left bots trading fire at range until
the cap and delayed objective completion several times over; with these
defaults a bot in sustained contact still spends most of its time advancing.
The longer engaged hold applies only while the bot is actually exchanging fire.

### Measured effect

Headless bot-only runs (every Acquisition attack/defend pairing, 600 s each,
and every Sabotage pairing, 420 s each; private inputs, deterministic). An
"ambush" is a bot hit by another bot while it had not fired or been hit for
5 s; "returned" means it fired within 10 s and before dying.

| | Acquisition before | after | Sabotage before | after |
| --- | --- | --- | --- | --- |
| Ambushed bots that returned fire | 26% | 55% | 38% | 75% |
| Died without replying | 49% | 21% | 44% | 15% |
| Median return-fire time per run | 650–850 ms | 300–400 ms | 1.05–1.75 s | 0.35–0.50 s |
| Sight detections while walking, median / 90th percentile | ≤ 25 m cap | 40 / 59 m | ≤ 25 m cap | 34 / 56 m |
| First-shot distance of an engagement, median / 90th percentile | 27 / 50 m | 29 / 54 m | 23 / 41 m | 27 / 50 m |
| Engagements started beyond 40 m | 355 | 507 | 504 | 917 |
| Objective completions | 8 captures | 6 | 33 | 38 |
| Perception CPU, slowest tick | 94 µs | 121 µs | 101 µs | 127 µs |

Every bot in these runs carries the factions' default (medium band) kit, so no
engagement started beyond 60 m. Sabotage defenders started fewer disarms (50 to
19 deactivations) because they fight back first. In Acquisition the longer
walking sight costs some objective progress (more stops on the way); capture
counts are small and noisy. No sight check was deferred for budget in any run.

## Cost

Perception runs once per world tick before bot movement. Sight is bounded by
the ray and geometry budgets above and by the per-bot interval; hearing is at
most 64 sounds per tick against each live bot (a distance check, plus a
segment distance for shots). The synthetic 64-bot check confirms the ray
budget per tick and that the round robin reaches every bot.

The weapon controller has its own per-tick budget, the firearm policy's
`max_rays_per_tick` (default 64) and `geometry_work_per_tick` (65,536). Bots
spend it in round-robin order, every bot already holding a target ahead of
every bot still searching: a held target needs one ray per shot, a search up
to `max_candidates_per_bot`. Served in plain round-robin order, a 256-bot
Domination match spent the budget on searches, reached each engaged bot about
every 500 ms and so fired about two shots a second instead of at the weapon's
refire rate. A full lobby also wants a larger budget: in a 256-bot headless
Domination run, `max_rays_per_tick` 256, `geometry_work_per_tick` 262,144 and
`max_candidates_per_bot` 4 raised an engaged bot's median from 8.5 to 10 shots
a second (the refire cap) for about 0.2 ms more firearm time per tick.

`mag-gameplay --bot-threads N` (default 1) answers perception's sight checks
and the weapon controller's target searches on N threads. Before each loop the
tick thread predicts, in budget order, the lines the loop will check, and the
workers answer them against the read-only collision world and structures. The
loop itself still runs on the tick thread. It takes a prefetched answer only
when its remaining work covers the work that answer spent, and charges that
work, so every result and receipt is the same as with one thread. Shot lines,
and lines a kill earlier in the tick changed, are asked directly. Route
planning is not parallelised.

`bot_perception_summary` records cumulative interrupts, resumes, sounds, near
misses, sight rays, deferred checks and perception CPU time every 10 s.

## Evidence

- `bot_perception_interrupt`: bot, life, cause (`damaged`, `sight`,
  `near_miss`, `{"heard": kind}`), source point, enemy, objective and operator flag.
- `bot_perception_resume`: bot, life, reason and how long the route was
  paused.
- `bot_perception_summary`: periodic totals, as above.

## Configuration

The population file's `perception` object; omitted fields use the defaults:

```json
{
  "enabled": true,
  "fov_degrees": 110,
  "view_distance_m": 60,
  "near_view_distance_m": 30,
  "far_view_distance_m": 100,
  "distance_reaction_ms": 600,
  "proximity_m": 4,
  "reaction_min_ms": 200,
  "reaction_max_ms": 600,
  "sight_interval_ms": 250,
  "sight_memory_ms": 4000,
  "sound_alert_ms": 2500,
  "max_hold_ms": 4000,
  "max_engaged_hold_ms": 15000,
  "attention_turn_degrees_per_second": 240,
  "primed_reaction_ms": 250,
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
