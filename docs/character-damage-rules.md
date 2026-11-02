# Selected-match character damage rules

`--character-damage-rules /absolute/DAMAGE.json` explicitly enables a disk-loaded
retail friendly-fire base. Omitting the flag preserves the existing base-one
behavior. An adjacent file is never loaded implicitly. This is source support;
it does not enable a deployed profile or establish native live acceptance.

The strict JSON root has `schema_version: 1`, `application_id: 20231`, `map_name`,
native `game_type`, `friendly_fire_multiplier`, and a `provenance` object. The
multiplier is finite native f32 within inclusive 0..100. Provenance must include
`spawn_catalog_sha256`, the SHA-256 of the exact original spawn-file bytes.
Runtime validates that digest, map and mode against the loaded catalog before
accepting the policy. Source hashes survive per-squad catalog clones and runtime
spawn selection. Unknown root fields or mismatches fail clearly.

Private maintained projection should follow the selected spawn metadata's exact
authored rule pointer and member digest, and verify the selected level/mode.
Keep the rule value and source provenance external. A separate global default
is not an implicit replacement for a missing selected-match rule.

For ordinary supported retail firearm, melee, grenade and bot-firearm damage:

```
base = configured factor for resolved, distinct, same-team characters; 1 otherwise
scale = max(0, f32(f32(base + generic_damage_taken) + specialized_damage_taken))
incoming = f32(raw_amount * scale)
```

The relationship uses actual world/team and character identity, never faction
or the damage source-object handle. Retained dead sources remain resolvable until
removal. Self, unresolved and enemy sources use base 1. Heal and Resuscitate do
not enter these ordinary armor calculations and retain their separate effects.

Incoming damage kind is already typed: firearm Bullet, melee Swipe, grenade
Explosive with a separate response flag. Metadata/object type numbers do not
select resistance. Explosive uses the explosive modifier, not impact resistance.
The ordered additions and raw multiplication precede armor rating subtraction.
A zero base can still produce damage through positive modifiers.

Grenade targets receive the base individually before armor and visibility.
Queued results already contain that amount and are never scaled a second time,
including after relationship changes. Existing generation, owner, recording,
life and reward rules remain in force.

The existing adjusted ordinary-damage 1000 HP admission cap is unchanged; large
configured factors can still reach that local refusal. Falling remains a
separate self/base-one path with an existing modifier/armor parity gap. This
feature does not enable unsupported retail Alpha weapons/bunker repair, invent
invulnerability producers, or claim complete character damage lifecycle parity.
