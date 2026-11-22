# Retail firearm mode indices

The ordinary retail Hit request carries the selected gear and the current
fire-mode index as separate bytes. An inventory report describes the active
ammo pool independently. Switching among ordinary modes can change the Hit
index while the weapon stays in ammo pool zero.

For an on-foot Hit, the server first checks the current equipment, inventory
version and ordinary ammo pool. A nonzero mode also requires an ordinary
projected bullet row for the equipped weapon and the corresponding loaded
WeaponData source. The item catalog digest, authored reference, weapon name and
ammo name must agree. The source list must have contiguous numbered rows of
distinct ordinary SemiAuto, Dual, Triple or Auto types. The request names a row
in that per-weapon list. These types' enum values are independent of their
positions. The existing mode-zero path also works with older metadata.

An admitted mode uses the same host ammunition, damage and range as the
equipped weapon's ordinary Hit. The resolver leaves melee, attachment and
mounted weapon rules in their own paths. A projectile mode does not gain
access to ordinary bullet damage through this check. Retail bullet audit rows
include the field `fire_mode`. Existing on-foot audit readers can still use
the legacy `magazine` alias.

Owners can add independently qualified ordinary rifles to the explicit
`bullets` selection for `tools/extraction/project-retail-combat.py`. The
server reads mode rows from the loaded item source, so the projected combat
metadata format needs no additional fire-mode list. This admission check
continues to use client-reported hits and observed ammunition.
