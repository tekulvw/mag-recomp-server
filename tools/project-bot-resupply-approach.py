#!/usr/bin/env python3
"""Project bounded bot supply route candidates onto the loaded navigation data."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess

MAX_ANCHORS = 16
MAX_SAMPLES = 16


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def file_sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def read_policy_input(path):
    with path.open("rb") as source:
        raw = source.read(65_537)
    require(len(raw) <= 65_536, "policy input exceeds size limit")
    return raw


def require(condition, message):
    if not condition:
        raise ValueError(message)


def number(value):
    return type(value) in (float, int) and math.isfinite(value)


def integer(value, lower, upper):
    return type(value) is int and lower <= value <= upper


def point(value):
    return (isinstance(value, list) and len(value) == 3
            and all(number(x) and abs(x) <= 100_000 for x in value))


def in_box(provider, position):
    delta = [value - center for value, center in zip(position, provider["center"])]
    return all(abs(sum(vector * coordinate for vector, coordinate in zip(axis, delta)))
               <= scale * 0.5 + provider["tolerance_m"]
               for axis, scale in zip(provider["axes"], provider["scales_xyz"]))


def project(provider_raw, world_sha, parameters, query):
    source = json.loads(provider_raw)
    require(integer(source["schema_version"], 1, 1) and integer(source["application_id"], 20231, 20231)
            and integer(source["mode"], 0, 0) and source["collision_sha256"] == world_sha,
            "provider catalog does not bind the current world")
    samples = parameters["sample_offsets"]
    require(isinstance(samples, list) and 1 <= len(samples) <= MAX_SAMPLES
            and all(point(offset) and math.hypot(*offset) <= 5 for offset in samples)
            and len(set(map(tuple, samples))) == len(samples), "invalid projection samples")
    projection_radius = parameters["projection_radius_m"]
    require(number(projection_radius) and 0.05 <= projection_radius <= 5,
            "invalid projection radius")
    knobs = {key: parameters[key] for key in ("query_budget_per_tick", "retry_ms", "goal_radius_m",
                                             "max_route_length_m", "seek_max_duration_ms")}
    extra = {"allowed_drop_id", "planning_window_ms"} if ("allowed_drop_id" in parameters
            or "planning_window_ms" in parameters) else set()
    require(set(parameters) == knobs.keys() | {"sample_offsets", "projection_radius_m"} | extra
            and (not extra or (integer(parameters["allowed_drop_id"], 1, 2**32-1)
                 and integer(parameters["planning_window_ms"], 1_000, 10_000)
                 and parameters["planning_window_ms"] <= knobs["seek_max_duration_ms"]
                 and knobs["retry_ms"] < parameters["planning_window_ms"]))
            and integer(knobs["query_budget_per_tick"], 1, 16)
            and integer(knobs["retry_ms"], 1, 2**32-1)
            and integer(knobs["seek_max_duration_ms"], 1_000, 120_000)
            and number(knobs["goal_radius_m"]) and 0.05 <= knobs["goal_radius_m"] <= 5
            and number(knobs["max_route_length_m"]) and 0 < knobs["max_route_length_m"] <= 192,
            "invalid approach policy")
    if extra:
        knobs.update({key: parameters[key] for key in extra})
    projected, seen_keys = [], set()
    detour_sha = None
    require(isinstance(source["providers"], list) and 1 <= len(source["providers"]) <= 16,
            "invalid provider inventory")
    for provider in source["providers"]:
        key = (provider["source_offset"], provider["name_token"])
        require(all(integer(value, 1, 2**32-1) for value in key) and key not in seen_keys
                and point(provider["center"]) and point(provider["scales_xyz"])
                and all(value > 0 for value in provider["scales_xyz"])
                and number(provider["tolerance_m"]) and provider["tolerance_m"] >= 0
                and len(provider["axes"]) == 3 and all(point(axis) for axis in provider["axes"]),
                "invalid source provider geometry or identity")
        seen_keys.add(key)
        anchors = {}
        for offset in samples:
            seed = [base + shift for base, shift in zip(provider["center"], offset)]
            result = query(seed, projection_radius)
            digest = result["detour_sha256"]
            require(isinstance(digest, str) and re.fullmatch("[0-9a-f]{64}", digest)
                    and (detour_sha is None or detour_sha == digest)
                    and result["map"] == source["map"] and result["world_sha256"] == world_sha,
                    "navigation candidate provenance mismatch")
            detour_sha = digest
            rows = result["candidates"]
            require(isinstance(rows, list) and len(rows) <= 128
                    and all(integer(candidate["component"], 0, 2**31-1)
                            and point(candidate["point"]) for candidate in rows),
                    "invalid navigation candidate response")
            for candidate in sorted(rows, key=lambda row: (row["component"], row["point"])):
                position = candidate["point"]
                if in_box(provider, position):
                    anchors[tuple(position)] = position
        require(1 <= len(anchors) <= MAX_ANCHORS, "invalid projected anchor count")
        projected.append({"source_offset": key[0], "name_token": key[1],
                          "anchors": list(anchors.values())})
    projected.sort(key=lambda row: (row["source_offset"], row["name_token"]))
    return {"schema_version": 1, "application_id": 20231, "map": source["map"], "mode": 0,
            "provider_catalog_sha256": sha(provider_raw), "collision_sha256": world_sha,
            "detour_sha256": detour_sha, **knobs, "providers": projected}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("--provider-catalog", "--mesh", "--collision-world", "--navmesh-cli", "--parameters", "--out"):
        parser.add_argument(name, required=True, type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    def query(seed, radius):
        command = [str(args.navmesh_cli.resolve()), "goals", "--mesh", str(args.mesh),
                   "--collision-world", str(args.collision_world), "--goal", ",".join(map(str, seed)),
                   "--goal-radius", str(radius)]
        return json.loads(subprocess.check_output(command, timeout=30))

    try:
        inputs = {"provider_catalog_sha256": args.provider_catalog, "mesh_sha256": args.mesh,
                  "collision_sha256": args.collision_world, "navigation_cli_sha256": args.navmesh_cli,
                  "parameters_sha256": args.parameters}
        outputs = [path for path in (args.out, args.receipt) if path is not None]
        output_paths = {path.resolve() for path in outputs}
        require(len(output_paths) == len(outputs)
                and output_paths.isdisjoint(path.resolve() for path in inputs.values()),
                "outputs and inputs must use distinct paths")
        provider_raw = read_policy_input(args.provider_catalog)
        parameters_raw = read_policy_input(args.parameters)
        digests = {"provider_catalog_sha256": sha(provider_raw), "parameters_sha256": sha(parameters_raw),
                   **{name: file_sha(inputs[name]) for name in
                      ("mesh_sha256", "collision_sha256", "navigation_cli_sha256")}}
        rules = project(provider_raw, digests["collision_sha256"], json.loads(parameters_raw), query)
        encode = lambda value: (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()
        encoded = encode(rules)
        receipt = encode({"schema_version": 1, "output_sha256": sha(encoded),
                          "detour_sha256": rules["detour_sha256"], "inputs": digests,
                          "scope": "authored provider boxes and operator-selected projection samples; "
                                   "candidate anchors only"})
        for path, data in ((args.out, encoded), (args.receipt, receipt)):
            if path is None:
                continue
            if args.check:
                require(path.read_bytes() == data, f"projected bytes differ from {path}")
            else:
                path.write_bytes(data)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        parser.exit(1, f"error: {error}\n")


if __name__ == "__main__":
    main()
