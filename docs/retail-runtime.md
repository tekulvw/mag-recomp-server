# Explicit retail gameplay startup

Select application `20231` explicitly when starting the retail gameplay bridge.
The selected application becomes the initial wire profile and is fixed for the
life of that runtime instance. A later `ConfigureMatch` request for another
application fails. The legacy mode without `--application-id` remains available
for existing prototype integrations; it is not the retail startup contract.

Retail startup requires all nine inputs below as explicit absolute paths. There
are no prototype sibling-file defaults for these inputs when retail is selected.
Catalogs stay outside the public source tree and container image.

| Gameplay argument | External input |
| --- | --- |
| `--item-catalog` | Raw retail item catalog, schema 2/application 20231 |
| `--spawn-catalog` | Selected neutral runtime spawn catalog and explicit match feature policy |
| `--wire-profiles` | Wire profile catalog containing application 20231; a retail-only catalog is accepted |
| `--character-blast-rules` | Qualified shared character blast rules accepted for the selected application |
| `--bootstrap-loadouts` | Retail starter equipment for all three supported factions |
| `--retail-combat-metadata` | Retail combat projection matching the raw item catalog |
| `--retail-armory-catalog` | Retail saved-equipment projection matching the raw item catalog |
| `--skill-catalog` | Raw retail skill table, schema 2/application 20231, for owned character modifiers |
| `--fall-damage-rules` | Retail falling-damage rules; this input is required and falling damage is enabled |

## Validate privately before starting services

Set `CATALOG_DIR` to an absolute directory containing the operator-selected
catalogs. The following filenames illustrate the input roles; choose filenames
that match the private extraction and selected map.

```sh
: "${CATALOG_DIR:?Set an absolute private catalog directory}"

mag-gameplay --application-id 20231 --validate-catalogs \
  --item-catalog "$CATALOG_DIR/retail-items.json" \
  --spawn-catalog "$CATALOG_DIR/selected-retail-spawns.json" \
  --wire-profiles "$CATALOG_DIR/wire-profiles.json" \
  --character-blast-rules "$CATALOG_DIR/character-blast-rules.json" \
  --bootstrap-loadouts "$CATALOG_DIR/retail-all-factions-bootstrap-loadouts.json" \
  --retail-combat-metadata "$CATALOG_DIR/retail-combat.json" \
  --retail-armory-catalog "$CATALOG_DIR/retail-armory.json" \
  --skill-catalog "$CATALOG_DIR/retail-skills.json" \
  --fall-damage-rules "$CATALOG_DIR/retail-falling-damage.json"
```

`--validate-catalogs` loads and validates the selected catalogs, then exits. It
requires `--application-id`, opens no gameplay listener, starts no world
scheduler, and does not create gameplay or telemetry logs. Omit `--log` and
`--tick-telemetry`; supplying either is an error. A successful receipt has
`event: "catalogs_validated"` and identifies its scope as schema and profile
validation, without claiming client acceptance.

Do not combine the validator with `--collision-query`; diagnostic queries are a
separate command. When an explicit application selects an optional collision
world, its map must match the selected spawn catalog in both offline validation
and normal startup.

Validation checks the selected item and skill applications, the wire profile,
starter equipment availability for all factions, and starter equipment validity
against the retail Armory. Bootstrap, combat, Armory and raw skill metadata must
identify the SHA-256 of the exact selected raw item JSON bytes. Changing those
bytes requires matching projections; a shared application ID alone is
insufficient. These links check consistency between supplied files. They do not
independently prove that the files were extracted from the intended originals.

Every populated starter and saved-equipment slot must also resolve through the
raw retail item catalog. Armory membership alone cannot accept an unresolved raw
item definition. Admission and equipment replication enforce this check across
all five inventories, including inventories that are not currently selected.
Unresolved item IDs fail explicitly; they are not renamed or converted.

The neutral spawn schema does not encode an application identity. Operators must
verify its source provenance, selected map, match policy and runtime identity
evidence separately. A successful application/profile check cannot establish
that a neutral spawn file came from retail. Existing retail entry catalogs can
retain unverified pool indices and pre-terrain-snap positions; parsing them does
not upgrade that evidence.

When ready for an authorized service start, use the same explicit inputs, remove
`--validate-catalogs`, and add `--log` pointing to a new runtime log outside the
catalog directory. The executable refuses to overwrite an existing log. Choose
the bind address and any additional optional feature inputs through the private
deployment configuration. This document does not change chart interfaces or
version pins.

## Enabled features and remaining limits

Retail bunker authority is unsupported in this startup mode. The selected match
profile must explicitly disable bunkers. Do not supply `--structure-catalog`,
`--bunker-catalog` or `--bunker-exit-policy`; these inputs are rejected for retail.
The runtime carries no bunker world and reports the feature as disabled because
its authority is unverified. It still publishes spawn-pool setup. This is not a
claim that the original map contains no bunkers.

Objectives require a compatible explicit objective catalog whenever the selected
match policy enables them. The offline validator checks the objective map and
wire profile before returning success. An entry policy that disables objectives
does not establish complete original match behavior.

Falling damage uses the required retail rules. Other combat behavior remains
bounded by the authority declarations and implemented paths in the supplied
combat catalog. Candidate ammo tokens do not become a generally verified live
ammo table, and unresolved authored item definitions remain unsupported. The
runtime and validator make no claim that all retail items, protocol operations or gameplay features are resolved. Private
client acceptance remains separate from public synthetic checks.

## Gameplay skills and Medius skill purchases are different inputs

Gameplay's `--skill-catalog` consumes the **raw retail skill table**, commonly
named `retail-skills.json`. It resolves the modifiers for a character's already
owned skill bits.

Medius uses `MAG_RETAIL_SKILL_CATALOG` for the **retail skill purchase projection**,
commonly named `retail-skill-purchases.json`. That projection includes purchase
tiers, costs and qualified equipment-award bindings. Supplying the raw modifier
table there is insufficient. Prepare it with the separate
[retail skill purchase projector](native-projectors.md#retail-skill-purchases).
Medius selects application 20231 through its `MagApplicationId` configuration and
uses `MAG_RETAIL_ARMORY_CATALOG` for retail saved equipment. Prototype
`MAG_SKILL_CATALOG` and `MAG_LOADOUT_DEFAULTS` inputs do not substitute for those
retail projections.

Use the private bundle workflow in [data-bundles.md](data-bundles.md) to select
and distribute the exact inputs needed by each service. The bundle tool does
not infer these distinctions from filenames.
