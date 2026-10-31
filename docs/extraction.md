# Regenerating local catalogs

The public repository distributes extraction **source code**, synthetic test inputs and schemas. It does not distribute game archives, executables, authored resource tables, captures, decompiler output, extracted geometry or populated runtime catalogs. Use original material you are entitled to access and keep every generated output outside the source checkout, for example `/external/generated-data/` or `/external/extracted/`. If you deliberately generate inside a checkout, use only an ignored directory and verify the ignore rule first; the publication guard rejects populated game catalogs and binary outputs. The names below are filenames, not downloadable game data.

Python tools require Python 3.11 or newer and its standard library. Native game execution, RPCS3, Ghidra and Lua are not needed for the direct archive/literal parsers. Some **external prerequisites** were originally established through native analysis or qualified captures: their acquisition is not included here. Ghidra installations, project databases, DWARF tooling, a decrypted executable or normalized exports may be needed to establish those independently. Merely possessing an archive does not establish runtime pool IDs, numeric wire bindings, native damage rules or accepted gameplay behavior.

The full [machine-readable catalog manifest](../tools/extraction/catalog-manifest.json) inventories all **53** known JSON filenames with a command, inputs, dependencies, status and limitation. “Direct” means an included parser can regenerate the stated data from explicit source inputs or a preceding included catalog. “External proof/export” means an included projector works only after the operator supplies a separately qualified input. “Gap” means the complete regeneration pipeline is not included. The inventory distinguishes these cases; it is not a claim of complete retail setup from a disc alone.

## Direct retail archive extraction

Run commands from the repository root. Capitalized arguments stand for your explicit external paths. Create output parent directories first. The readers are pinned to the original supported retail build; another update or DLC is not silently treated as equivalent.

```sh
python3 tools/extraction/extract-item-catalog.py --game-archive GAME --output ITEMS
python3 tools/extraction/extract-retail-rules.py --game-archive GAME --items ITEMS --output RULES --coverage-output COVERAGE
python3 tools/extraction/extract-retail-maps.py --disc DISC --game-archive GAME --output MAPS
python3 tools/extraction/extract-retail-spawns.py --disc DISC --game-archive GAME --output SPAWN_METADATA_DIRECTORY
python3 tools/extraction/build-retail-spawn-catalog.py --disc DISC --game-archive GAME --map MAP_NAME --active TEAM/PLATOON/SQUAD --output SPAWNS
python3 tools/extraction/project-retail-sabotage-objectives.py --disc DISC --game-archive GAME --output OBJECTIVE_METADATA
python3 tools/extraction/project-retail-abilities.py --rules RULES --output ABILITIES
python3 tools/extraction/extract-retail-mediation-profile.py --elf RETAIL_ELF --output MEDIATION_PROFILE
```

`DISC` contains `PS3_GAME/USRDIR/run/mag/ps3/levels/*.psarc`; `GAME` is the externally decrypted global `game.psarc`. `ITEMS` contains namespace-qualified authored identities. It is **not** a complete numeric runtime item binding: retail faction lists restart their local ordinals. The runtime needs qualified numeric bindings and appropriate bootstrap data for those features. Those upstream binding/bootstrap steps remain gaps in this release.

The spawn builder supports Suppression and SVER Sabotage entry. Pass the exact authored map name and an explicit team/platoon/squad selection. It preserves authored coordinates before terrain snapping and marks runtime indices unverified. Objectives remain disabled in its match policy. The all-map metadata extractor does not turn authored bunker exit candidates into grounded, collision-safe exits. Unknown objective fields stay typed/raw rather than acquiring guessed meanings. The objective metadata projector inventories a bounded subset of script calls/conditions without executing Lua or reconstructing general branch semantics.

The mediation profile extracts a protocol word through ELF load mappings. Its disabled character-catalog grants are explicit policy. These tools do not decrypt SELF/PSARC inputs or obtain game files.

## Projections requiring external evidence or policy

These commands are useful stages but do not acquire or verify the complete upstream native evidence:

```sh
python3 tools/extraction/extract-retail-catalogs.py --game-archive GAME --items ITEMS --native-rules STRUCTURE_NATIVE_RULES --skills-output SKILLS --structures-output STRUCTURES
python3 tools/extraction/extract-retail-progression-catalog.py --game-archive GAME --witness TRAINING_WITNESS --output PROGRESSION
python3 tools/extraction/project-retail-leadership-passives.py --rules RULES --native-witness LEADERSHIP_WITNESS --output PASSIVES
python3 tools/extraction/project-retail-leadership-role-policy.py --rules RULES --default-record LOCAL_RECORD --output ROLE_POLICY
python3 tools/extraction/extract-retail-armory.py --catalog-dir EXTERNAL_CATALOG_DIRECTORY --output ARMORY
python3 tools/extraction/project-retail-objective-runtime.py --catalog OBJECTIVE_METADATA --bindings TOKEN_SLOT_BINDINGS --startup-proof STARTUP_WITNESS --rules RULES --output OBJECTIVE_RUNTIME
```

`STRUCTURE_NATIVE_RULES` is the externally verified `native_rules` object used by the structure loader, including health-condition thresholds, dead/rebuild thresholds and provenance. Its constants cannot be recovered from the selected authored resource tables. The combined skill/structure command requires this object for the structure output; it does not substitute prototype constants for retail.

`TRAINING_WITNESS` supplies `sources.decrypted_game_archive_sha256`, `authored_reward.experience_zpr_sha256`, `authored_reward.experience_bonus`, `authored_reward.scale` and `native_flag.mask`. The projector compares archive/member identities and authored reward/scale values. It does not derive the native completion flag from Lua.

`LEADERSHIP_WITNESS` has schema version 1, application ID 20231, a supported `retail_elf_sha256` and `consumer_constants.refresh_hold_ms`/`removal_grace_ms`. `LOCAL_RECORD` contains only `leadership_level`, `nominations` and `matches_since_last_command`; choose these fallback-policy values deliberately. The role-policy projector reads election weights from rules.

The armory projector expects `retail-items.json`, `retail-item-bindings.json`, `retail-all-factions-bootstrap-loadouts.json` and `retail-skill-purchases.json` in the supplied directory. It validates relationships but does not create the missing native bindings or owner-qualified starter records.

`TOKEN_SLOT_BINDINGS` uses format `mag_retail_objective_slot_bindings`, schema version 1, the supported application/map/entity-family identity, qualification `bounded_concurrent_token_slot_capture`, source-image/capture hashes and reviewed `objects` with token, object_index and packed_id. `STARTUP_WITNESS` supplies the separately qualified desired-state mapping and Active-gate dataflow accepted by `project-retail-objective-runtime.py`. Source catalog/cardinality/token/slot consistency is checked; JSON claims are not independent proof of the capture. Add `--include-interactions` for interaction/scoring projection. `--interaction-radius-m` and `--local-end-game-delay-ms` are explicit local policies, not extracted balance witnesses. `project-retail-objective-observer.py` similarly consumes an external spawn-observer configuration and explicit pool/binding/startup witnesses; it writes configuration and never attaches to a running client.

The exact input contracts and checked fields are defined by the included projector functions. No raw captures, game-body/decompiler exports, native evidence reports or automatically generated observer roots are bundled.

## Collision and navigation pipeline

First generate the all-map metadata `MAPS` above. Then convert map resources into neutral local geometry:

```sh
python3 tools/collision/extract-retail.py --disc DISC --manifest MAPS --output COLLISION_CATALOG --summary-output COLLISION_SUMMARY
python3 tools/collision/instance_linkage.py --archive LEVEL_PSARC --map MAP_NAME --output INSTANCE_JOIN_JSON_GZ
```

The converter writes `coverage.json`, per-map manifests under `maps/retail-bcus98110-01.00/`, and neutral geometry under `geometry/`. It scans bounded ZRB/HSX/HKX/HKT members, validates declared packfile/resource ownership and preserves unsupported cases in reports. It emits no raw resource copies. Shared `main.psarc` resources are outside this map-only conversion. The instance reader writes a compressed JSON join from complete CLV instance records and native resource headers; identifiers remain opaque where names are unavailable.

This first stage is **resource-local conversion**, not a collision world. Retail terrain/compiled adapters currently support the original Cosmodrome map and require independently established placement evidence. They read original resource members directly from `LEVEL_PSARC`; there is no hidden dependency on retained raw-member files or an unpublished manifest augmentation:

```sh
python3 tools/collision/retail_terrain.py --archive LEVEL_PSARC --terrain-proof TERRAIN_PROOF --retail-elf RETAIL_ELF --map-manifest MAP_RESOURCE_MANIFEST --catalog COLLISION_CATALOG --output TERRAIN_WORLD_DIRECTORY
python3 tools/collision/retail_compiled.py --archive LEVEL_PSARC --terrain-proof TERRAIN_PROOF --placement-proof PLACEMENT_PROOF --retail-elf RETAIL_ELF --map-manifest MAP_RESOURCE_MANIFEST --instance-join INSTANCE_JOIN_JSON_GZ --catalog COLLISION_CATALOG --output STATIC_WORLD_DIRECTORY
```

`TERRAIN_PROOF` uses schema `mag_retail_terrain_identity_v1` with supported build/map, `source_elf_sha256` and identity `world_transform`. `PLACEMENT_PROOF` uses schema `mag_retail_compiled_placement_v1` with supported `source_elf_sha256`, `functions` and `call_chain`. These adapters verify the supplied proof identities against the original ELF, **not the native-analysis assertions inside the proof**. Public native-proof generation is a documented gap. Operators must independently establish those witnesses before using the adapters. Existing embedded native instruction verifiers were intentionally excluded from the source release.

The compiled adapter compares the supplied instance join with full regeneration from the archive, validates CLV/mesh/source hashes and refuses unresolved compiled resources. It applies compiled matrices once, preserves all neutral faces and removes only exact degenerate triangles. Material/filter parity and later moving/destructible/entity-created collision remain unresolved. An optional bounded region is supplied with both `--physics-region-min x,y,z` and `--physics-region-max x,y,z`; declaring that region is an operator assertion about initial static coverage, not live physics acceptance. Without such a region, navigation preparation refuses to infer coverage.

Once an independently qualified world has a verified static coverage region:

```sh
python3 tools/navmesh/prepare.py --world WORLD_JSON --expected-world-sha256 WORLD_SHA256 --output BAKE_DIRECTORY
cargo run --manifest-path gameplay/Cargo.toml --bin mag-navmesh -- bake --input BAKE_DIRECTORY/input.json --output NAVMESH_ARTIFACT
```

The preparation tool preserves winding and all faces, rounds vertices to f32, computes a domain-separated geometry hash, and refuses invalid bounds, degenerate triangles or unverified coverage. Baking needs the Rust/C++ toolchain and vendored Recast/Detour described in the build instructions. It produces a local `.mag-nav` JSON artifact accepted by `--navmesh-world`; it does not need a running game or server. The runtime independently binds the navmesh to the collision-world hash. No retail geometry/navmesh is distributed.

## Prototype projections

```sh
python3 tools/extraction/extract-item-catalog.py --assets PROTOTYPE_SCRIPTS --elf PROTOTYPE_ELF --token-proof TOKEN_PROOF --output PROTOTYPE_ITEMS
python3 tools/extraction/extract-skill-catalog.py --source CHARACTER_SKILLS_ZPR --items PROTOTYPE_ITEMS --output PROTOTYPE_SKILLS
python3 tools/extraction/extract-loadout-defaults.py --source DEFAULTS_EXPORT --output PROTOTYPE_LOADOUTS
python3 tools/extraction/extract-character-grants.py --skills CHARACTER_SKILLS_ZPR --items PROTOTYPE_ITEMS --loadouts PROTOTYPE_LOADOUTS --output PROTOTYPE_GRANTS
python3 tools/extraction/extract-progression-catalog.py --assets PROTOTYPE_RESOURCE_ROOT --output PROTOTYPE_PROGRESSION
python3 tools/extraction/extract-bunker-runtime-catalog.py --evidence EXPORT_BUNDLE --active-policy ACTIVE_POLICY --output-dir PROTOTYPE_OUTPUT_DIRECTORY
```

`PROTOTYPE_RESOURCE_ROOT` contains `scripts/` and `settings/`. The item projector requires the original supported prototype ELF and a token proof with `source_elf_sha256`, `token_proof.all_256_words_match_reflected_polynomial_0xedb88320` and checked `profiles`. Each profile supplies authored ammo name/native token plus weapon/range/damage comparison fields. The proof generator is not bundled. Unknown Lua syntax fails rather than executing code.

`DEFAULTS_EXPORT` contains the qualified `faction`, `class`, `source_sha256` and `loadouts`; each loadout has contiguous index, update_id and 22 ordered slot/item_id rows. The projector checks this normalized export; it does not acquire native loadout initialization evidence.

The prototype spawn/bunker projector expects this external bundle layout:

```text
bunker-exit-catalog/catalog.json
bunker-exit-catalog/MAP_NAME.json
spawn-catalog/squad-spawns.json
spawn-catalog/MAP_NAME.json
bunker-turret/verified-spawn66.json
objective-runtime/verified-objective-ids.json
bunker-damage-repair/spawn-pool-binding-proof.json
```

It supports the two bundled prototype maps identified in its source. These named files are **external prerequisites**, not files secretly present in the public repository. Raw upstream extraction/capture qualification is not included. `ACTIVE_POLICY` explicitly supplies `map_name`, `squad_index`, `default_spawn` and `available`; the projector checks every selection against supplied map/squad exports. It does not embed the earlier test location or choose a retail policy from prototype evidence.

## Coverage and checks

Required runtime inputs depend on the selected features. The gameplay executable requires item and spawn catalogs; optional combat, objectives, skills, armory, leadership, collision and navmesh flags enable additional content paths. Medius loaders define their own schemas for loadouts, skills, progression, armory, statistics and mediation. The prototype character-grants loader looks beside the supplied loadout defaults for `prototype-character-grants.json`. Synthetic public examples are for tests. They do not recreate playable content.

Additional structure, blast, combat, bootstrap, binding, purchase and falling-damage tooling is documented in [native-projectors.md](native-projectors.md). Falling damage has a direct supported-build extractor; several other projectors require qualified native proofs or snapshots whose acquisition remains external. Movement/mode metadata and remaining diagnostic outputs are identified separately in the inventory. Missing native evidence is never filled by an inferred authored ordinal or default balance table.

```sh
python3 -m unittest discover -s tests -p test_extraction.py -v
python3 tools/check_public_tree.py
```

Checks exercise newly invented Lua tables, PSARC resources, ZRB headers, transforms, default-export rows and neutral geometry, plus command help/imports. They do not run a game, extract real game data, validate external witness truth or establish live acceptance. Source adaptation history and precursor source-code digests are recorded in [PROVENANCE.md](../tools/extraction/PROVENANCE.md).

## All catalog filenames

| Catalog | Status | Included command or remaining input |
| --- | --- | --- |
| `bot-names.json` | operator-configuration | Operator-authored policy/protocol configuration; consult runtime loaders and synthetic public examples |
| `bot-objective-policy.json` | operator-configuration | Operator-authored policy/protocol configuration; consult runtime loaders and synthetic public examples |
| `character-blast-rules.json` | external-proof | `project-character-blast-rules.py` |
| `development-bot-script.json` | operator-configuration | Operator-authored policy/protocol configuration; consult runtime loaders and synthetic public examples |
| `local-development-progression.json` | operator-configuration | Operator-authored policy/protocol configuration; consult runtime loaders and synthetic public examples |
| `local-owned-skill-trial.json` | operator-configuration | Operator-authored policy/protocol configuration; consult runtime loaders and synthetic public examples |
| `local-xp-trial.json` | operator-configuration | Operator-authored policy/protocol configuration; consult runtime loaders and synthetic public examples |
| `prototype-bunkers.json` | external-export | `extract-bunker-runtime-catalog.py` |
| `prototype-character-grants.json` | external-export | `extract-character-grants.py` |
| `prototype-items.json` | external-proof | `extract-item-catalog.py` |
| `prototype-loadouts.json` | external-export | `extract-loadout-defaults.py` |
| `prototype-mediation-profile.json` | operator-configuration | Operator-authored policy/protocol configuration; consult runtime loaders and synthetic public examples |
| `prototype-progression.json` | direct | `extract-progression-catalog.py` |
| `prototype-skills.json` | direct | `extract-skill-catalog.py` |
| `prototype-spawns.json` | external-export | `extract-bunker-runtime-catalog.py` |
| `prototype-structures.json` | external-proof | `extract-structure-catalog.py` |
| `retail-abilities.json` | direct | `project-retail-abilities.py` |
| `retail-all-factions-bootstrap-loadouts.json` | external-proof | `project-retail-bootstrap.py` |
| `retail-armory.json` | external-proof | `extract-retail-armory.py` |
| `retail-collision-resources.json` | direct | `tools/collision/extract-retail.py` |
| `retail-combat.json` | external-proof | `project-retail-combat.py` |
| `retail-extraction-coverage.json` | direct | `extract-retail-rules.py` |
| `retail-extraction-manifest.json` | gap | Hashes and provenance for a completed operator extraction run |
| `retail-falling-damage.json` | direct | `extract-retail-falling-damage.py` |
| `retail-item-bindings.json` | external-proof | `compare-native-item-catalog.py` |
| `retail-itemlist-native-order.json` | gap | Owner-qualified native token-order snapshots, binding attestation and starter-record evidence |
| `retail-items.json` | direct | `extract-item-catalog.py` |
| `retail-leadership-passives.json` | external-proof | `project-retail-leadership-passives.py` |
| `retail-leadership-role-policy-native.json` | direct-with-policy | `project-retail-leadership-role-policy.py` |
| `retail-leadership-role-policy.json` | operator-configuration | Operator-selected development leadership assignments |
| `retail-maps.json` | direct | `extract-retail-maps.py` |
| `retail-mode-gates.json` | gap | Independent native semantic evidence and original authored data where applicable |
| `retail-movement.json` | gap | Independent native semantic evidence and original authored data where applicable |
| `retail-progression.json` | external-proof | `extract-retail-progression-catalog.py` |
| `retail-rules.json` | direct | `extract-retail-rules.py` |
| `retail-sabotage-objective-observer.json` | external-proof | `project-retail-objective-observer.py` |
| `retail-sabotage-spawn-observer.json` | gap | Native spawn-manager root, TOC/function and allocation-policy witnesses |
| `retail-skill-purchases.json` | external-proof | `extract-retail-skill-purchases.py` |
| `retail-skills.json` | external-proof | `extract-retail-catalogs.py` |
| `retail-structures.json` | external-proof | `extract-retail-catalogs.py` |
| `retail-sver-bootstrap-loadouts.json` | external-proof | `project-retail-bootstrap.py` |
| `retail-sver-sab-defuse-fixture.json` | gap | Qualified objective catalog and explicit private test progression state |
| `retail-sver-sab-defuse-fixture.report.json` | gap | Qualified objective catalog and explicit private test progression state |
| `retail-sver-sab-local-objective-interactions.json` | external-proof | `project-retail-objective-runtime.py` |
| `retail-sver-sab-objective-interactions.json` | external-proof | `project-retail-objective-runtime.py` |
| `retail-sver-sab-objective-runtime.json` | external-proof | `project-retail-objective-runtime.py` |
| `retail-sver-sab-objectives-spawns.json` | gap | Entry spawn catalog plus qualified runtime objective activation policy |
| `retail-sver-sab-objectives.json` | direct | `project-retail-sabotage-objectives.py` |
| `retail-sver-sab-spawns.json` | direct | `build-retail-spawn-catalog.py` |
| `retail-sver-sup-spawns.json` | direct | `build-retail-spawn-catalog.py` |
| `retail-v1-mediation-profile.json` | direct | `extract-retail-mediation-profile.py` |
| `statistics-login-profiles.json` | operator-configuration | Operator-authored policy/protocol configuration; consult runtime loaders and synthetic public examples |
| `wire-profiles.json` | operator-configuration | Operator-authored policy/protocol configuration; consult runtime loaders and synthetic public examples |
