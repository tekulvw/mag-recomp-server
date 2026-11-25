#!/usr/bin/env python3
"""Project a bot-only provider catalog from one original retail level.

Supported: the three Sabotage maps and the six Acquisition and Domination maps.
"""
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
HUMAN = runpy.run_path(str(Path(__file__).resolve().parent / "project-retail-human-resupply.py"))
ACQDOM_MAPS = HUMAN["MAPS"]
DEFAULT_MAP = "mag_cosmodrome_final"
# Sabotage keeps its explicit visible, unfrozen, unsnapped allowlist. On the
# Acquisition and Domination maps that allowlist selects nothing on Canal and
# Panama, so those maps select every authored placement under an identity
# parent: the same placements human static resupply serves on these maps.
VISIBLE = "visible_unfrozen_unsnapped"
AUTHORED = "authored_identity_parent"
# Per-map qualification: the selection, the expected provider count it yields
# and the SHA-256 of the private witness that qualified that selection. Level
# archive and CLV identities come from the pinned Sabotage and Acquisition/
# Domination map tables.
QUALIFIED = {
    "mag_cosmodrome_final": dict(
        providers=3, selection=VISIBLE,
        qualification_sha256="f24127b06399d1353e4bb1c1a73943278227cd31a1488eda409096099b089b0d"),
    "mag_kennicott_final": dict(
        providers=2, selection=VISIBLE,
        qualification_sha256="39f2839c01b2251b84acac85b11cbcc0e66fae460afde11017abc03ff3840d06"),
    "mag_remote_comms_final": dict(
        providers=3, selection=VISIBLE,
        qualification_sha256="0198100a8c9bae0ae20b9df17a3cfa4b0ee4b3f689a00828abd47a1f744c3a1c"),
    "mag_alaska_subpen_final": dict(
        providers=10, selection=AUTHORED,
        qualification_sha256="4b4aedc1ac7efea8ac319783ce52b61ba26c86950e953bdf7323765bcf366d6a"),
    "mag_panama_military_base_final": dict(
        providers=9, selection=AUTHORED,
        qualification_sha256="9ab8eaf4351a2832534a79f20eca934bdde28276c94125dd579e50fa817b0a7b"),
    "mag_voz_final": dict(
        providers=11, selection=AUTHORED,
        qualification_sha256="93fba73764fad8c88230d999f5646e1e877fa4a93cce442d94a70e8d3408fde9"),
    "mag_baku_final": dict(
        providers=20, selection=AUTHORED,
        qualification_sha256="56b3915d59e7d05a31e8f375a48eb65db6aee97977dc60eeadad4f158bc63181"),
    "mag_canal_final": dict(
        providers=20, selection=AUTHORED,
        qualification_sha256="8d85fa6ee3720c69f1dd5f9541bf248d2b5f9d0022ee18b6e04dbf928d757c5a"),
    "mag_unalaska_final": dict(
        providers=21, selection=AUTHORED,
        qualification_sha256="5214ed98dd87a769c2c9718630fe67ccbe8d2d54926f00c1fda7ca25264814fd"),
}
CLASS = "SupplyPointObjectProperties"
QUALIFIED_START_TOLERANCE_M = 1.0
require, sha = SHARED["require"], SHARED["sha"]


def map_pins(map_name):
    require(map_name in QUALIFIED and (map_name in SABOTAGE_MAPS) != (map_name in ACQDOM_MAPS),
            f"unsupported retail map: {map_name}")
    if map_name in SABOTAGE_MAPS:
        level = SABOTAGE_MAPS[map_name]
        mode, level_sha256, clv_sha256 = 0, level["level_sha256"], level["clv_sha256"]
    else:
        level = ACQDOM_MAPS[map_name]
        mode, level_sha256, clv_sha256 = level.mode, level.archive_sha, level.clv_sha
    pins = dict(QUALIFIED[map_name], mode=mode, level_sha256=level_sha256, clv_sha256=clv_sha256,
                member=f"/run/mag/ps3/levels/{map_name}/{map_name}.clv")
    require(pins["selection"] == (VISIBLE if mode == 0 else AUTHORED), "unsupported map selection")
    return pins


def values(fields):
    result = {field["name"]: field["value"] for field in fields}
    require(len(result) == len(fields), "duplicate authored fields")
    return result


def provider(row, definitions, selection=VISIBLE):
    definition = definitions.get(row["preceding_token"])
    if definition is None:
        return None
    fields = values(row["fields"])
    visible, frozen, snapped = (fields[key] for key in
                                ("IsVisible_M", "IsFrozen_M", "SnapToTerrain_M"))
    require(all(type(value) is bool for value in (visible, frozen, snapped)), "invalid placement flags")
    require(selection in (VISIBLE, AUTHORED), "unsupported provider selection")
    if selection == VISIBLE and (not visible or frozen or snapped):
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
    pins = map_pins(map_name)
    spawns = json.loads(spawn_raw)
    mode = spawns["match_profile"]["game_type"]
    require(spawns["active"]["map_name"] == map_name and type(mode) is int and mode == pins["mode"],
            f"spawn catalog must select {map_name} in mode {pins['mode']}")
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
    instances = clv.tables("InstanceProperties")
    providers = [row for instance in instances
                 if (row := provider(instance, definitions, pins["selection"])) is not None]
    if pins["selection"] == AUTHORED:
        # Every selected placement must be a depth-two child of an identity
        # parent, so its authored translation is its world position.
        nodes = HUMAN["prefab_paths"](clv, instances)
        paths = {int(row["offset"], 16): path for path, row in nodes.items()}
        for row in providers:
            path = paths.get(row["source_offset"])
            require(path is not None and len(path) == 2
                    and HUMAN["identity_parent"](clv, nodes[path[:1]]),
                    "supply placement requires an identity parent")
    require(len(providers) == pins["providers"],
            f"{map_name} must have {pins['providers']} qualified replacement providers")
    require(len({row["name_token"] for row in providers}) == len(providers),
            "ambiguous supply point name token")
    return {
        "schema_version": 1, "application_id": 20231, "map": map_name, "mode": pins["mode"],
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
                        help=f"retail map to project (default {DEFAULT_MAP})")
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
                          "selection": QUALIFIED[args.map]["selection"], **rules["source"]})
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
