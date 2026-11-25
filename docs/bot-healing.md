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
  - a downed human teammate is revived when `revive` is on and the match
    has incapacitation rules.
- **Combat:** a medic whose weapon holds an enemy only uses the item for a
  teammate at or below 40% health or downed.
- **Cooldown:** one use starts the item's authored cooldown, and each patient
  is treated by at most one medic per tick. Enemies are never treated.

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
  "revive_health_per_mille": 250
}
```

`"teammates": false` or `"medic_role_slots": []` keeps every bot on its
default kit and self-healing only.

## Evidence

`bot_teammate_healing_started` records the medic, item, sphere centre and
radius, whether it was fighting, whether it treated itself, and each patient
(`bot`, `human` or `revived`, with health before).

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
- Bots are never downed, so only humans are revived.
