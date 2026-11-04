# Objective trigger volumes

`tools/extraction/extract-retail-objective-volumes.py` reads every retail level
archive and emits, for each objective, the authored trigger volumes its
`ObjectiveTriggerProperties` records link to. Its output is game-derived data:
write it outside the source checkout and never commit it. Public tests use only
invented values.

```sh
python3 tools/extraction/extract-retail-objective-volumes.py --disc DISC [--game-archive GAME] [--map LEVEL]... --output OBJECTIVE_VOLUMES
```

`--game-archive` only adds the configured map name, mode and owning faction.
`--map` restricts extraction to named levels. `--check` regenerates in memory
and requires the existing output to be byte-identical.

## Box semantics

Every objective trigger volume in the supported retail build is a
`TriggerVolumeBoxProperties` instance. The extractor fails on any other class
rather than approximating it. The original engine's construction and
containment test, verified for the supported retail executable, are:

- `Translate` is the box centre.
- `RotateX/Y/Z` are degrees, composed as `Rx * Ry * Rz` for row vectors. The
  rows of that matrix are the box axes, exactly as
  `retail_map_helpers.euler_basis` returns them.
- `ScaleX/Y/Z` are the full edge lengths along those axes. The uniform `Scale`
  field is not used. Half-extents are half of each scale.
- A point `p` is enclosed with padding `r` when, for every axis `e_i`,
  `|(p - Translate) · e_i| <= Scale_i / 2 + r`.

## Output

```json
{
  "schema_version": 1,
  "format": "mag_retail_objective_trigger_volumes",
  "application_id": 20231,
  "volume_semantics": "oriented_box_half_scale",
  "maps": [{
    "map": "LEVEL",
    "source": {"archive": "...", "archive_sha256": "...", "clv_member": "...", "clv_sha256": "..."},
    "objectives": [{
      "objective_token": "0x...",
      "objective_class": "ObjectiveSecureAndHoldProperties",
      "triggers": [{
        "trigger_token": "0x...", "volume_token": "0x...",
        "trigger_position": [0, 0, 0],
        "box": {"centre": [0, 0, 0], "rotate_degrees": [0, 0, 0], "scale": [1, 1, 1],
                "half_extents": [0.5, 0.5, 0.5], "axes": [[1, 0, 0], [0, 1, 0], [0, 0, 1]]}
      }]
    }]
  }]
}
```

Triggers are listed in level-file order. Arrival, secure-and-hold and
structural-demolition objectives are all included; the runtime decides which
ones it uses.

## Runtime use

`project-retail-objective-runtime.py --include-interactions --trigger-volumes OBJECTIVE_VOLUMES`
attaches each interaction or demolition rule's boxes as `trigger_volumes` and
sets `distance_policy: "authored_trigger_box"`. The linked trigger tokens in the
authored objective metadata must match the volume catalog exactly, with one box
per `trigger_count`. For a catalog that was already projected, apply the same
step without re-running the projection:

```sh
python3 tools/extraction/apply-objective-trigger-volumes.py --runtime-catalog OBJECTIVE_RUNTIME --catalog OBJECTIVE_METADATA --trigger-volumes OBJECTIVE_VOLUMES --output OBJECTIVE_RUNTIME_WITH_BOXES
```

`OBJECTIVE_METADATA` must be the exact file recorded in the runtime catalog's
`source.authored_catalog_sha256`.

Under `authored_trigger_box` the gameplay server accepts an activation, defuse
or continued hold when the actor's body column, from the tracked feet position
up to the default body height, overlaps any of the objective's boxes grown by
the default body radius. That padding is server policy standing in for the
client's physics overlap. Bots choose standing points inside a box, grounded
within the box's vertical span, and only act once the same check passes.
`maximum_distance_m` is ignored under this policy. Catalogs that keep
`development_point_distance` behave exactly as before.

## Not established

- The character reference point and padding the original client uses before it
  offers the interaction prompt. The client registers characters through a
  physics overlap with the trigger, not through this point test.
- The client's additional approach-angle gate for interactive objects.
- The mapping from native trigger index to the linked volumes.
- Runtime pool indices and live interaction acceptance.
