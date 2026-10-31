# Public synthetic inputs

Every JSON file in this directory was independently authored from the Rust schema
and validation code. None was copied from extracted game settings, runtime
captures, or a private catalog. These inputs demonstrate schemas and allow local
unit tests to compile. They do not establish compatibility with a game client.

- `synthetic-wire-profiles.json` supplies the identifiers and bit widths the
  public validator already requires. Its evidence fields explicitly identify it
  as synthetic. Production startup still accepts `--wire-profiles PATH`.
- `synthetic-bot-objective-policy.json` is an invented bounded policy. Objective
  interactions default to disabled. An operator can provide a complete
  `objective_policy` in a bot population JSON file. The strict schema rejects
  unknown fields, so this adjacent provenance statement identifies the input.
- `synthetic-movement-metadata.json` is a metadata placeholder. The public movement
  implementation continues to use its existing constants, not values parsed
  from this file.

Bot names are generated in code as `Synthetic Bot AA` through independently
assigned letter pairs. No original developer-credit names are embedded.

Game item, loadout, spawn, structure, skill, combat, leadership, falling-damage,
collision and navigation catalogs are operator-provided resources. The public
package deliberately supplies no development copies of those catalogs.
`Engine::try_new` reports their absence; `Engine::new` remains the existing
convenience wrapper that panics on missing development resources. Construct
`EngineResources` explicitly when integrating the library. Startup does not
create empty substitute worlds when required resources are absent.

The Dockerfile builds only source plus the three synthetic inputs listed above.
It includes no game catalogs in the final runtime image. Supply runtime resource
paths through command-line options and mounts when operating a private service.

Bunker seat transitions additionally require an operator-authored
`--bunker-exit-policy PATH` file, or
`EngineResources::with_bunker_exit_policy(policy)`. There is no default policy.
Its strict JSON schema has `transform` (`position` and unit `rotation_xyzw`) and
one to 32 `candidates`, each with `ranking_offset` and `position_offset` vectors.
Offsets used to rank camera directions and offsets for settled positions are
explicitly separate. The operator must validate terrain and clearance; this
transform helper does not provide those checks. Without this policy the engine
denies seat mount, dismount, and bunker passenger respawn with
`bunker_exit_policy_required`, while ordinary on-foot spawns still use the
loaded spawn catalog's point.

Legacy API changes for the source split: `CharacterRespawn::at_position` replaces
map-specific respawn constructors. `DefensiveEmplacementCreation` requires
explicit fields; its map-specific constructor and `AlphaSpawnSetup` wrappers
that synthesized an emplacement were removed. Use `AlphaSpawnSetup` with
explicit bunker creations. No original map coordinates, model exit offsets or
bunker quaternions are embedded in those helpers.
