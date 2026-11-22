#!/usr/bin/env python3
"""Compose opted-in Acquisition APC crew inputs from owner-qualified projections."""
import argparse
import copy
import hashlib
import json
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    return (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()


def upgrade_ground(ground, indices):
    sources = {r["vehicle_index"]: r["spawn_type"] for r in ground["vehicle_sources"]}
    require(ground["schema_version"] == 1 and len(indices) == 4
            and indices == {i for i, role in sources.items() if role == 3},
            "Acquisition APC roster mismatch")
    require(len(ground["driver_seats"]) == 4
            and {d["vehicle_index"] for d in ground["driver_seats"]}
            == {i for i, role in sources.items() if role == 9},
            "Acquisition Escort driver roster mismatch")
    result = copy.deepcopy(ground)
    result["schema_version"] = 2
    by_index = {v["index"]: v for v in result["vehicles"]}
    visibility = {r["vehicle_index"]: r for r in result["seat_visibility"]["vehicles"]}
    for index in sorted(indices):
        vehicle = by_index[index]
        require(vehicle["passenger_seats"] == list(range(2, 10)) and vehicle["maximum_occupancy"] == 8
                and bool(vehicle["passenger_exit_candidates"]) and visibility[index]["seats"] == [
                    {"seat": seat, "occupant_visible": False} for seat in range(2, 10)],
                "Acquisition APC seats or exit qualification mismatch")
        # Reuse the established replacement moving APC admission policy.
        vehicle.update(maximum_mount_distance_m=8., maximum_mount_position_age_ms=750)
        result["driver_seats"].append({"vehicle_index": index, "seat": 0, "maximum_speed_mps": 25.})
        visibility[index]["seats"].insert(0, {"seat": 0, "occupant_visible": False})
    return result


def compose(registry_path, receipt_path, apc_path, escort_path):
    registry_raw = registry_path.read_bytes()
    receipt = json.loads(receipt_path.read_bytes())
    pins = receipt["source_inputs_sha256"]
    require(pins.get(str(registry_path)) == sha(registry_raw), "crew registry differs from qualification")
    witnessed = {row["profile_id"]: row for row in receipt["profiles"]}
    apc = json.loads(apc_path.read_bytes())
    escort = json.loads(escort_path.read_bytes())
    require(apc["schema"] == "mag_domination_apc_turrets_v1"
            and escort["schema"] == "mag_acquisition_escort_turrets_v1"
            and len(apc["variants"]) == 3 and len(escort["profiles"]) == len(witnessed) == 6,
            "qualified turret source roles or coverage differ")
    variants = {v["faction"]: v for v in apc["variants"]}
    escort_profiles = {p["spawn_catalog_sha256"]: p for p in escort["profiles"]}
    output = {}
    profiles = []
    for row in json.loads(registry_raw)["profiles"]:
        witness = witnessed.pop(row["profile_id"])
        paths = {key: (registry_path.parent / row[key]).resolve() for key in ("spawn_catalog", "ground_vehicles")}
        raw = {key: path.read_bytes() for key, path in paths.items()}
        require(all(pins.get(str(path)) == sha(raw[key]) for key, path in paths.items()),
                "crew spawn/Ground bytes differ from qualification")
        ground = json.loads(raw["ground_vehicles"])
        require(ground["source"]["native_elf_sha256"] == apc["source"]["retail_elf_sha256"]
                and ground["seat_visibility"]["archive_sha256"] == apc["source"]["retail_archive_sha256"],
                "APC turret and Ground source builds differ")
        placement = []
        for item in witness["apcs"]:
            vehicle = next(v for v in ground["vehicles"] if v["index"] == item["index"])
            faction = item["faction"]
            require(variants[faction]["primary_item_id"] == item["primary_item"]
                    and vehicle["vehicle_type_token"] == apc["source"]["vehicle_type_token"]
                    and (vehicle["spawn_point_index"], vehicle["spawn_point_token"], vehicle["faction"])
                    == (item["spawn_index"], int(item["spawn_token"], 16), faction),
                    "APC placement or qualified mounted weapon differs")
            placement.append({"vehicle_index": vehicle["index"], "spawn_point_index": vehicle["spawn_point_index"],
                              "spawn_point_token": vehicle["spawn_point_token"], "faction": faction})
        updated = encode(upgrade_ground(ground, {p["vehicle_index"] for p in placement}))
        name = paths["ground_vehicles"].name
        require(name not in output, "duplicate Ground output name")
        output[name] = updated
        spawn_sha = sha(raw["spawn_catalog"])
        old_escort = escort_profiles.pop(spawn_sha)
        require(old_escort["ground_catalog_sha256"] == sha(raw["ground_vehicles"])
                and old_escort["map_name"] == witness["map"], "Escort source changed before composition")
        old_escort["ground_catalog_sha256"] = sha(updated)
        profiles.append(dict(old_escort, vehicles=placement))
    require(not witnessed and not escort_profiles, "incomplete Acquisition APC crew coverage")
    apc["schema"] = "mag_acquisition_apc_turrets_v1"
    apc["profiles"] = sorted(profiles, key=lambda entry: (entry["map_name"], entry["spawn_catalog_sha256"]))
    output["apc-turrets.json"] = encode(apc)
    output["escort-turrets.json"] = encode(escort)
    output["proof.json"] = encode({
        "input_sha256": {name: sha(path.read_bytes()) for name, path in (
            ("registry", registry_path), ("qualification", receipt_path), ("apc_turrets", apc_path),
            ("escort_turrets", escort_path))},
        "output_sha256": {name: sha(data) for name, data in output.items()},
    })
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("registry", "qualification", "apc-turrets", "escort-turrets", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        result = compose(args.registry.resolve(), args.qualification, args.apc_turrets, args.escort_turrets)
        if args.check:
            require(all((args.output / name).read_bytes() == value for name, value in result.items()),
                    "APC crew projection differs")
        else:
            args.output.mkdir()
            for name, value in result.items():
                (args.output / name).write_bytes(value)
    except (OSError, ValueError, KeyError, TypeError, StopIteration) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
