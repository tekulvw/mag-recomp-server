# Domination APC primary gunners

APC gunner support is an optional source-bound Domination increment. It admits
physical seat1 on the eight existing source-type3 APCs per profile. A driver in
seat0 and a gunner in seat1 have independent control. Reinforcement passengers
continue using seats2–9, and all occupants count toward the existing cap8.

## Owned-data projection and configuration

Use `tools/extraction/project-retail-apc-turrets.py` with original retail files
you own and have permission to use. Supply `--elf`, `--game-archive`,
`--main-archive`, the previously extracted `--ground-sources`, `--item-catalog`
and `--item-bindings`. Repeat `--profile SPAWNS GROUND` for all six Domination
profiles and supply `--output` and optional `--proof-output`. Repeat the command
with `--check` to reproduce existing files byte for byte. Outputs belong outside
the public checkout.

The input Ground catalogs must already provide the qualified APC driver route,
current seat visibility and source bindings. Rules bind the exact loaded Ground
and spawn bytes, original vehicle type, faction weapons and named seat offsets.
The runtime overlay adds gunner policy after successful qualification; it does
not rewrite the retained spawn, Ground or lifecycle inputs.

Set `core.ground_turrets` in the selected Domination gameplay configuration to
the projected rules file. The direct loader and match-profile registry both
validate the rules against the attached item/combat catalogs. Hospital turret
configuration remains independent and can be used alongside APC gunners.

For combat projection, add the three authored APC primary weapons to the optional
`ground_apc_bullets` selection. These receive the `ground_apc_primary` usage.
Ordinary, hospital and APC selections must be disjoint. Mounted-only rows cannot
authorize saved kits or ordinary on-foot fire.

## Control and lifecycle

The gunner uses common turret owner field6; the authenticated driver alone can
control motion through common owner field1. Without a driver, field1 stays
server-owned under the replacement policy. The original game can assign common
ownership to its first occupant; that metadata fallback does not grant motion
authority in this server.

Gunner position uses the original model's named `turret_01` seat offset composed
with the vehicle's current body pose. It does not use the hospital root fallback
or the APC's initial map position after the vehicle moves.

Each new grant clears equipment readiness and uses a fresh definition version.
Current reservation, life, exact seat and operational vehicle authority are
required for aim and fire. Seat release and vehicle/life transitions revoke that
authority. The match-lifetime grant counter is retained through vehicle respawn;
exhaustion refuses a new mount without changing occupancy.

The family1 owner decoder consumes the common turret and driver fields in one
list, then authorizes their changes independently. Initial Ground creation
includes current turret definition and state. Shared seat updates retain FIFO
delivery behind creation, with existing record-before-install guarantees and
packet bounds.

## Supported damage and acceptance

Only the initialized primary, gear0, ammo pool0 and fire mode0 can enter a damage
transaction. Character hits use existing armor, life, kill/stat and rewards.
Domination type3 and type7 Ground targets use the existing vehicle damage,
penetration, team, armor and body calculation, with recorded health/lifecycle
publication. The current source APC armor absorbs these mounted primaries;
operational type7 bodies can take damage. Their player seats remain unavailable.

An initialized Domination APC gunner can also hit an operational Helo and
publish its damage after Helo creation. The current authored APCs and Helos
both belong to team 0, so these hits apply the configured friendly-fire
multiplier before armor.

Ground and Helo Hit accept authenticated TCP or UDP framing as replacement
compatibility policy, including on-foot shooters. The retail reliable small-message
dispatch is verified; its final platform socket selection remains unresolved.
Owner aim and motion reports keep their existing UDP route. The hit wire has no
unique shot/grant identifier; current role checks do not claim to classify
arbitrary unseen hit replays.

Type7 gunner seats, secondary rockets and Objective targets require separate
support. Reference seat transforms do not establish live model animation or
muzzle placement. Native controls, first-report timing, visuals and presentation
still require live-match review before a human cuts a release.
