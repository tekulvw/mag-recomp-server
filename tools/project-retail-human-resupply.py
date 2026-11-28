#!/usr/bin/env python3
"""Project source-qualified Sabotage, Acquisition and Domination static resupply providers."""
import argparse
from collections import Counter
import json
import math
from pathlib import Path
import runpy
import struct
from typing import NamedTuple


ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools/extraction"
SPAWN = runpy.run_path(str(TOOLS / "extract-retail-spawns.py"))
SHARED = runpy.run_path(str(TOOLS / "extract-retail-objective-volumes.py"))
MAP_HELPERS = runpy.run_path(str(TOOLS / "retail_map_helpers.py"))
NATIVE = runpy.run_path(str(TOOLS / "extract-retail-falling-damage.py"))
require, sha = SHARED["require"], SHARED["sha"]
RETAIL_SHA = NATIVE["SOURCES"]["retail"]
RULES_ARCHIVE_SHA = NATIVE["SOURCES"]["archive"]
DOM_SPAWN_BINDINGS_SHA = "7ef4dd3e613b001dd27bdbfae4140e9b320ca9e6f2eab6ef1c68e64c7f989918"
SCOPE = "retail_fresh_successful_compiled_load"
SUPPLY_CLASS = "SupplyPointObjectProperties"
IDENTITY = (1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)


class MapSource(NamedTuple):
    mode: int
    defender: int
    archive_sha: str
    clv_sha: str
    instance_count: int
    provider_count: int


MAPS = {
    "mag_cosmodrome_final": MapSource(0, 2,
        "8157f5abc55ddd2ca9eaad3379e87ae909d0b8f7c9b08fa04f1f1e2d8018d96e",
        "d336caea88351dda5f3204a34e64fdde8eabdf0532b3133e2338cdd150488f4b", 4558, 5),
    "mag_kennicott_final": MapSource(0, 0,
        "399a38f568f4cd4c9c1d787b8ebbe857506b807638dc19f7e2e893ef051332ec",
        "f17814c8eea43f636541059c8eb910d3707ced53ff9d1cc87d0ad83300825b56", 3581, 4),
    "mag_remote_comms_final": MapSource(0, 1,
        "f066f00a74a58d36a531526f4216538300362d5f358a0f40c6b75f2edc226313",
        "71f5dd11198a5d8d34055276a0fa031f8b8e833f2b8a6c6b9edb9fc4f3a1c888", 5787, 5),
    "mag_alaska_subpen_final": MapSource(1, 0,
        "fb0e9c12653dcecd9c6f444ec6c5a997aec7f99f481f7951065bf4b64a39f7a0",
        "b4c2cfe7db94901b98c10a11f1d57a52cfed4941e061196627b3a753a3fe3a7e", 8506, 10),
    "mag_panama_military_base_final": MapSource(1, 1,
        "232e7abf68e89c669bcf6b3335830748975ce4216d3f5ed6b8b3317910e6b1f2",
        "c054fe57e5fc8b81f2769368ac756cff36eafc8bf071cccc64ff912e330f1c6e", 9908, 9),
    "mag_voz_final": MapSource(1, 2,
        "e8bf8760097604ef7186583ef9f1208f5d8dc5fddc4f972f30990a79590cead4",
        "19f7fc0bcf9877633776ca5b1ae4910404499f79315003f5f0671e536c355a65", 8535, 11),
    "mag_baku_final": MapSource(2, 2,
        "e355622ac2b42c91d9aa22c0b46f80cbbc190dc61cc75c710157d08b4ba674e7",
        "64add78c0f427360d58f82afa09eacf3abe743eb6bb5f57ae236a1c733f39e6f", 12164, 20),
    "mag_canal_final": MapSource(2, 1,
        "90295495a150c06762284b15de411fca11657557cb228fba8f854ee68ee6059e",
        "7eff93236d9c5d2f56aadcb406cb7095ea9d69a18d96407e0801343ee859fdce", 11530, 20),
    "mag_unalaska_final": MapSource(2, 0,
        "bbd2f66f3664aca428b04f879e9268df121b6d7fb4ca155f3e0b53a7a241b8ed",
        "bdc4a33b6f3542ac84756f1bd22f6babc30f150aa07dbcc8ccf7e62a94aff0c3", 11959, 21),
}


def native_data(elf):
    # Elf verifies the full supported original before resolving these layout sites.
    word = elf.word
    classes = (
        (SUPPLY_CLASS, 0x1AD5EB0, 0x4C, 0xE19EB0),
        ("SupplyPointLevelObject", 0x1A71730, 0x10, 0x735E80),
        ("SimLevelSupplyPoint", 0x1AD6368, 0x10, 0xE1BF50),
    )
    for name, table, slot, entry in classes:
        name_bytes = f"N13zWorldObjects{len(name)}{name}E".encode() + b"\0"
        name_address = word(word(table - 4) + 4)
        require(elf.read(name_address, len(name_bytes))[0] == name_bytes
                and word(word(table + slot)) == entry, f"unsupported native {name} class")
    mask_entry = word(word(0x1AD5EB0 + 0x48))
    require(word(mask_entry) == 0x38600006, "static supply must participate in client and Sim passes")

    # Census the original automatic-ID request form in the executable image.
    requests = []
    for offset, address, length, flags in elf.segments:
        if flags & 1:
            raw = memoryview(elf.raw)[offset:offset + length // 4 * 4]
            requests.extend(address + index * 4 for index, (value,) in enumerate(struct.iter_unpack(">I", raw))
                            if value & 0xFC1FFFFF == 0x3C002A80)
    require(requests == [0x735EDC, 0xE1BF84]
            and word(0x735EE4) == word(0xE1BF8C) == 0x6084FFFF,
            "unexpected static family-42 automatic allocation sites")
    family = word(0x72104) & 0xFFFF
    pool_capacity = word(0x72100) & 0xFFFF
    require(family == 42 and pool_capacity == 32
            and word(0x72104) == word(0x90911C)
            and word(0x72100) == word(0x909118)
            and word(0x1B4ADAC) != word(0x1B96F38),
            "static supply client and Sim pool registration differs")
    require(word(0x2FB2C4) == 0xA1250002 and word(0x2FB394) == 0x64000080
            and word(0x2FB3C4) == 0x90040004 and word(0x2F6858) == 0x7D3DF12E,
            "unsupported native automatic slot allocation")
    require(word(0x735FBC) == 0x80030018 and word(0x735FC0) == 0x901B0070
            and word(0x6521F4) == 0x80190070 and word(0x6521F8) == 0x90090004
            and word(0x67E348) == 0x9123001C and word(0x67EED0) == 0x801B001C
            and word(0x661030) == 0x801E001C,
            "unsupported native selected target dataflow")

    # Original Start passes its tolerance to the installed box membership query.
    tolerance, half_extent = [struct.unpack(">f", elf.read(address, 4)[0])[0]
                              for address in (0x1B55870 - 14784, 0x1B55870 - 14472)]
    require(all(math.isfinite(value) and value > 0 for value in (tolerance, half_extent)),
            "invalid native static supply range constants")
    ranges = {
        "recursive_prefab": (0x33D7BC, 0x33D874),
        "level_loaded_sorted_callback": (0xF82FF0, 0xF84078),
        "client_registered_array": (0x720E0, 0x72110),
        "sim_registered_array": (0x909108, 0x909128),
        "configure_resets_array": (0x2F872C, 0x2F87B8),
        "preserved_native_family": (0x2FC94C, 0x2FC984),
        "first_clear_slot": (0x2FB26C, 0x2FB3E8),
        "successful_allocation_commit": (0x2F6800, 0x2F6870),
        "client_level_loaded": (0x735E80, 0x736040),
        "sim_level_loaded": (0xE1BF50, 0xE1C060),
        "selected_target_state_transition": (0x652188, 0x652204),
        "interaction_state_target": (0x67E338, 0x67E35C),
        "interaction_start": (0x67EEC4, 0x67EF0C),
        "interaction_stop": (0x661024, 0x66106C),
        "sim_interaction_lookup": (0x789924, 0x789A10),
        "sim_authored_box": (0x86AE00, 0x86B1B4),
        "sim_start_range": (0x868F88, 0x868FE4),
        "sim_box_membership": (0x86A200, 0x86A3AC),
    }
    return {"family": family, "pool_capacity": pool_capacity,
            "start_tolerance_m": tolerance, "unit_half_extent": half_extent,
            "direct_request_addresses": requests,
            "spans": {name: {"address": start, "size": end - start, "sha256": sha(elf.read(start, end - start)[0])}
                      for name, (start, end) in ranges.items()}}


def fields_by_name(row):
    fields = {field["name"]: field for field in row["fields"]}
    require(len(fields) == len(row["fields"]), "duplicate authored fields")
    return fields


def parse_definition(clv):
    definitions = []
    for match in SHARED["CLASS_HEADER"].finditer(clv.b):
        header = match.start()
        if match[1] != SUPPLY_CLASS.encode() or header < 16 or clv.u32(header - 16) != 7:
            continue
        token, end = clv.u32(header - 4), clv.u32(header - 8)
        require(token != 0 and match.end() < end <= len(clv.b), "invalid authored supply definition")
        fields, stop = clv.fields(match.end(), end)
        values = fields_by_name({"fields": fields})
        tick = values["TickRate"]["value"]
        refresh = values["PercentRefreshRate"]["value"]
        require(stop + 1 == end and values["UsesNonUniformScale"]["value"] is True
                and type(tick) is float and math.isfinite(tick) and tick > 0
                and type(refresh) is float and math.isfinite(refresh) and 0 < refresh <= 100,
                "invalid authored static supply rates or shape")
        definitions.append({"token": token, "offset": header - 16, "sha256": sha(clv.b[header - 16:end]),
                            "tick_rate_seconds": tick, "percent_refresh_rate": refresh})
    require(len(definitions) == 1, "expected a unique authored static supply definition")
    return definitions[0]


def prefab_paths(clv, rows):
    """Walk contiguous compiled prefab records in the original root and child order."""
    nodes = []
    for row in sorted(rows, key=lambda row: int(row["offset"], 16)):
        offset, end = (int(row[key], 16) for key in ("offset", "end"))
        start = offset - 12
        require(4 <= start < end <= len(clv.b) - 13
                and clv.u32(start) == 5 and clv.u32(start + 4) == row["preceding_token"]
                and clv.u32(offset - 4) == 7 and sha(clv.b[offset - 4:end]) == row["source_sha256"],
                "invalid compiled prefab record")
        geometry_present = clv.b[end + 8]
        require(geometry_present in (0, 1), "invalid compiled prefab geometry flag")
        cursor = end + 9
        if geometry_present:
            collision = clv.u32(end + 13)
            require(end + 17 <= collision < len(clv.b) - 4
                    and clv.u32(cursor) == clv.u32(collision) == 0x10001
                    and clv.b[collision + 4] in (0, 1), "invalid compiled prefab geometry")
            cursor = collision + (77 if clv.b[collision + 4] else 5)
        require(cursor + 4 <= len(clv.b), "compiled prefab child count outside source")
        nodes.append((start, cursor + 4, clv.u32(cursor), row))
    require(nodes and all(a[1] == b[0] for a, b in zip(nodes, nodes[1:])), "compiled prefab coverage has a gap")

    expected_roots = clv.u32(nodes[0][0] - 4)
    stack = []
    paths = {}
    root = 0
    for _, _, count, row in nodes:
        while stack and stack[-1][1] == stack[-1][2]:
            stack.pop()
        if stack:
            parent, index, total = stack[-1]
            path = (*parent, index)
            stack[-1] = (parent, index + 1, total)
        else:
            path = (root,)
            root += 1
        paths[path] = row
        if count:
            stack.append((path, 0, count))
    require(root == expected_roots and all(index == count for _, index, count in stack),
            "compiled prefab coverage has incomplete roots or children")
    return paths


def identity_parent(clv, row):
    end = int(row["end"], 16)
    return (row["preceding_token"] == 0 and clv.b[end + 8] == 1
            and clv.u32(end + 9) == 0x10001 and clv.u32(end + 13) == end + 101
            and clv.u32(end + 17) == 7 and clv.u32(end + 21) == end + 29
            and clv.u32(end + 25) == clv.u32(end + 29) == 0
            and struct.unpack_from(">16f", clv.b, end + 33) == IDENTITY)


def project_providers(clv, instances, definition, native):
    paths = prefab_paths(clv, instances)
    fields = {path: fields_by_name(row) for path, row in paths.items()}
    require(all(row["ObjectID"]["type"] == "uint32" and row["ObjectID"]["value"] == 0xFFFFFFFF
                for row in fields.values()), "expected automatic LevelObject ID on every compiled placement")
    names = Counter(row["Name"]["token"] for row in fields.values())
    # The native sorted callback order follows ascending automatic LevelObject
    # slots, so other prefab classes can leave gaps without changing this order.
    scheduled = [(path, row) for path, row in paths.items() if row["preceding_token"] == definition["token"]]
    require(0 < len(scheduled) <= native["pool_capacity"], "static supply placement count outside native pool")
    providers = []
    for slot, (path, row) in enumerate(scheduled):
        source_fields = fields[path]
        name = source_fields["Name"]["token"]
        require(int(name, 16) != 0 and names[name] == 1, "expected unique authored Name on static supply")
        require(len(path) == 2 and path[0:1] in paths and identity_parent(clv, paths[path[0:1]]),
                "static supply requires an identity parent with no prefab token")
        box = SHARED["box"]({key: field["value"] for key, field in source_fields.items()},
                            MAP_HELPERS["euler_basis"])
        providers.append({
            "packed_target": (native["family"] << 24) | 0x00800000 | slot,
            "low16": slot,
            "tick_rate_seconds": definition["tick_rate_seconds"],
            "percent_refresh_rate": definition["percent_refresh_rate"],
            "box": {"center": box["centre"], "axes": box["axes"],
                    "euler_xyz_degrees": box["rotate_degrees"], "scales_xyz": box["scale"],
                    "start_tolerance_m": native["start_tolerance_m"], "unit_half_extent": native["unit_half_extent"]},
            "source": {"instance_offset": int(row["offset"], 16), "instance_name_token": int(name, 16),
                       "instance_sha256": row["source_sha256"], "prefab_path": list(path),
                       "parent_instance_sha256": paths[path[:-1]]["source_sha256"]},
        })
    return providers


def validate_profile(spawn_raw, world_raw):
    spawn = json.loads(spawn_raw)
    map_name = spawn["active"]["map_name"]
    level = MAPS.get(map_name)
    require(level is not None, f"unsupported static supply map {map_name}")
    profile = spawn["match_profile"]
    mode, attacker = profile["game_type"], profile["attacking_faction"]
    require(spawn["format"] == "mag_spawn_catalog" and type(spawn["schema_version"]) is int
            and spawn["schema_version"] == 1 and type(mode) is int and mode == level.mode
            and type(attacker) is int and attacker in (0, 1, 2) and attacker != level.defender,
            f"spawn catalog must select a qualified mode and attacking faction for {map_name}")
    map_rows = [row for row in spawn["maps"] if row["name"] == map_name]
    require(len(map_rows) == 1 and type(map_rows[0]["owner_faction"]) is int
            and map_rows[0]["owner_faction"] == level.defender, "wrong spawn catalog defending faction")
    source = json.loads(spawn["source"])
    require(source["map"] == map_name and source["level_archive_sha256"] == level.archive_sha
            and source["level_clv_sha256"] == level.clv_sha,
            f"spawn catalog source must match supported original {map_name}")
    if mode == 0:
        # Sabotage spawn catalogs bind the original rules archive; the supplied
        # ELF is checked directly and the worker binds it again through the
        # profile's objective catalog.
        require(source.get("game_archive_sha256") == RULES_ARCHIVE_SHA
                and source.get("native_elf_sha256", RETAIL_SHA) == RETAIL_SHA,
                "wrong Sabotage rules archive source")
    elif mode == 1:
        require(source.get("native_elf_sha256") == RETAIL_SHA, "wrong Acquisition native ELF source")
    else:
        # Current Domination spawn catalogs bind the qualified original
        # family-35 witness, which carries this same original ELF identity.
        require(source.get("native_spawn_bindings_sha256") == DOM_SPAWN_BINDINGS_SHA
                and source.get("native_elf_sha256", RETAIL_SHA) == RETAIL_SHA,
                "wrong Domination native spawn witness")
    world = json.loads(world_raw)
    metadata = world["metadata"]
    require(world["format"] == "mag_collision_world" and type(world["schema_version"]) is int
            and world["schema_version"] == 1 and world["coordinate_space"] == "world"
            and metadata["map_name"] == map_name and metadata["build"] == "retail-bcus98110-01.00"
            and isinstance(metadata["provenance"], list)
            and f"Original CLV SHA256 {level.clv_sha}" in metadata["provenance"],
            f"collision world source must match supported original {map_name}")
    return {"map": map_name, "mode": mode, "attacking_faction": attacker,
            "spawn_sha256": sha(spawn_raw), "collision_sha256": sha(world_raw)}


def project(elf, archive_raw, spawn_raw, world_raw):
    profile = validate_profile(spawn_raw, world_raw)
    map_name = profile["map"]
    level = MAPS[map_name]
    member = f"/run/mag/ps3/levels/{map_name}/{map_name}.clv"
    require(sha(archive_raw) == level.archive_sha, f"unsupported original {map_name} level archive")
    archive = SPAWN["Psarc"](archive_raw)
    require(archive.names.count(member) == 1, f"expected one original {map_name} CLV member")
    raw = archive.extract(archive.names.index(member) + 1)
    require(sha(raw) == level.clv_sha, f"unsupported original {map_name} CLV")
    native = native_data(elf)
    clv = SPAWN["retail_clv"](raw)
    definition = parse_definition(clv)
    instances = clv.tables("InstanceProperties")
    require(len(instances) == level.instance_count, f"incomplete original {map_name} prefab inventory")
    providers = project_providers(clv, instances, definition, native)
    require(len(providers) == level.provider_count, f"incomplete original {map_name} supply allocation roster")
    return {
        "schema_version": 1, "format": "mag_retail_human_static_resupply", "application_id": 20231,
        "entity_family": native["family"], **profile,
        "source": {"retail_elf_sha256": RETAIL_SHA, "level_archive_sha256": level.archive_sha,
                   "clv_sha256": level.clv_sha, "clv_member": member, "id_scope": SCOPE,
                   "allocation_pool_capacity": native["pool_capacity"],
                   "allocation_proof_sha256": sha(encode(native)),
                   "definition": {name: definition[name] for name in ("offset", "token", "sha256")}},
        "providers": providers,
    }, native


def encode(value):
    return (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()


def write_output(path, raw, *, check):
    if check:
        require(path.read_bytes() == raw, f"projected bytes differ from {path}")
    else:
        with path.open("xb") as file:
            file.write(raw)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("--elf", "--level-archive", "--spawn-catalog", "--collision-world", "--out"):
        parser.add_argument(name, required=True, type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--check", action="store_true", help="verify existing output bytes without writing")
    args = parser.parse_args()
    try:
        outputs = [args.out] if args.receipt is None else [args.out, args.receipt]
        all_paths = [args.elf, args.level_archive, args.spawn_catalog, args.collision_world, *outputs]
        require(len({path.resolve() for path in all_paths}) == len(all_paths), "inputs and outputs must be distinct")
        require(all(not path.resolve().is_relative_to(ROOT) for path in outputs),
                "outputs must be outside the source checkout")
        require(args.check or not any(path.exists() for path in outputs),
                "outputs already exist, use --check or choose new paths")
        rules, native = project(NATIVE["Elf"](args.elf, "retail"), args.level_archive.read_bytes(),
                                args.spawn_catalog.read_bytes(), args.collision_world.read_bytes())
        raw = encode(rules)
        write_output(args.out, raw, check=args.check)
        if args.receipt is not None:
            receipt = {"schema_version": 1, "catalog_sha256": sha(raw), "source": rules["source"],
                       "spawn_sha256": rules["spawn_sha256"], "collision_sha256": rules["collision_sha256"],
                       "native": native}
            write_output(args.receipt, encode(receipt), check=args.check)
    except (AssertionError, OSError, ValueError, KeyError, TypeError, struct.error) as error:
        parser.exit(1, f"error: {error}\n")


if __name__ == "__main__":
    main()
