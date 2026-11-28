# Regenerating local catalogs

The public repository distributes extraction **source code**, synthetic test inputs and schemas. It does not distribute game archives, executables, authored resource tables, captures, decompiler output, extracted geometry or populated runtime catalogs. Use original material you are entitled to access and keep every generated output outside the source checkout, for example `/external/generated-data/` or `/external/extracted/`. If you deliberately generate inside a checkout, use only an ignored directory and verify the ignore rule first; the publication guard rejects populated game catalogs and binary outputs. The names below are filenames, not downloadable game data.

Python tools require Python 3.11 or newer and its standard library. Native game execution, RPCS3, Ghidra and Lua are not needed for the direct archive/literal parsers. Some **external prerequisites** were originally established through native analysis or qualified captures: their acquisition is not included here. Ghidra installations, project databases, DWARF tooling, a decrypted executable or normalized exports may be needed to establish those independently. Merely possessing an archive does not establish runtime pool IDs, numeric wire bindings, native damage rules or accepted gameplay behavior.

The full [machine-readable catalog manifest](../tools/extraction/catalog-manifest.json) inventories all **57** known JSON filenames with a command, inputs, dependencies, status and limitation. “Direct” means an included parser can regenerate the stated data from explicit source inputs or a preceding included catalog. “External proof/export” means an included projector works only after the operator supplies a separately qualified input. “Gap” means the complete regeneration pipeline is not included. The inventory distinguishes these cases; it is not a claim of complete retail setup from a disc alone.

## Direct retail archive extraction

Run commands from the repository root. Capitalized arguments stand for your explicit external paths. Create output parent directories first. The readers are pinned to the original supported retail build; another update or DLC is not silently treated as equivalent.

```sh
python3 tools/extraction/extract-item-catalog.py --game-archive GAME --output ITEMS
python3 tools/extraction/extract-retail-rules.py --game-archive GAME --items ITEMS --output RULES --coverage-output COVERAGE
python3 tools/extraction/extract-retail-maps.py --disc DISC --game-archive GAME --output MAPS
python3 tools/extraction/extract-retail-domination.py --disc DISC --game-archive GAME --output DOMINATION_METADATA
python3 tools/extraction/extract-retail-spawns.py --disc DISC --game-archive GAME --output SPAWN_METADATA_DIRECTORY
python3 tools/extraction/build-retail-spawn-catalog.py --disc DISC --game-archive GAME --map MAP_NAME --active TEAM/PLATOON/SQUAD --output SPAWNS
python3 tools/extraction/project-retail-sabotage-objectives.py --disc DISC --game-archive GAME [--map SABOTAGE_MAP] --output OBJECTIVE_METADATA
python3 tools/extraction/project-retail-sabotage-structures.py --level-archive LEVEL_PSARC [--animated-root-packs ROOTS] --output STRUCTURES_DIRECTORY
python3 tools/extraction/extract-retail-objective-volumes.py --disc DISC [--game-archive GAME] --output OBJECTIVE_VOLUMES
python3 tools/extraction/project-retail-abilities.py --rules RULES --output ABILITIES
python3 tools/extraction/extract-retail-mediation-profile.py --elf RETAIL_ELF --output MEDIATION_PROFILE
```

`DISC` contains `PS3_GAME/USRDIR/run/mag/ps3/levels/*.psarc`; `GAME` is the externally decrypted global `game.psarc`. `ITEMS` contains namespace-qualified authored identities. It is **not** a complete numeric runtime item binding: retail faction lists restart their local ordinals. The runtime needs qualified numeric bindings and appropriate bootstrap data for those features. Those upstream binding/bootstrap steps remain gaps in this release.

The spawn builder supports Suppression and the three retail Sabotage maps. The archive's map configuration supplies each defender: `mag_cosmodrome_final` (SVER), `mag_kennicott_final` (Valor) and `mag_remote_comms_final` (Raven). Each map therefore has two attacker profiles, and together they cover all six ordered faction pairs. Pass the exact authored map name, an explicit team/platoon/squad selection and `--attacking-faction`. Only the legacy Cosmodrome profile defaults its attacker to Valor. Authored objective counts are pinned per Sabotage map, and a changed count is refused. It preserves authored coordinates before terrain snapping and marks runtime indices unverified. Objectives remain disabled in its match policy. The all-map metadata extractor does not turn authored bunker exit candidates into grounded, collision-safe exits. Unknown objective fields stay typed/raw rather than acquiring guessed meanings. The objective metadata projector inventories a bounded subset of script calls/conditions without executing Lua or reconstructing general branch semantics. `--map` selects one of the three Sabotage maps (default `mag_cosmodrome_final`). Each map pins its level archive and CLV hashes, its configured defender and its authored objective, control, list, platoon, trigger and forward-spawn counts. Live objective slot bindings remain a separate per-map capture: `project-retail-objective-runtime.py` refuses a map without its own qualified `TOKEN_SLOT_BINDINGS`.

### Spawn catalog capacity

`build-retail-spawn-catalog.py --capacity POLICY` chooses the world size written to `match_profile`. Every policy uses squads of 8, the retail Squad object's size:

| Policy | Sabotage, Suppression | Acquisition | Domination |
|---|---|---|---|
| `local64` (default) | team 32, squad 8 | team 32, squad 8 | team 32, squad 8 |
| `authored` | refused | team 64, squad 8 (128 players) | team 128, squad 8 (256 players) |

`authored` takes the team size from the mode's recorded `authored_maximum_players` and refuses a map whose authored squads would not seat exactly that many on each side. Acquisition and Domination catalogs record the policy in `source.admission_policy`. Sabotage and Suppression output is unchanged.

Capacity changes only the seat counts. Every policy keeps every authored squad binding at its authored address (team × 16 + platoon × 4 + squad, the address the client uses), with the same spawns and defaults: 16 squads in Acquisition (indices 0-7 and 16-23) and 32 in Domination. At `local64` a side has 32 seats and 8 (Acquisition) or 16 (Domination) squads of 8, so at most 4 squads per side are full at once. The worker chooses which. It seats each human in its side's emptiest squad, lowest index first. Bots then fill the squads that hold humans, then the fullest, then the lowest index (see [bot fill](worker-match-profiles.md)). A side with no humans therefore fills platoon 0's four squads (indices 0-3 or 16-19).

Earlier projectors wrote squad 4 for Acquisition and Domination (`Local maximum64/team32/squad4` in `source`). The default output changed on purpose so that 64-player profiles use the retail squad size; a catalog's `source` shows which rule produced it.

A squad-bound bot population for a world filled to its full size must bind at least `squad_capacity` slots to **every** squad in the catalog, because any squad can hold a human and its bots (`bot_population_insufficient_squad_slots`). With squads of 8 that is 128 slots in Acquisition and 256 in Domination, at both `local64` and `authored`. A population built for 4 per squad does not load against an 8-per-squad catalog. Domination with squads of 8 needs a mag-gameplay release newer than 0.12.1, which accepts Domination squads above 4.

The objective volume extractor reads every level archive and emits each objective trigger's authored oriented box; see [objective trigger volumes](objective-trigger-volumes.md) for the box semantics and what remains unestablished.

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
python3 tools/extraction/apply-objective-trigger-volumes.py --runtime-catalog OBJECTIVE_RUNTIME --catalog OBJECTIVE_METADATA --trigger-volumes OBJECTIVE_VOLUMES --output OBJECTIVE_RUNTIME_WITH_BOXES
```

`STRUCTURE_NATIVE_RULES` is the externally verified `native_rules` object used by the structure loader, including health-condition thresholds, dead/rebuild thresholds and provenance. Its constants cannot be recovered from the selected authored resource tables. The combined skill/structure command requires this object for the structure output; it does not substitute prototype constants for retail.

`TRAINING_WITNESS` supplies `sources.decrypted_game_archive_sha256`, `authored_reward.experience_zpr_sha256`, `authored_reward.experience_bonus`, `authored_reward.scale` and `native_flag.mask`. The projector compares archive/member identities and authored reward/scale values. It does not derive the native completion flag from Lua.

`LEADERSHIP_WITNESS` has schema version 1, application ID 20231, a supported `retail_elf_sha256` and `consumer_constants.refresh_hold_ms`/`removal_grace_ms`. `LOCAL_RECORD` contains only `leadership_level`, `nominations` and `matches_since_last_command`; choose these fallback-policy values deliberately. The role-policy projector reads squad, platoon and Company election weights from the same hashed rules member and emits schema 4. The worker uses Company weights only in Domination. Use `--platoon-only` for schema 3 or `--squad-only` for schema 2 when reproducing or checking older projections.

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

This first stage is **resource-local conversion**, not a collision world. The terrain adapter supports only the original Cosmodrome map. The compiled adapter accepts any retail map through `--map-name`. It requires a terrain proof naming that map and independently established placement evidence. Without `--terrain-token`, it uses the map manifest's single bound type1 terrain resource and refuses a manifest with no terrain resource or more than one. Compiled records whose collision is attached to a parent otherwise require `--reference-pose` and `--parent-proof`, and such output cannot declare a physics region. As an explicit alternative, `--exclude-animated-parented` omits compiled instances whose authored definition class is `AnimatedObjectProperties` and whose collision resource is parent-attached. Their collision moves with the parent, so the adapter treats it as dynamic collision outside the initial static scope. The adapter lists every exclusion with its instance id and name, resource token, resource member and packfile SHA256s, parent tokens and reason. The list goes in the world provenance, the region provenance, `unknowns` and `assembly.json` (`dynamic_exclusions`). A world built this way may declare a physics region, because that region's static claim names what it leaves out. Parented collision on any other class is still refused, and the option cannot be combined with `--reference-pose`. When a map has nothing to exclude, the option leaves the output byte-identical. With it, `--animated-root-packs DIRECTORY` also writes each excluded instance's root-attached packs (those with no parent node) as world-coordinate meshes. A runtime can then place them as standing collision ([sabotage-structures.md](sabotage-structures.md)). The world is unchanged. A second explicit option, `--exclude-unshipped-resources`, covers compiled records whose collision resource the game does not ship. It needs at least one `--shipped-archive` (for example the shared `main.psarc` and the game archive); the level archive is always searched. A record is excluded only when its resource token is unbound in the map manifest and the token's bytes occur in no member of any searched archive other than the level CLV that references it. A token found anywhere else, such as an unconverted resource, stays unresolved and the export still refuses it. Every exclusion is listed with its instance id and name, class, resource token, record SHA256, the searched archive SHA256s and the reason. The list goes in the world and region provenance, `unknowns` and `assembly.json` (`unshipped_exclusions`). Without the option, or when nothing is excluded, the output is unchanged. They read original resource members directly from `LEVEL_PSARC`; there is no hidden dependency on retained raw-member files or an unpublished manifest augmentation:

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

A world without its own verified region can take its coverage from a hash-bound [coverage sidecar](collision-coverage-sidecar.md); see [whole-map runtime navigation](#whole-map-runtime-navigation).

### Runtime navmesh layouts

`mag-navmesh bake` picks the tile layout from the bake bounds. The grid uses 51.2 m tiles.

| Format | Movement profile | Tiles | Polygons per tile | Detour bytes | Bundle version |
|---|---|---|---|---|---|
| `mag_navmesh_world_v2` | 45 degrees | 256 | 16,384 | 32 MiB | 1 |
| `mag_navmesh_world_v3` | projected | 256 | 16,384 | 32 MiB | 3 |
| `mag_navmesh_world_v4` | 45 degrees | 512 | 8,192 | 128 MiB | 5 |
| `mag_navmesh_world_v5` | projected | 512 | 8,192 | 128 MiB | 7 |

When the grid has at most 256 tiles (an 819.2 m square), the bake writes the 256-tile formats, byte for byte as before. A larger grid uses the wide formats. `--layout wide` forces the wide layout for a smaller grid. With 32-bit Detour polygon references, 512 tiles leave room for 8,192 polygons per tile while keeping the 10 salt bits Detour needs. The wide layout bakes each tile from the faces that overlap it, which yields the same tile bytes. It also refuses bounds taller than 409.55 m, the Recast span height limit at 0.05 m cells. Older artifacts (`mag_navmesh_world_v1` to `v3`) load unchanged, and the loader checks each format against its own tile, polygon and byte limits. A serialized artifact stores Detour bytes as a JSON number array, about 3.5 characters per byte. Loaders accept artifact files up to 513 MiB. A loaded mesh keeps only Detour's checked copy of the tiles.

### Optional retail movement slope profile

The supported application 20231 retail build provides an explicit movement slope profile. Project it from your own exact retail ELF and game archive; the tool verifies the ELF, archive and character-settings member hashes before emitting the external JSON:

```sh
python3 tools/navmesh/project-retail-movement-profile.py --elf RETAIL_ELF --game-archive GAME_PSARC --output MOVEMENT_JSON
python3 tools/navmesh/project-retail-movement-profile.py --elf RETAIL_ELF --game-archive GAME_PSARC --output MOVEMENT_JSON --check
```

Pass `--movement-profile MOVEMENT_JSON` explicitly to `mag-navmesh bake`, `query`, `inspect`, or `component`. Reference `bake-reference`, `inspect-reference` and `query-reference` accept the same option. Use the same projected file for baking and loading. Profiled runtime artifacts use `mag_navmesh_world_v3`, or `mag_navmesh_world_v5` for the wide layout; profiled reference bundles use `mag_navmesh_reference_v2`. Their manifests bind the profile hash, and their binary containers have separate versions. Missing or different rules, a legacy artifact paired with the new rules, or a profiled artifact paired with legacy drop certificates are rejected.

For a direct gameplay worker, add `--movement-profile /absolute/MOVEMENT_JSON` alongside `--application-id 20231`, `--collision-world` and `--navmesh-world`. A match-profile registry instead accepts a `movement_profile` path beside that profile's `navmesh_world`; registry paths follow its existing resolution rules. The selected runtime mesh supplies the bot body's slope policy throughout its physical lifecycle. This changes only the ascent/no-slide slope; the replacement runner retains its existing dimensions, step height, gravity and speed.

Omitting the option preserves the existing 45-degree bakes and runtime behavior. The explicit profile does not qualify collision coverage or convert a reference bundle into a gameplay mesh. Runtime static coverage and current-body admission remain separate requirements. Movement profiles, collision worlds and generated navigation data remain external to the source distribution.

### Optional bot body shape

Bots move with an upright box (the body's 0.35 m radius and 1.8 m height) unless a bot body profile selects an upright capsule. The profile is an external JSON document; this repository supplies no dimensions:

```json
{"format": "mag_bot_body_profile_v1",
 "body": {"shape": "capsule", "radius": R, "half_height": H, "centre_height": C}}
```

The capsule is every point within `radius` of a vertical axis that runs `half_height` above and below its centre, `centre_height` above the bot's feet point. `{"shape": "box"}` names the default box. Unknown fields fail. A capsule needs a 0.05–2 m radius, a 0–2 m half-height, its top 0.2–4 m above the feet, and its lowest point within 0.5 m of the feet; it may sit slightly below them.

The shape is part of the body's identity. Drop certificates are traced with it, and a mesh records the shape it is for. A box mesh or certificate records nothing, so box artifacts serialize as before, and an artifact without a shape is the box. A mesh loads only for the runtime body's shape. A box drop certificate under a capsule body, or a capsule certificate under the box, fails with `navmesh_drop_body_shape_mismatch`. A mesh recorded for another shape fails with `navmesh_body_shape_mismatch`, even with no drops.

Pass `--body-profile BODY_JSON` to `mag-navmesh bake` to record the shape in a new mesh. `add-drop`, `search-spawn-exits` and `replay-drop` take the same option and trace with that body. `search-spawn-exits` refuses seed drops certified with another shape. `goals`, `query`, `inspect` and `component` take it to load such a mesh. `mag-navmesh rebind-body --mesh MESH_JSON --body-profile BODY_JSON --output NEW_MESH_JSON` re-targets an existing mesh at another shape. It removes the mesh's drop certificates, which were traced with the old body; certify them again with `add-drop` and `search-spawn-exits`. Omit `--body-profile` to rebind to the box.

For a direct gameplay worker, add `--bot-body-profile /absolute/BODY_JSON` alongside `--navmesh-world`. A match-profile registry instead accepts a `bot_body_profile` path beside that profile's `navmesh_world`. The shape applies to the bots' locomotion and local traversal checks, and to the bodies in their dynamic collision: humans and bots there take the runtime shape, and a capsule meets another capsule exactly. Without a profile, everything stays the box. The capsule's grounding uses its contact normal: on a face, the face normal; on an edge or corner, the direction from it to the capsule's axis. A lip or crest then holds the body only as it would hold a rounded body. The capsule's step lift stops at the first contact overhead, so the body can step while passing under a beam it fits beneath. A capsule placed with its feet on authored ground (a spawn, a drop launch, a resupply endpoint) overlaps that ground when its rounded bottom reaches below the feet or the ground slopes. It starts settled instead: lifted by at most the depth its rounded bottom reaches into ground at the slope limit (never more than a step), which must be free, then swept back down onto the ground, so its feet rest slightly above the authored point. Any other initial overlap is still refused, and the box starts as before. The capsule body does not claim native character-controller parity.

Capsule bots also pass each other as rounded bodies. A moving capsule bot reserves the capsule at each pose along the path it just walked (its current pose last), so another capsule meets it axis to axis and slides round it along the contact's tangent, rather than stopping against the box around that path. A capsule bot that already overlaps another bot's capsule (two bots spawned closer than their summed radius) may make any move that does not bring the two axes closer, including settling onto the ground and stepping. A capsule bot overlapping any capsule body is moved out of the overlap along the axis-to-axis direction at most 1.5 m/s per update, sliding along static geometry and other bodies, never into them; a stationary capsule is still never walked through. On a route, capsule traffic gives way by object index: of two bots, the lower index keeps its course (for up to two seconds of being held up), and the higher one turns 60 degrees, steps square to the side, or backs off at an angle or straight back, preferring the same hand each time, and after backing off keeps looking a second ahead for the bot it made way for. A head-on pair therefore passes instead of mirroring each other, and in a passage too narrow to pass in the higher index retreats until the other is through. Touching a body during a step that then ends against a wall counts as waiting on traffic, not as a blocked route. Humans keep their rule: they are never moved, and a bot overlapping one may only make moves that take it away from the human. None of this applies to the box, whose traffic is unchanged. Checks that a spot or path is clear for a bot (spawn, role, drop-holding and traffic-yield spots, approach lanes, the end of a route, a drop's trajectory) ask with the capsule as well, so it meets standing capsule bots round rather than through the box around them. A capsule's goal (a passage launch, a role spot) may rest with its feet up to its settle lift above the navmesh it projects onto, as a settled capsule does, before the goal counts as too far from the mesh. A capsule bot that finds no holding spot around a passage launch and is frozen inside another bot's approach lane (or a protected passage) steps to the nearest supported spot that keeps the lane spacing plus its own radius from every lane, instead of waiting in the lane; the box still waits where it stands.

### Whole-map runtime navigation

Bots can use a whole map when the operator asserts its coverage in a [coverage sidecar](collision-coverage-sidecar.md). The reviewed world file stays unchanged. The flow is: prepare the coverage sidecar, prepare the bake input with it, bake, then certify drops and spawn exits against the same world and sidecar. For each map:

```sh
world=WORLD_JSON
world_sha256=$(python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$world")
python3 tools/navmesh/make-coverage-sidecar.py --world "$world" --expected-world-sha256 "$world_sha256" \
  --whole-world --operator-assertion "REVIEW STATEMENT" --output COVERAGE_JSON
python3 tools/navmesh/prepare.py --world "$world" --expected-world-sha256 "$world_sha256" \
  --collision-coverage COVERAGE_JSON --output PREP_DIRECTORY
cargo build --release --locked --manifest-path gameplay/Cargo.toml --bin mag-navmesh
gameplay/target/release/mag-navmesh bake --input PREP_DIRECTORY/input.json --output MESH_JSON
gameplay/target/release/mag-navmesh inspect --mesh MESH_JSON
gameplay/target/release/mag-navmesh add-drop --mesh MESH_JSON --collision-world "$world" \
  --collision-coverage COVERAGE_JSON --launch X,Y,Z --landing X,Y,Z --speed SPEED --id ID --output MESH_WITH_DROP_JSON
gameplay/target/release/mag-navmesh search-spawn-exits --mesh MESH_WITH_DROP_JSON --collision-world "$world" \
  --collision-coverage COVERAGE_JSON --parameters SEARCH_JSON --fall-damage-rules FALL_JSON --health 400 \
  --output EXITS_DIRECTORY
```

A runtime route query searches at most 16,384 Detour nodes. On a whole-map mesh, a long ground route inside one walking component can exhaust that pool. The runtime then plans the route in stages. At load it groups each tile's polygons into clusters that connect without leaving the tile, and joins neighbouring clusters through one representative crossing per ordered pair. A staged query first orders the clusters from start to goal. It then searches the polygon corridor in pieces of about 96 m along that order, keeps the first half of each piece so the next piece starts inside a locally shortest path, and string-pulls the joined corridor. A direct search whose path outgrows its 1,024 polygons before reaching the goal is staged the same way. One query, including the direct attempt, uses at most four node pools. A staged corridor and its straight path may hold up to 4,096 polygons, which long routes on finely divided meshes (such as the 54-degree movement-profile meshes) need. A route that still cannot complete keeps the direct partial result and reports `out_of_nodes` or `partial`. Routes that fit the pool and 1,024 polygons are unchanged. Staged routes report their piece count as `stages`.

`prepare.py` reports `runtime_grid_tiles` and `runtime_layout` in its manifest and refuses bounds over 512 tiles. The bake prints the chosen format. Bake bounds default to the first verified region: the world's own region, else the sidecar's. `search-spawn-exits` extends the drop links already in the mesh, so add each seed drop first. A drop certificate records the body it was traced with. `add-drop`, `search-spawn-exits` and `replay-drop` take `--movement-profile` and trace with the profiled mesh's body (the retail slope limit), so a profiled mesh (`v3`/`v5`) carries drops certified with its own profile. A mesh refuses a certificate traced with any other body: a profiled mesh refuses default-body certificates, and a 45-degree mesh (`v2`/`v4`) refuses profiled ones. Default-body certificates serialize as before. At runtime, pass the same sidecar with `--collision-coverage` or a match profile's `collision_coverage`.

#### Character movement layers

Every static triangle carries a retail collision layer (the low five bits of its `filter_info`). Retail decides which layers collide through 32 filter rows, one per layer, where bit *n* of row *m* means layers *m* and *n* collide. The worlds hold four static layers. Layer 9 is the ordinary static world. Layer 11's row collides only with layers 0, 17, 18 and 27: ground vehicles (27) and vehicle hulks (17), never a character. Its triangles are invisible sheets that keep vehicles out of footpaths, yards and doorways. Layer 29's row collides with layers 1, 16, 17, 18 and 27, so it also stops characters. Layer 31's row collides only with layers 7 and 31.

A player walks through a layer-11 sheet. Treating it as a wall sealed whole spawn areas: on VOZ, the squad 16–19 hangars connect to the rest of the map only through such a sheet. The character controller's own layer isn't recorded in the projected data. The candidates are the rows that collide with layer 9 and not with layer 11: rows 1, 3, 4, 5, 7, 13, 14, 16, 19, 23, 24, 28 and 30. They all keep layer 9 and leave out layer 11, and differ only on layers 29 and 31. Use row 1. It keeps layer 29, which, like a player-and-vehicle blocker, collides with vehicles and with candidate rows 1 and 16. It leaves out layer 31, a handful of triangles that collide with nothing on a candidate row except row 7.

Pass the rows and the character's layer to `prepare.py`:

```sh
python3 tools/navmesh/prepare.py --world "$world" --expected-world-sha256 "$world_sha256" \
  --collision-coverage COVERAGE_JSON --filter-rows GAMEPLAY_CONFIG_JSON --character-layer 1 \
  --output BAKE_DIRECTORY
```

`--filter-rows` takes a JSON file holding the 32 rows, either as a list or under a `filter_rows` key (an Acquisition gameplay configuration has one). The preparer leaves out every instance on a layer the character's row passes through and keeps instances without filter metadata. It records the kept layers as `character_movement_layers` in its input and manifest, with the rows file's hash and the excluded counts per layer. `mag-navmesh bake` copies `character_movement_layers` into the artifact. When a navmesh with that field is paired with its collision world (in the engine, in a match profile, or in `add-drop`, `search-spawn-exits` and `replay-drop`), bot movement and drop certification collide only with those layers. Rays, projectiles, blasts, seats and every other query still see every triangle. Without the options, the input, mesh and bot movement are unchanged.

The same rows decide what static rays see. Bullets and other projectile flights query as layer 23, and blast exposure queries as layer 24. Both rows collide with layer 9 and with none of layers 11, 29 or 31. So a player's shots and blasts pass the invisible sheets that stop only vehicles. With `collision_filter_rows` in a match profile, or `--collision-filter-rows` for the direct worker, the static-world rays for the following use those rows:

- bot sight and bot shots (row 23);
- Sabotage rocket flights (row 23);
- blast exposure: development blast visibility, Sabotage frags and rockets, and Acquisition destruction explosions (row 24);
- the Acquisition arrival check, which already had its row 24.

Acquisition destruction uses its configuration's own rows. Triangles without filter metadata still block. Without the rows, every triangle blocks.

### Offline whole-map reference navigation

Use an explicit reference bake to inspect routes through the emitted static geometry of a provisional world. The preparer preserves the original world hash, winding, and region metadata. It derives its bake envelope from the full f32 vertex inventory, with vertical padding for standing clearance.

```sh
python3 tools/navmesh/prepare.py --reference --world WORLD_JSON --expected-world-sha256 WORLD_SHA256 --output PREP_DIRECTORY
cargo build --release --locked --manifest-path gameplay/Cargo.toml --bin mag-navmesh
gameplay/target/release/mag-navmesh bake-reference --input PREP_DIRECTORY/input.json --output REFERENCE_DIRECTORY
gameplay/target/release/mag-navmesh inspect-reference --mesh REFERENCE_DIRECTORY
gameplay/target/release/mag-navmesh query-reference --mesh REFERENCE_DIRECTORY --start X,Y,Z --goal X,Y,Z --goal-radius 1.5
```

The output is a new directory containing `manifest.json` and `detour.bin`. Inspection verifies the binary hash and tile layout and reports the occupied tile count and largest tile polygon count. The reference grid supports up to 512 tiles at 51.2 m per side, 8,192 polygons per tile, and 192 MiB of binary tiles. Queries report the reference route format and retain disconnected and partial results. Keep the source world and its preparation receipt with the bundle so its scope can be verified independently.

The reference bundle is an offline route-planning artifact. It describes the baked static geometry and its represented transforms. Gameplay movement and certified drops require the separate runtime mesh and a verified physics region, from the world or a coverage sidecar. Dynamic obstacles, moving bodies, and objective sequences require their own current-state checks.

A reference query reports a disconnected goal when the full goal search finds no candidate in the start component and no loaded edge can cross the component labels. For a selected goal in the same component, the existing 16,384-node search can try the same endpoints in reverse once if the initial search runs out of nodes. The retry must reach both endpoints and produce a valid forward corridor. A query can still return a partial corridor or exhaust its search budget.

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
| `retail-vehicle-collision.json` | direct | `extract-retail-vehicle-collision.py` ([vehicle impacts](vehicle-impact-damage.md)) |
| `retail-incapacitation.json` | direct | `extract-retail-incapacitation-rules.py` ([incapacitation](incapacitation.md)) |
| `retail-item-bindings.json` | external-proof | `compare-native-item-catalog.py` |
| `retail-itemlist-native-order.json` | gap | Owner-qualified native token-order snapshots, binding attestation and starter-record evidence |
| `retail-items.json` | direct | `extract-item-catalog.py` |
| `retail-leadership-passives.json` | external-proof | `project-retail-leadership-passives.py` |
| `retail-leadership-role-policy-native.json` | direct-with-policy | `project-retail-leadership-role-policy.py` |
| `retail-leadership-role-policy.json` | operator-configuration | Operator-selected development leadership assignments |
| `retail-maps.json` | direct | `extract-retail-maps.py` |
| `retail-domination-map-metadata.json` | direct | [Domination metadata extractor](domination-map-metadata.md) |
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

Domination profile and controller extraction, including the external initial-load
qualification inputs, is described in [Domination runtime](domination-runtime.md).

## Regional walking coverage

A direct retail Acquisition worker can opt a fixed bot population into one
bounded region of the represented initial fixed triangles. The operator
supplies and approves external source qualification. The projector compares
complete ordered geometry and fixed-root membership, verifies archive and CLV
identity, and rechecks the retained original ELF spans for source-qualified
omissions. The supplied proofs establish the interpretation of those spans and
the resource census. The runtime checks the resulting identities and bounds.

```sh
python3 tools/navmesh/project-regional-walk-coverage.py \
  --world "$world" --assembled-world "$assembled_world" --assembly "$assembly" \
  --root-inventory "$root_inventory" --absence "$absence_receipt" \
  --level-archive "$level_archive" --main-archive "$main_archive" --retail-elf "$elf" \
  --min "$min_x" "$min_y" "$min_z" --max "$max_x" "$max_y" "$max_z" \
  --output "$walk_coverage"
python3 tools/navmesh/prepare.py --world "$world" \
  --expected-world-sha256 "$world_sha256" --regional-walk-coverage "$walk_coverage" \
  --output "$prep_dir"
mag-navmesh bake-regional-walk --input "$prep_dir/input.json" \
  --coverage "$walk_coverage" --output "$walk_mesh"
mag-navmesh query-regional-walk --mesh "$walk_mesh" --collision-world "$world" \
  --coverage "$walk_coverage" --start "$start_xyz" --goal "$goal_xyz" --goal-radius 2
```

Query endpoints are comma-separated X,Y,Z points. The regional bake uses the
existing 45-degree profile. Set `walking_coverage_sha256` to the raw sidecar
digest and `walking_detour_sha256` to the artifact's `mesh.detour_sha256` in the
population's `regional_collision` object. Keep the exact sidecar bounds and
existing world, spawn, Ground, dynamic-catalog and Structural identities there.

Alongside the normal direct Acquisition inputs, use `--application-id 20231`,
`--navmesh-world "$walk_mesh"`, `--regional-walk-coverage "$walk_coverage"`, and
the fixed `--bot-population` and `--bot-fill-total` options. Supply the coverage
path as an absolute path. This scope requires a fixed regional population and
rejects autonomous movement and drop links. The runtime checks current
obstructions and the full body sweep separately from the baked route. Catalog
validation checks artifact and config bindings. A connected scheduler replay
tests allocation, walking, and live obstruction.

The original collision world and its metadata are unchanged. Static material
and projectile queries, certified drops, ordinary verified navmeshes, and
offline reference navmeshes retain their existing coverage contracts.
