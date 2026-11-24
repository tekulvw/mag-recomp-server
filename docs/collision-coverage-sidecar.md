# Collision coverage sidecar

A collision world's `metadata.physics_regions` lists the boxes where its ground
and static obstacles are verified. Bot navigation refuses everything outside
them: the runtime navmesh bake, navmesh loading, bot spawning and movement, and
bot line-of-sight checks. Most reviewed worlds have no regions, and adding regions
to a world file changes its SHA-256. That invalidates every receipt bound to the
world, such as ability static-ray receipts.

A coverage sidecar declares the regions in a separate file instead. It names one
world by SHA-256 and map, so the world bytes and their hash stay unchanged.

## Format

`mag_collision_coverage_v1` is a JSON object of at most 64 KiB. Unknown fields
are refused.

```json
{
  "format": "mag_collision_coverage_v1",
  "map_name": "synthetic_map",
  "world_sha256": "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0",
  "basis": "operator_assertion",
  "provenance": [
    "operator assertion: whole-map static collision reviewed for bot navigation",
    "written by tools/navmesh/make-coverage-sidecar.py"
  ],
  "physics_regions": [
    {
      "min": [-512.0, -40.0, -480.0],
      "max": [548.0, 120.0, 590.0],
      "ground_verified": true,
      "obstacles_verified": true,
      "provenance": ["operator assertion: whole-map static collision reviewed for bot navigation"]
    }
  ]
}
```

| Field | Rule |
|---|---|
| `format` | Exactly `mag_collision_coverage_v1` |
| `map_name` | The world's `metadata.map_name`, 1–128 bytes |
| `world_sha256` | Lowercase SHA-256 of the exact world file bytes |
| `basis` | `operator_assertion` or `source_verified`; both authorize navigation, and startup logs the value |
| `provenance` | Nonempty list of nonblank strings |
| `physics_regions` | 1–64 regions with the same fields and meaning as a world's `physics_regions`: finite `min` < `max` on every axis, each coordinate within ±100,000 m, and a nonempty `provenance` |

## Semantics

- The loader refuses a sidecar whose `world_sha256` or `map_name` differs from
  the loaded world (`collision_coverage_world_mismatch`). A world takes at most one sidecar.
- Bot navigation uses the world's own regions and the sidecar's regions as one
  list. A box is covered when a single region with both `ground_verified` and
  `obstacles_verified` contains it. Regions are not merged, so a box that spans two
  adjacent regions is not covered.
- Static world casts for projectiles and active abilities, and static material
  queries, use only the world's own metadata. A sidecar does not change them.
- A fixed Acquisition population with a regional walking mesh keeps its own
  coverage scope (see [bot regional collision](bot-regional-collision.md)).
  Without one, the population's region check uses the combined list.

## Writing a sidecar

`tools/navmesh/make-coverage-sidecar.py` writes a sidecar for a world after it
checks the world's hash. It requires `--operator-assertion TEXT`, which it
records in each region's provenance. Writing a sidecar is a claim by the operator
that the world represents the ground and static obstacles inside each region.
The tool does not check the claim.

```sh
python3 tools/navmesh/make-coverage-sidecar.py --world WORLD_JSON \
  --expected-world-sha256 WORLD_SHA256 --whole-world \
  --operator-assertion "whole-map static collision reviewed for bot navigation" \
  --output COVERAGE_JSON
```

`--whole-world` writes one region spanning every world vertex, rounded to f32.
`--margin-y` pads it vertically (default 3 m, the same standing clearance as a
reference bake) and `--margin-xz` pads it horizontally (default 0). The default
padding gives the same bounds as `prepare.py --reference`, so a world that fits
the 512-tile reference grid also fits the runtime grid. Repeat
`--region MIN_X MIN_Y MIN_Z MAX_X MAX_Y MAX_Z` to add explicit boxes.

## Using a sidecar

| Consumer | Option |
|---|---|
| `tools/navmesh/prepare.py` | `--collision-coverage COVERAGE_JSON` appends the sidecar's regions to the bake input and records the sidecar's hash in the preparation manifest |
| `mag-navmesh add-drop`, `goals`, `search-spawn-exits`, `replay-drop` | `--collision-coverage COVERAGE_JSON` beside `--collision-world` |
| `cargo run --example sample_bot_population` | Optional ninth argument, after `FALL_RULES` |
| Direct gameplay worker | `--collision-coverage COVERAGE_JSON` beside `--collision-world` |
| Match-profile registry | `collision_coverage` path beside the row's `collision_world` |

The runtime checks coverage against the loaded world and sidecar, not against
the regions stored in a navmesh artifact. A navmesh baked with a sidecar must
therefore be loaded with the same sidecar.
