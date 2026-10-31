# Native resource projectors

These offline tools complete several extraction and projection steps described in
[the catalog inventory](../tools/extraction/catalog-manifest.json). Every input and
output path is explicit. They do not execute game code, start a game, inspect a
running process, generate disassembly, or acquire native capture evidence. Keep
original inputs and generated outputs in an operator-owned location outside this
repository. No resulting catalog or proof bundle is included in the release.

The distinction between extraction and projection matters here. The falling
extractor reads supported original files directly. Structure projection also needs
external native rules. Blast, bootstrap, combat and purchase projection require
qualified external evidence; the acquisition and semantic analysis needed to
produce that evidence are not supplied. An owner attestation is trusted input,
not independent process verification. Synthetic tests demonstrate the code's
contracts and do not establish game compatibility.

All commands below are run with Python 3.11 or newer from the repository root.
Their dependencies are the Python standard library and neighboring public parser
modules. Output files are created exclusively: existing files are preserved.
`--check` on a projector compares an existing output byte for byte without writing.
The binding comparator creates a fresh review report and has no `--check` mode.

## Prototype structures

```sh
python3 tools/extraction/extract-structure-catalog.py \
  --assets INPUT_SCRIPTS --items INPUT_ITEMS \
  --native-rules INPUT_NATIVE_RULES --output OUTPUT_STRUCTURES
```

`INPUT_SCRIPTS` is the prototype `scripts` directory. The tool requires its sibling
`settings` directory and the source members listed in its `SOURCES` tuple. It
parses literal Lua/ZPR, projects equipment references, authored durability,
repair metadata and ammunition, and retains source hashes and literal provenance.
It never executes Lua or substitutes a missing durability profile.

`INPUT_NATIVE_RULES` is an externally qualified JSON object with
`health_condition_threshold_percent` (three descending finite percentages),
`dead_below_health`, `minimum_rebuilt_health_percent`, and any native provenance
needed by the operator. The native values are absent from authored resources and
are not bundled. The public runtime loader applies its own validation too.
The output is `prototype-structures.json`.

## Character blast preparation

```sh
python3 tools/extraction/project-character-blast-rules.py \
  --evidence INPUT_WITNESS_DIRECTORY --checker INPUT_ORIGINAL_CHECKER \
  --output OUTPUT_BLAST_RULES
```

This produces `character-blast-rules.json` only from the exact qualified witness
bytes identified by the tool's SHA256 pins. Required relative witness names are
`comparison.json`, `blast-callback.json`, `blast-armor-literal.json`, `toc-cells.json`
and `dev/dev-blast-armor-literal.json`. The external original checker source must
match the hash recorded by the pinned comparison report. That checker and the
witness acquisition tools are not supplied here. The projector verifies callback
range continuity and byte digests, literal cell consistency, original build
identities, TOC associations and agreement with the qualified comparison finding.
The source contains no original instruction-word table.

A bounded helper can create the DEV literal witness from its exact hash-qualified
original file without executing it:

```sh
python3 tools/extraction/project-character-blast-rules.py \
  --retain-dev-literal --dev-elf INPUT_DEV_ELF --output OUTPUT_DEV_LITERAL
```

This helper does not create the remaining witnesses or the comparison finding.
The complete witness bundle remains an external prerequisite.

## Native item bindings and bootstrap equipment

```sh
python3 tools/extraction/compare-native-item-catalog.py \
  --snapshot INPUT_NATIVE_SNAPSHOT --catalog INPUT_RETAIL_ITEMS \
  --owner-attestation INPUT_ATTESTATION --output OUTPUT_BINDING_REPORT

python3 tools/extraction/project-retail-bootstrap.py \
  --snapshot INPUT_NATIVE_SNAPSHOT --catalog INPUT_RETAIL_ITEMS \
  --owner-attestation INPUT_ATTESTATION --rules INPUT_RETAIL_RULES \
  --gear-slots INPUT_QUALIFIED_SLOT_ENUM --class-ref CharacterClassInfoN \
  --faction FACTION_NUMBER --output OUTPUT_BOOTSTRAP
```

The comparator requires the complete supported four-list native snapshot, the
matching authored item catalog, and an owner attestation tied to its exact digest
and original build. It checks every position and emits bindings only on a complete
match. Missing, altered, reordered or unattested inputs fail. Its report describes
only these supplied inputs; it does not establish a universal retail hash rule.

The bootstrap projector invokes that comparator, checks catalog/rule provenance,
and requires the exact hash-qualified gear-slot enumeration. It resolves three
starter inventories for the explicitly selected class and numeric faction. These
inputs are not acquired from a live process by either tool. Single-faction and
multiple-faction outputs follow the same `retail-all-factions-bootstrap-loadouts.json`
runtime schema; use multiple-faction mode when the runtime needs all three.

For multiple factions, repeat `--faction-class NUMBER:CharacterClassInfoN` for all
three mappings instead of `--class-ref` and `--faction`. Also supply
`--faction-enum INPUT_QUALIFIED_FACTION_ENUM` and
`--faction-identities INPUT_IDENTITIES`. The identities JSON maps numeric faction
keys to `[enum_label, display_name, authored_level_name, authored_display_name]`.
The enum has a pinned digest. The projector checks faction numbers against it and
checks authored class and display-level identities against the explicit rules
catalog. No built-in map or display-name table is supplied.

## Retail combat

```sh
python3 tools/extraction/project-retail-combat.py \
  --catalog INPUT_RETAIL_ITEMS --bindings INPUT_BINDING_REPORT \
  --bootstrap INPUT_BOOTSTRAP --bullet-witness INPUT_QUALIFIED_BULLET_PROOF \
  --melee-witness INPUT_QUALIFIED_MELEE_PROOF --selection INPUT_SELECTION \
  --runtime-policy INPUT_SERVER_POLICY --output OUTPUT_COMBAT
```

This produces `retail-combat.json`. Binding, bootstrap and bullet-witness inputs
must match the exact retained evidence digests compiled into the projector. This
retains the existing evidence qualification boundary; editing or regenerating an
upstream artifact does not silently qualify a new artifact. The melee witness is
external trusted native analysis. `INPUT_SELECTION` contains four nonempty,
duplicate-free equipment-name arrays: `bullets`, `melee`, `grenades`, and `healing`,
plus `melee_witness_sha256`. Choose names from the explicitly supplied authored
catalog and independently qualify the selection against the native witnesses.
The tool resolves unique definitions and validates numeric, timing, range,
body and binding constraints. Selection does not constitute new native proof.

`INPUT_SERVER_POLICY` supplies the existing runtime policy fields:
`projectile_motion`, `grenade_token_basis`, `projectile_expiry_grace_ms`,
`healing_tick_policy` and `healing_tick_seconds`. String policy identifiers must
match the runtime's supported modes; the expiry grace must be an integer from
zero through 10000 milliseconds and the supported healing cadence is the integer
one second. No weapon balance, equipment-name selection list or expiry grace
default is embedded in the public tool. Output provenance records the selection
and policy digests. Native physics, collision and live acceptance remain separate
questions.

## Retail falling damage

```sh
python3 tools/extraction/extract-retail-falling-damage.py \
  --elf INPUT_RETAIL_ELF --game-archive INPUT_RETAIL_ARCHIVE \
  --config-output OUTPUT_FALLING_RULES
```

This directly produces `retail-falling-damage.json` from the exact supported
retail ELF and decrypted archive identified by SHA256. The ELF reader requires
PPC64 big-endian encoding, bounded program headers, valid file-backed segments
and unique bounded address mappings. The extractor parses the archive's
`server.zpr` using the public literal parser and reads the existing supported-build
scalar offsets. It checks runtime field bounds and finite numeric values.
Unsupported or changed files fail before extraction. No DEV ELF, debugger,
disassembler, copied instruction verifier, debug string or raw native witness
output is needed or generated. The fixed layout is not a general ELF analyzer.

## Retail skill purchases

```sh
python3 tools/extraction/extract-retail-skill-purchases.py \
  --inputs INPUT_QUALIFIED_FILE_MANIFEST --output OUTPUT_SKILL_PURCHASES
```

This produces `retail-skill-purchases.json`. `--inputs` is a JSON object mapping
each logical source key below to an explicit file path. These are schema keys,
not implicit locations in an installed checkout. Each file must match its pinned
SHA256; the acquisition of these proofs is not supplied.

- `gameplay/evidence/retail-skill-unlocks-native-20261026.json`
- `gameplay/data/retail-skills.json`
- `gameplay/data/retail-items.json`
- `gameplay/data/retail-item-bindings.json`
- `gameplay/data/retail-itemlist-native-order.json`
- `gameplay/evidence/dev-itemlist-globalindex-49242c-20261024.json`

The projector validates source identity, catalog bindings, purchase bounds and
native equipment-bit semantics. It preserves authored costs, tiers and modifiers,
projects the first matching native item ID to its available-item bit, and records
missing native tokens as skipped equipment awards. It does not invent a binding.
There is one explicit output; it does not also write a hidden fixture copy.

## Verification and provenance

Run `python3 -B -m unittest discover -s tests -p 'test_native_projectors.py'` and
`python3 -B tools/check_public_tree.py`. Tests build invented resources, scalars,
ELF segments and PSARC members in temporary directories. Some positive tests
repin a fresh in-memory module instance to their synthetic inputs; production
commands have no pin-override option. Source precursor fingerprints are recorded
in [PROVENANCE.md](../tools/extraction/PROVENANCE.md). Retaining tool source and
hash identifiers conveys no rights in externally supplied game material.
