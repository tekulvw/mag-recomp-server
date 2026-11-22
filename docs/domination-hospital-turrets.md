# Domination hospital primary turrets

Indexed hospital turrets are an optional part of the Domination gameplay
configuration. They support the original hidden operator seat, primary hitscan
damage to characters, current aim replication and late joins. Existing profiles
retain their behavior until explicitly configured.

## Projecting owned retail data

Use `tools/extraction/project-retail-hospital-turrets.py` with retail files you own
and have permission to use. Supply the original retail ELF, game archive and main
archive, the previously projected hospital placement source and structure
catalog, and the attested item catalog and bindings. Repeat `--profile SPAWNS
HOSPITALS` for all six Domination profiles. The tool reads these inputs without
modifying them and writes an external runtime rules file:

```sh
python3 tools/extraction/project-retail-hospital-turrets.py \
  --elf "$RETAIL_ELF" --game-archive "$GAME_ARCHIVE" \
  --main-archive "$MAIN_ARCHIVE" --hospital-sources "$HOSPITAL_SOURCES" \
  --item-catalog "$ITEM_CATALOG" --item-bindings "$ITEM_BINDINGS" \
  --structures "$STRUCTURE_CATALOG" \
  --profile "$SPAWNS_1" "$HOSPITALS_1" \
  --profile "$SPAWNS_2" "$HOSPITALS_2" \
  --profile "$SPAWNS_3" "$HOSPITALS_3" \
  --profile "$SPAWNS_4" "$HOSPITALS_4" \
  --profile "$SPAWNS_5" "$HOSPITALS_5" \
  --profile "$SPAWNS_6" "$HOSPITALS_6" \
  --output hospital-turrets.json --proof-output hospital-turret-proof.json
```

Repeat the same command with `--check` to verify existing output byte for byte.
The optional proof records model-node and protocol qualification. The runtime
rules bind the source identities, faction variants, hospital placements and
authored weapon identities. Hospital structure types that share a property file
are distinguished by their vehicle type.

The combat projector accepts an optional `hospital_bullets` list in its selection
file. Select the three authored hospital primary weapons alongside the ordinary
combat selection, then run the existing attested combat projection. These rows
receive the `hospital_primary` usage. The server excludes them from ordinary
character equipment, saved kits and other damage constructors.

Keep projected data and original game files private. The public tools and server
contain the consumers; they do not bundle the game assets or generated catalogs.

## Configuration and authority

Set `core.hospital_turrets` in the selected Domination gameplay configuration to
the projected rules path. Load the matching item and combat catalogs before
binding direct configuration or the match-profile registry. Both configuration
paths validate the loaded hospital world and selected spawn identity.

Only the current living defender in the exact hospital's exclusive operator seat
can report aim or fire. Each new grant receives a fresh equipment definition
version; equipment readiness is cleared on release, death, destruction and other
authority loss. Duplicate mount requests replay the committed grant. Aim-only
updates cannot initialize a new grant. Owner updates use the authenticated UDP
route and increasing connection sequence. Character hits use the existing
authenticated client route and require an active combat round.

The original hit message contains no unique shot or equipment-grant identifier.
The implementation checks current authority and equipment; it does not claim to
identify arbitrary unseen hit replays. Grant versions are not reused within a
hospital's match lifetime. Exhaustion rejects a new mount without changing
occupancy.

Character damage uses the existing armor, life, kill statistics and reward
transaction. Qualified mounted primaries can also hit current Domination type3
and type7 Ground vehicles through the existing vehicle-channel, penetration,
team, armor and body transaction. Source APC armor absorbs these rounds;
operational type7 bodies can take damage under the configured team rules.
Ground Hit accepts authenticated TCP and UDP as explicit compatibility policy;
owner reports remain UDP. Aim, ownership, equipment, damage and late-join publication retain
the existing record-before-install and bounded packet/recipient checks. Two
occupied hospitals retain independent indices and authority.

## Remaining acceptance and scope

The source qualification covers retail field widths, authored primary and
secondary definitions, the named gunner seat fallback and the hulk's last yaw.
Primary hitscan supports Characters, the scoped Domination Ground targets and
operational Helos. A current Hospital gunner can hit a Helo with an authenticated
TCP or UDP Hit, and the resulting damage update follows that recipient's Helo
creation. The on-foot Helo Hit keeps its existing UDP route.
Heat simulation, Objective targets and native live-match presentation require
their own qualification and acceptance. See [Hospital secondary rockets](hospital-secondary-rockets.md)
for the separate projectile path. The independent
APC gunner route is described in [Domination APC gunners](domination-apc-gunners.md).
The secondary definition remains present because native local weapon creation
and replicated equipment definition are separate paths.

Offline original-data reproduction and connected engine checks do not establish
native timing or visual acceptance. Review first-report timing, aiming, occupied
seat presentation, damage and lifecycle behavior in a live match before a human
cuts a release.
