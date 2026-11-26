# Varied bot kits and appearance

Without a kit catalog, every bot carries its faction's first bootstrap kit and
presents head 0 and voice 0. Squad medics swap their first-aid pack for the
medkit, and Domination engineers swap it for the repair tool. A match profile
row can name a `bot_kits` file (`mag_bot_kits_v1`). Its bots then carry
different equipment, uniforms, head and face apparel, faces and voices.

The catalog is operator data. It names retail item IDs, so it belongs in the
private data bundle, never in this repository. The tests use invented items.

## Format

```json
{
  "format": "mag_bot_kits_v1",
  "application_id": 20231,
  "provenance": {"source": "operator projection"},
  "heads": [0, 1, 2],
  "voices": [0, 1, 2],
  "apparel_head_per_mille": 650,
  "apparel_face_per_mille": 300,
  "roles": [
    {"role": "rifleman", "weight": 30},
    {"role": "support", "weight": 14, "max_per_squad": 2}
  ],
  "factions": [
    {"faction": 0,
     "kits": [{"id": "0-rifleman-01", "role": "rifleman", "items": [/* 22 slots */]}],
     "apparel": [{"armor": 31, "uniforms": [41], "head": [42], "face": [43]}]}
  ]
}
```

- `items` holds the 22 inventory slots in the client's item slot order:
  primary, sidearm, knife, launcher, two gear slots, PDA, C4, satchel,
  parachute, head apparel, face apparel, armor, uniform, then the primary's
  sight, magazine, muzzle and underbarrel, the sidearm's sight, magazine and
  muzzle, and the launcher sight. The apparel and magazine slots must be
  empty. The server fills apparel from the `apparel` row of the kit's armor.
- A kit's `role` is one of `rifleman`, `medic`, `engineer`, `support`,
  `marksman`, `anti_armor` or `close_quarters`. `roles` gives the draw weight
  of every role except `medic` and `engineer`. A role may set a per-squad cap.
  A kit may set `weight` (default 1). `cost` and `skill_points` are audit
  values from the projection and are ignored at runtime.
- Heads must be below 15 and voices below 9. Retail character creation offers
  heads 0 to 14 and nine voices to every faction. Vocal IDs 9 to 11 belong to
  the faction commanders.

## Selection

- **Role.** A bot in a squad role slot that the healing policy makes a medic
  gets a `medic` kit. In Domination, a bot in an engineer slot of the repair
  policy gets an `engineer` kit. Every other bot draws a role by weight. Roles
  whose `max_per_squad` its squad already reaches are skipped. If every role is
  capped, the caps are ignored.
- **Kit and apparel.** The kit is drawn within the role. The uniform, head
  apparel and face apparel are drawn from the kit's armor pools. The draw is
  seeded by the faction, the bot's name slot and the selected match's
  formation ID. A bot keeps its kit for all its lives in a match. Another match
  can give it different equipment.
- **Face and voice.** Head and voice are seeded only by faction and name slot.
  The same named bot keeps its face and voice in every match.

The medic and engineer swaps still run after selection. A duty kit already
carries its item, so the swap leaves it unchanged. Each `bot_population_spawn`
record carries `kit`, `kit_role`, `role_slot`, `head`, `voice` and `primary`
when a catalog is installed. Without a catalog, records are unchanged.

## Validation

Startup and `--validate-catalogs` check every kit and apparel item against the
worker's item catalog, retail Armory and retail combat metadata. Startup stops
on any error.

- Every item is offered to the kit's faction and resolves to an authored
  definition.
- The primary and sidearm fit their gear slots. The knife slot holds a knife.
  Gear slots hold equipment. A launcher has its authored required sight.
- Sights, muzzles and underbarrel attachments resolve for their host weapon
  through the same attachment rules as human hits. The primary resolves to a
  bullet profile on foot, with a recoil model the bot firearm can use, and the
  armor to an armor profile.
- Every kit carries a healing item. Only medic kits carry the medkit, and only
  engineer kits carry the repair tool, so the healing and repair duties stay
  tied to role slots.
- A uniform's armor dependency names the kit's armor. Head and face apparel
  list that armor and use the matching attachment type.
- Every faction has medic and engineer kits and at least one drawn role.
  Both match factions must be covered.

Bot firearms now resolve attachments the way an on-foot human's do. A bot
whose rifle has a suppressor, grip, bipod or launcher still fires. Default
kits carry no such attachments, so they are unaffected.

## Limits

- Bot aim covers assault rifles, SMGs and machine guns. Retail sniper rifles
  and shotguns fail its recoil model, so a catalog that contains them is
  rejected. Marksman kits are scoped assault rifles.
- Bots do not fire launchers, underbarrel launchers or grenades, and they do
  not place mines. Those items are carried for appearance.
- Armor weight has its usual damage effect on bots. Bot movement speed does
  not depend on armor.
- The operator projection decides which apparel counts as valid. It currently
  uses only apparel whose authored armor list names the armor.
