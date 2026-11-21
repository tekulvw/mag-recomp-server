#!/usr/bin/env python3
"""Project a bot-only provider catalog from the original Sabotage Cosmodrome level."""
import argparse
import json
import math
from pathlib import Path
import re
import runpy

TOOLS = Path(__file__).resolve().parent / "extraction"
SHARED = runpy.run_path(str(TOOLS / "extract-retail-objective-volumes.py"))
HELPERS = runpy.run_path(str(TOOLS / "retail_map_helpers.py"))
MAP = "mag_cosmodrome_final"
MEMBER = f"/run/mag/ps3/levels/{MAP}/{MAP}.clv"
LEVEL_SHA = "8157f5abc55ddd2ca9eaad3379e87ae909d0b8f7c9b08fa04f1f1e2d8018d96e"
CLV_SHA = "d336caea88351dda5f3204a34e64fdde8eabdf0532b3133e2338cdd150488f4b"
CLASS = "SupplyPointObjectProperties"
QUALIFIED_START_TOLERANCE_M = 1.0
RECEIPT_SHA = "f24127b06399d1353e4bb1c1a73943278227cd31a1488eda409096099b089b0d"
require, sha = SHARED["require"], SHARED["sha"]


def values(fields):
    result = {field["name"]: field["value"] for field in fields}
    require(len(result) == len(fields), "duplicate authored fields")
    return result


def provider(row, definitions):
    definition = definitions.get(row["preceding_token"])
    if definition is None:
        return None
    fields = values(row["fields"])
    visible, frozen, snapped = (fields[key] for key in
                                ("IsVisible_M", "IsFrozen_M", "SnapToTerrain_M"))
    require(all(type(value) is bool for value in (visible, frozen, snapped)), "invalid placement flags")
    if not visible or frozen or snapped:
        return None
    box = SHARED["box"](fields, HELPERS["euler_basis"])
    name = fields["Name"]
    require(isinstance(name, str) and re.fullmatch(r"0x[0-9a-f]{8}", name), "invalid instance name token")
    return {
        "source_offset": int(row["offset"], 16), "name_token": int(name, 16),
        "instance_sha256": row["source_sha256"],
        "definition_offset": definition["offset"], "definition_token": row["preceding_token"],
        "definition_sha256": definition["source_sha256"],
        "center": box["centre"], "euler_xyz_degrees": box["rotate_degrees"],
        "scales_xyz": box["scale"], "axes": box["axes"],
        "tick_rate_seconds": definition["tick_rate_seconds"],
        "tolerance_m": QUALIFIED_START_TOLERANCE_M,
    }


def validate_bindings(spawn_raw, collision_raw):
    spawns = json.loads(spawn_raw)
    mode = spawns["match_profile"]["game_type"]
    require(spawns["active"]["map_name"] == MAP and type(mode) is int and mode == 0,
            "spawn catalog must select Cosmodrome Sabotage")
    collision = json.loads(collision_raw)
    require(collision["format"] == "mag_collision_world"
            and collision["coordinate_space"] == "world"
            and collision["metadata"]["map_name"] == MAP,
            "collision world must use Cosmodrome world coordinates")


def project(archive_raw, spawn_raw, collision_raw):
    validate_bindings(spawn_raw, collision_raw)
    require(sha(archive_raw) == LEVEL_SHA, "wrong original Cosmodrome level archive")
    extractor = runpy.run_path(str(TOOLS / "build-retail-spawn-catalog.py"))["extractor"]()
    archive = extractor.Psarc(archive_raw)
    raw = archive.extract(archive.names.index(MEMBER) + 1)
    require(sha(raw) == CLV_SHA, "wrong original Cosmodrome CLV")
    clv = extractor.retail_clv(raw)
    definitions = {}
    for match in SHARED["CLASS_HEADER"].finditer(raw):
        header = match.start()
        if match[1].decode() != CLASS or header < 16 or clv.u32(header - 16) != 7:
            continue
        token, end = clv.u32(header - 4), clv.u32(header - 8)
        require(token != 0 and token not in definitions and header < end <= len(raw),
                "ambiguous or invalid supply definition")
        fields, stop = clv.fields(match.end(), end)
        authored = values(fields)
        tick = authored["TickRate"]
        require(stop + 1 == end and authored["UsesNonUniformScale"] is True
                and type(tick) is float and math.isfinite(tick) and tick > 0,
                "invalid supply definition")
        definitions[token] = {"offset": header - 16, "source_sha256": sha(raw[header - 16:end]),
                              "tick_rate_seconds": tick}
    providers = [row for instance in clv.tables("InstanceProperties")
                 if (row := provider(instance, definitions)) is not None]
    require(len(providers) == 3, "Cosmodrome must have three qualified replacement providers")
    require(len({row["name_token"] for row in providers}) == len(providers),
            "ambiguous supply point name token")
    return {
        "schema_version": 1, "application_id": 20231, "map": MAP, "mode": 0,
        "spawn_sha256": sha(spawn_raw), "collision_sha256": sha(collision_raw),
        "source": {"archive_sha256": LEVEL_SHA, "clv_sha256": CLV_SHA,
                   "qualification_sha256": RECEIPT_SHA},
        "providers": providers,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("--level-archive", "--spawn-catalog", "--collision-world", "--out"):
        parser.add_argument(name, required=True, type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        files = [args.level_archive, args.spawn_catalog, args.collision_world, args.out]
        if args.receipt is not None:
            files.append(args.receipt)
        require(len(set(path.resolve() for path in files)) == len(files), "input and outputs must be distinct")
        rules = project(args.level_archive.read_bytes(), args.spawn_catalog.read_bytes(),
                        args.collision_world.read_bytes())
        encode = lambda obj: (json.dumps(obj, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
        result = encode(rules)
        receipt = encode({"schema_version": 1, "catalog_sha256": sha(result),
                          "selection": "visible_unfrozen_unsnapped", **rules["source"]})
        for path, content in ((args.out, result), (args.receipt, receipt)):
            if path is None:
                continue
            if args.check:
                require(path.read_bytes() == content, f"projected bytes differ from {path}")
            else:
                path.write_bytes(content)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"error: {error}\n")


if __name__ == "__main__":
    main()
