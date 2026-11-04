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

## Not established

- The character reference point and padding the original client uses before it
  offers the interaction prompt. The client registers characters through a
  physics overlap with the trigger, not through this point test.
- The client's additional approach-angle gate for interactive objects.
- The mapping from native trigger index to the linked volumes.
- Runtime pool indices and live interaction acceptance.
