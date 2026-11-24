#!/usr/bin/env python3
"""Project a bot-only provider catalog from one original retail Sabotage level."""
import argparse
import json
import math
from pathlib import Path
import re
import runpy

TOOLS = Path(__file__).resolve().parent / "extraction"
SHARED = runpy.run_path(str(TOOLS / "extract-retail-objective-volumes.py"))
HELPERS = runpy.run_path(str(TOOLS / "retail_map_helpers.py"))
SABOTAGE_MAPS = runpy.run_path(str(TOOLS / "project-retail-sabotage-objectives.py"))["SABOTAGE_MAPS"]
DEFAULT_MAP = "mag_cosmodrome_final"
# Per-map qualification: the expected visible, unfrozen, unsnapped provider
# count and the SHA-256 of the private witness that qualified that selection.
# Level archive and CLV identities come from the pinned Sabotage map table.
QUALIFIED = {
    "mag_cosmodrome_final": dict(
        providers=3,
        qualification_sha256="f24127b06399d1353e4bb1c1a73943278227cd31a1488eda409096099b089b0d"),
    "mag_kennicott_final": dict(
        providers=2,
        qualification_sha256="39f2839c01b2251b84acac85b11cbcc0e66fae460afde11017abc03ff3840d06"),
    "mag_remote_comms_final": dict(
        providers=3,
        qualification_sha256="0198100a8c9bae0ae20b9df17a3cfa4b0ee4b3f689a00828abd47a1f744c3a1c"),
}
CLASS = "SupplyPointObjectProperties"
QUALIFIED_START_TOLERANCE_M = 1.0
require, sha = SHARED["require"], SHARED["sha"]


def map_pins(map_name):
    require(map_name in QUALIFIED and map_name in SABOTAGE_MAPS,
            f"unsupported retail Sabotage map: {map_name}")
    level = SABOTAGE_MAPS[map_name]
    return dict(QUALIFIED[map_name], level_sha256=level["level_sha256"],
                clv_sha256=level["clv_sha256"],
                member=f"/run/mag/ps3/levels/{map_name}/{map_name}.clv")


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


def validate_bindings(spawn_raw, collision_raw, map_name=DEFAULT_MAP):
    map_pins(map_name)
    spawns = json.loads(spawn_raw)
    mode = spawns["match_profile"]["game_type"]
    require(spawns["active"]["map_name"] == map_name and type(mode) is int and mode == 0,
            f"spawn catalog must select {map_name} Sabotage")
    collision = json.loads(collision_raw)
    require(collision["format"] == "mag_collision_world"
            and collision["coordinate_space"] == "world"
            and collision["metadata"]["map_name"] == map_name,
            f"collision world must use {map_name} world coordinates")


def project(archive_raw, spawn_raw, collision_raw, map_name=DEFAULT_MAP):
    pins = map_pins(map_name)
    validate_bindings(spawn_raw, collision_raw, map_name)
    require(sha(archive_raw) == pins["level_sha256"], f"wrong original {map_name} level archive")
    extractor = runpy.run_path(str(TOOLS / "build-retail-spawn-catalog.py"))["extractor"]()
    archive = extractor.Psarc(archive_raw)
    raw = archive.extract(archive.names.index(pins["member"]) + 1)
    require(sha(raw) == pins["clv_sha256"], f"wrong original {map_name} CLV")
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
    require(len(providers) == pins["providers"],
            f"{map_name} must have {pins['providers']} qualified replacement providers")
    require(len({row["name_token"] for row in providers}) == len(providers),
            "ambiguous supply point name token")
    return {
        "schema_version": 1, "application_id": 20231, "map": map_name, "mode": 0,
        "spawn_sha256": sha(spawn_raw), "collision_sha256": sha(collision_raw),
        "source": {"archive_sha256": pins["level_sha256"], "clv_sha256": pins["clv_sha256"],
                   "qualification_sha256": pins["qualification_sha256"]},
        "providers": providers,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("--level-archive", "--spawn-catalog", "--collision-world", "--out"):
        parser.add_argument(name, required=True, type=Path)
    parser.add_argument("--map", choices=sorted(QUALIFIED), default=DEFAULT_MAP,
                        help=f"retail Sabotage map to project (default {DEFAULT_MAP})")
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        files = [args.level_archive, args.spawn_catalog, args.collision_world, args.out]
        if args.receipt is not None:
            files.append(args.receipt)
        require(len(set(path.resolve() for path in files)) == len(files), "input and outputs must be distinct")
        rules = project(args.level_archive.read_bytes(), args.spawn_catalog.read_bytes(),
                        args.collision_world.read_bytes(), args.map)
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
