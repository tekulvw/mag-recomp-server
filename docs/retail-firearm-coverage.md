# Retail firearm coverage

Ordinary character hits work for every retail character firearm: assault
rifles, submachine guns, machine guns, shotguns, sniper rifles and sidearms. The
projected combat metadata (`retail-combat.json`) can list only a few of them, such as
the starter weapons. The server builds bullet rows for the rest from the loaded
item catalog (`--item-catalog`). It does this only when the combat metadata's
`provenance.catalog_sha256` matches that exact item catalog.

## Derived bullet rows

A firearm item gets a derived row when the item catalog has all of these:

- a `kWeapon` item bound to a weapon whose category is one of the six character
  firearm categories;
- a hitscan ammunition definition with a nonzero ammunition token;
- finite character, vehicle and objective damage values, with the reduction
  fractions no higher than one;
- an effective range no greater than a positive maximum range;
- magazine capacity and initial magazine count from 1 through 255.

A derived row uses the same fields as a projected row: ammunition damage,
armour pierce and reduction, effective and maximum range, and magazine values.
Hit resolution then follows the ordinary rules. The reported scale and the range
falloff are multiplied separately, and the target's armour and body location
apply afterwards. Projected rows always take precedence. An item already listed
under any combat group is not derived again. Derived ammunition tokens are
candidates only and are never marked as verified.

The startup `listening` event lists the derived item IDs under
`retail_combat_metadata.item_catalog_bullets`. Firearms that are skipped, and the
reason, are under `item_catalog_bullet_skips`.

A derived firearm must also be in its own gear: gear 1 holds a sidearm, and
gear 0 holds any other character firearm. Otherwise the hit is refused with
`unsupported_bullet_weapon_gear`.

## Shotguns

The server does not count shots. Each Hit the client reports for one pellet
deals the ammunition's authored character damage once.

## Ammunition pools

Every authored character firearm names exactly one hitscan ammunition. No
authored alternate bullet magazine exists, such as armour-piercing or explosive
rounds. Ordinary hits therefore still require ammunition pool 0. Any other pool
is refused with `unsupported_bullet_magazine`. Underbarrel launchers fire
through their own projectile path.

## Attachments

No authored attachment carries damage, range or ammunition values. Sights,
suppressors, forward grips and bipods only change things like sound, camera and
handling, so they never change the host weapon's bullet. A suppressor therefore
deals the same damage over the same range as the bare weapon. An underbarrel
launcher names its own weapon, which fires through the separate launcher
runtime.

Each populated attachment slot is classified as follows:

| Gear | Slot | Accepted kind |
| --- | --- | --- |
| 0 | 15 | sight |
| 0 | 16 | suppressor |
| 0 | 17 | bipod, forward grip or qualified underbarrel launcher |
| 1 | 19 | sight |
| 1 | 20 | suppressor |

The primary and sidearm sight slots (14 and 18) are not checked.

An attachment is accepted when all of these hold:

- it resolves to an authored attachment item and definition;
- it has an empty replacement weapon, unless it is a qualified launcher;
- it has a known kind and the primary character role;
- it suits the host's gear role.

It must also appear in the host's authored attachment list, or be authored for
the host's faction (or for all factions). The item list offers some sights and
grips that no weapon list names; this second rule is what admits them.

A sniper rifle that requires an underbarrel attachment still needs that
attachment in slot 17. A host with a required underbarrel attachment accepts no
other bipod or grip.

Each refusal has its own code, beginning with `unsupported_bullet_attachment`:
`_host`, `_host_role`, `_unresolved`, `_replacement`, `_launcher`, `_type`,
`_role`, `_incompatible`, `_required_underbarrel`,
`_required_underbarrel_missing`, `_slot` and `_while_mounted`.

A refused ordinary firearm Hit records a `refused_equipment` field in its audit.
This field holds the gear, the weapon item and the populated attachment slots,
so missing coverage can be seen in the event log.
