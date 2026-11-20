# Project active ability map bindings

Generate one binding file per map with the original retail level PSARC and the
exact Objective catalog the worker will load. Use the qualified identity or pose
receipt for that catalog. The projector reads the inputs offline and writes the
binding consumed by the `map_bindings` entry in the active-abilities manifest.
Keep original assets and generated bindings in the operator's private data
directory. [Worker match profiles](worker-match-profiles.md)

The projector generates binding schema 3 with `damage_triggers` and
`precision_targets` arrays. Rerun it before enabling the platoon effect setup.
A precision target binds a nonzero-category Structural objective to its exact
type-12 network index and authored team. The worker checks that the live
objective belongs to the other team at command admission.
Each entry binds a source-projected damage trigger box to a Structural
objective and one of its demolition trigger slots. An empty array records that
the projection found no qualifying triggers. The ability setup manifest
continues to select the binding file through `map_bindings`.

## Acquisition

Use the initial type-12 Objective identity receipt consumed by
`project-retail-acquisition-initial-objectives.py`. It must match the worker
catalog's `source.binding_capture_sha256` and contain the requested map. The
projector reads linked Objective positions from the original compiled level. It
can also emit the root receipt whose digest appears in the binding.

```sh
python3 tools/extraction/project-retail-active-ability-map.py \
  --level-archive "$RETAIL_LEVEL_PSARC" \
  --gameplay-objectives "$WORKER_OBJECTIVES_JSON" \
  --acquisition-initial-bindings "$INITIAL_OBJECTIVE_BINDINGS_JSON" \
  --acquisition-roots-output "$ACQUISITION_ROOTS_JSON" \
  --output "$ABILITY_MAP_BINDINGS_JSON"
```

## Domination

Use the qualified per-map Objective poses consumed by
`project-retail-domination-gameplay-objectives.py`. The projector compares
the original linked Objective positions with the qualified poses and checks
every token-to-index mapping against the worker catalog.

```sh
python3 tools/extraction/project-retail-active-ability-map.py \
  --level-archive "$RETAIL_LEVEL_PSARC" \
  --gameplay-objectives "$WORKER_OBJECTIVES_JSON" \
  --domination-qualified-poses "$QUALIFIED_OBJECTIVE_POSES_JSON" \
  --output "$ABILITY_MAP_BINDINGS_JSON"
```

The projector checks the archive and compiled level digests from the selected
receipt. It reads the authored platoon Objective links and subtype-13 anti-air
assets and derives inclusive X/Z limits from the selected terrain resource.
Separate Y bounds come from the compiled level header and support the vertical
static ray. These projections cover retail levels with zero explicit boundary
points and the verified identity-transform terrain fallback. The worker
requires the corresponding static-ray receipt as a separate manifest input.
