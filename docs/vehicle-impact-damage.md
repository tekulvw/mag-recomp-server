# Ground-vehicle impacts on characters

A driven ground vehicle hurts the characters it hits. The feature is opt-in:
supply an absolute `--vehicle-collision-rules /absolute/VEHICLE_COLLISION.json`
with `--application-id 20231`. Without it the server still decodes every
report and refuses it with `vehicle_collision_rules_missing`, so the gap stays
visible in the event log.

## Who reports a hit

The server does not simulate vehicle-character contact. The retail client of
the player driving a ground vehicle detects it and sends message `0x020d`:

| Bytes | Field |
| --- | --- |
| 1 | Kind: 0 stuck, 1 collision, 2 catastrophic collision |
| 4 | Attacker: the vehicle's object ID (`0x0100xxxx` for a ground vehicle) |
| 4 | Defender: the character's object ID (`0x0380xxxx`) |

The server accepts a collision report only from the vehicle's current driver
(seat 0 with the driving grant) while the round allows combat. A report about
any other vehicle is refused with `vehicle_report_requires_vehicle_driver`.
A vehicle with no driver therefore never hurts anyone, as in retail, where a
vehicle without an owning client applies no impact damage. Helicopters have no
character-impact damage in retail; a helicopter report is recorded with outcome
`helicopter_collisions_have_no_effect` and changes nothing. Stuck reports are
not supported yet and are refused with `vehicle_stuck_report_unsupported`.

The victim is a human in the same world who is on foot and alive or downed, or
a registered bot. A character that got out of this same vehicle less than
`recent_exit_grace_ms` ago is not hit by it (outcome `recent_occupant`), so a
squad leaving an APC is not run over when the driver pulls away.

## Damage

An ordinary collision uses the closing speed. The server estimates each
velocity from the two latest accepted positions: the driver's vehicle reports
for the vehicle and the owner reports for a human victim. It divides by at
least `minimum_motion_interval_ms` and treats a position older than
`motion_freshness_ms` as standing still. Bots have no sampled motion here and
count as standing still. When only one of the two moves, the closing speed is
that one's speed. When both move, it is the vehicle's speed less the
character's speed along the vehicle's direction of travel, as a magnitude: a
victim running into the vehicle takes more, one running away takes less.

```
raw = damage_per_mps * (closing_speed - minimum_velocity_mps)   when closing_speed > minimum_velocity_mps
```

Slower collisions do nothing (outcome `below_minimum_speed`).

A catastrophic collision deals a flat `catastrophic_damage_hp`. When the victim
is on the vehicle's side and the match's friendly-fire multiplier is above
zero, the raw amount is first divided by that multiplier. The friendly-fire
base applied next cancels the division, so a catastrophic hit still kills a
teammate whenever friendly fire is on. With friendly fire off it does nothing.
Without `--character-damage-rules` the multiplier is 1.

The raw amount then goes through the victim's ordinary damage path: the
friendly-fire base and owned `kCharModDamageTaken` and
`kCharModImpactDamageTaken` modifiers, then armor at hit location 0 with no
armor reduction. The damage kind is ExternalKinetic (5) without the
KineticResponse flag. A lethal impact can therefore down a player under the
[incapacitation](incapacitation.md) rules, like a bullet, if the adjusted damage
is below the downing threshold. Damage at or above it kills outright.

The driver is the attacker of record and the vehicle is the source object.
Kill and friendly-kill statistics, the enemy-kill reward, the finishing reward
for a downed enemy and the bot's death count follow the existing rules for
damage the driver caused. The death notice names source type Vehicle (3) with
the vehicle's object ID as its index and goes to both squads.

## Producing the rules

```sh
python3 tools/extraction/extract-retail-vehicle-collision.py \
  --elf /absolute/RETAIL_ELF --game-archive /absolute/game.psarc \
  --output /absolute/VEHICLE_COLLISION.json
```

The tool checks both inputs' SHA-256. It reads `VehicleCollisionDamageMinVelocity`
and `VehicleCollisionDamageFactor` from the archive's server properties and
the other values from fixed supported-build operands, after checking each
operand's instruction encoding. The output records the source digests in
`provenance`. Repeat with `--check` to verify reproduction. Keep the output
outside this repository with the other private data bundle files.

The schema is strict:

```json
{
  "schema_version": 1, "source_build": "retail", "application_id": 20231,
  "minimum_velocity_mps": 4.0, "damage_per_mps": 20.0,
  "catastrophic_damage_hp": 250.0, "recent_exit_grace_ms": 1500,
  "motion_freshness_ms": 2500, "minimum_motion_interval_ms": 40.0,
  "damage_type": 5, "damage_source_type": 3
}
```

These numbers are invented for illustration and are not the retail values.

## Limits

- The driver's client decides that a contact happened and whether it was
  catastrophic. The server bounds the driver's reported vehicle speed but does
  not check the contact geometry.
- Retail also marks a hit victim as recently struck for 1.5 s, which its
  movement verification appears to use to excuse the push. This server has no
  equivalent; the victim's own client moves it.
- Bots do not drive, so a bot is never the attacker.
