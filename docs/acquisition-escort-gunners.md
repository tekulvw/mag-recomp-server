# Acquisition Escort gunners

Acquisition can optionally enable hidden gunner seat1 on the four source-type9
Escorts in each profile. The driver remains in seat0, passengers use seats2–9,
and all occupants share the existing cap8. Reinforcement uses separate APCs.
The gunner's original HMG has independent ownership and equipment readiness.

## Owned data and configuration

Run `tools/extraction/project-retail-escort-turrets.py` with original retail files
you own and have permission to use. Supply `--elf`, `--game-archive`,
`--main-archive`, the existing `--ground-sources` source-pose catalog,
`--item-catalog` and `--item-bindings`. Repeat `--profile SPAWNS GROUND` for the
six current Acquisition profile pairs, with `--output` and optional
`--proof-output`. Repeat the same command with `--check` for exact reproduction.
The output must be outside the public checkout.

The projector resolves `turret_01` by its authored name in each original model.
It preserves the full reference parent chain and derives the weapon ID from the
attested item mapping. It does not assume the Domination APC's model node index.
Runtime rules bind the original identities and exact loaded spawn/Ground bytes.

Set top-level `ground_turrets` in the Acquisition gameplay configuration to the
projected rules path. Both direct worker startup and the match-profile registry
bind the rules after loading item metadata. The original Domination APC schema
and `core.ground_turrets` configuration remain supported independently.

Add the three authored Escort HMGs to the combat projector's optional
`ground_escort_bullets` selection. This produces `ground_escort_primary` rows.
Ordinary, hospital, APC and Escort selections must be disjoint. Some original
type7 guns share the HMG item IDs; an exact current Escort grant is still
required. Metadata membership does not enable another vehicle's seats or an
ordinary character loadout.

## Gameplay and lifecycle

Boarding requires the current operational, captured vehicle and the existing
team, life, creation, distance and occupancy checks. Source-qualified seat pose
uses the named reference offset and the live vehicle transform. Driver motion
and gunner controls remain independent. A fresh equipment grant is required
after every new mount; the counter is retained through same-match lifecycle
changes. Initial family1 creation carries current turret state for late joiners.

Retail kits with no secondary weapon still report an empty secondary item beside
the primary. The server accepts only its exact current-version descriptor, keeps
the secondary definition absent, and preserves primary readiness checks. This
also supports the same native report from Domination APC gunners.

Primary infantry hits use the existing armor, life, kill statistics and reward
transaction. Qualified Escort primaries can also use the existing Acquisition
type9 Ground damage calculation. Authenticated TCP/UDP Hit compatibility and
UDP owner reports follow the shared mounted-primary rules.

Capture follows the existing mode rules. Delivery still uses the driver for its
arrival credit, ejects both occupants at qualified exits, revokes gunner
readiness and clears turret ownership in the same recorded state8 transition.
A failed record leaves state, seats, score, queues and grants uncommitted.

Helo/Objective targets, type7 player seats and secondary weapons require separate
support. Original-data projection and source Engine replay do not establish
native first-report timing, input or visual acceptance. Live-match review and
release cutting remain with the human.
