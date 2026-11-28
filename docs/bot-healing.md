# Bot healing

World-owned bots heal themselves and, when they carry an item that heals
others, injured and downed teammates. Amounts, durations and cooldowns come
from the same item catalog and healing effects that human healing uses. This
is a replacement design written for this server.

## Self-healing

A standing bot at or below `health_threshold_per_mille` (750) of its maximum
health uses the first healing item in its kit on itself, once its cooldown
allows (`bot_healing_started`, `bot_healing_tick`).

## Medics

The item that heals others is the healing item with an authored recipient
sphere in the retail combat catalog (the medkit). Its sphere sits
`forward_offset` metres ahead of the user with the authored `radius`.

- **Kit:** bots in the squad role slots listed in `medic_role_slots`
  (default `[1]`, one per squad) carry that item in place of their
  self-healing item. Every other item in the kit, including the weapon, is
  unchanged. A kit without a healing slot stays as it is. The default kits
  carry no such item, so this is a server choice, not a retail entitlement.
- **Use:** a medic that is alive, grounded, not taking a passage and not in a
  supply task, with its cooldown ready, looks for teammates inside the
  sphere's reach. It faces the neediest one (downed first, then the lowest
  health share) and treats every teammate inside the sphere, itself
  included when injured:
  - a bot or human at or below the health threshold, not already being
    healed, receives the item's healing effect, built with the human
    healing rules (`received_bot_healing_tick` for bots; a human's own
    healing ticks for humans);
  - a downed human or bot teammate is revived when `revive` is on and the
    match has incapacitation rules.
- **Combat:** a medic whose weapon holds an enemy only uses the item for a
  teammate at or below 40% health or downed.
- **Cooldown:** one use starts the item's authored cooldown, and each patient
  is treated by at most one medic per tick. Enemies are never treated.

### Reaching downed teammates

When `approach_downed` and `revive` are on, every downed teammate, human or
bot, that a medic can still revive gets one bot medic sent to it: the nearest
free medic of its side within the downed-bot medic radius
(`incapacitation.medic_radius_m`, 30 m, straight-line; see "Downed bots" in
[incapacitation.md](incapacitation.md)). Free means alive, not downed,
grounded and not on another job.

- **Priority:** the medic leaves its ordinary objective movement and its
  cover. It is never taken from a higher duty: an armed-objective, blast or
  defuse duty, a repair or demolition duty, an objective interaction, a
  supply action or a passage. When one of those comes up, it drops the
  approach.
- **One medic per patient:** a medic serves one patient, and a patient has
  one medic. At most four new routes start per tick, nearest pairs first.
- **Only walkable approaches:** a medic is sent only when it stands on the
  patient's navmesh walking component and the planned route takes no
  passage (it holds no permit, so it would stop there) and is at most twice
  the radius long. Otherwise that medic is not sent to that patient again
  during the patient's downed life.
- **Walk and revive:** the medic walks the navmesh to the patient and stops
  once the patient is well inside its medkit's sphere. The ordinary medkit
  use above then revives the patient.
- **Resuming:** the medic keeps its objective task through the approach and
  resumes it afterwards, and a revived bot resumes the task it had when it
  went down: both keep their destination and only plan the route there
  again.
- **Release:** the approach ends, and the medic returns to its objective,
  when the patient is revived or bleeds out, when the medic dies or goes
  down, when it takes a higher duty, or when it gets no closer for 6 s. After
  that last case the same medic is not sent to the same patient again during
  that downed life.

The downed bot's own wait is unchanged: it still checks for a medic within
30 m at 10 s and every second after, and a medic on its way counts.

Receipts: `bot_medic_approach_started`, `bot_medic_approach_arrived`,
`bot_medic_approach_ended` (with `reason`: `patient_revived`,
`patient_lost`, `medic_down`, `higher_duty`, `supply_action`, `passage`,
`no_progress` or `disabled`) and `bot_medic_approach_unroutable`.

### Revival

A bot revival follows the human Resuscitate request. The patient's damage
record is restored to `revive_health_per_mille` (250) of the reviver's
maximum health, capped at the patient's own maximum, and the patient becomes
alive with the same life. Both squads receive the resuscitation notice, and
the new health is published to every recipient. Bots own no skills, so this
policy share stands in for the human resuscitation modifier. A bot healer
earns no XP, and the patient's previous attacker credit is cleared as for a
human revival.

## Configuration

The population file's `healing` object; omitted fields use the defaults:

```json
{
  "enabled": true,
  "health_threshold_per_mille": 750,
  "teammates": true,
  "medic_role_slots": [1],
  "revive": true,
  "revive_health_per_mille": 250,
  "approach_downed": true
}
```

`"teammates": false` or `"medic_role_slots": []` keeps every bot on its
default kit and self-healing only.

## Evidence

`bot_teammate_healing_started` records the medic, item, sphere centre and
radius, whether it was fighting, whether it treated itself, and each patient
(`bot`, `human`, or `revived` or `revived_bot` with the restored health).

Headless bot-only runs, with private inputs and deterministic:

| | Acquisition, 6 × 600 s | Sabotage, 6 × 420 s |
| --- | --- | --- |
| Medkit uses on teammates | 29 | 92 |
| Teammates treated | 29 | 93 |
| Health restored to teammates | 1,005 HP | 4,234 HP |
| Uses while fighting | 1 | 10 |

Humans and revivals are covered by the synthetic tests; the headless runs have
no human teammates.

## Not implemented

- Medics do not walk to injured teammates. They treat whoever is within
  reach of their sphere, typically squadmates sharing an objective position.
  Only downed teammates bring a medic over (above).
