#!/usr/bin/env python3
"""Build optional Acquisition projectile contact bindings from qualified owner inputs."""
import argparse
import hashlib
import json
from pathlib import Path
import re

def sha(path):
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def read(path, limit=2 * 1024 * 1024):
    with path.open("rb") as stream:
        raw = stream.read(limit + 1)
    if len(raw) > limit:
        raise ValueError(f"contact projection input too large: {path}")
    return json.loads(raw)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(value):
    return isinstance(value, str) and re.fullmatch("[0-9a-f]{64}", value) is not None


def project(map_name, qualification, filter_receipt, static_world, static_receipt,
            dynamic_catalog, sequence_index):
    proof = read(qualification)
    selected = proof["maps"][map_name]
    static = read(static_receipt)
    source_index = read(sequence_index)
    native_filters = read(filter_receipt)
    require(proof["native_filter_rows_receipt_sha256"] == sha(filter_receipt),
            "original filter receipt pin mismatch")
    allowed = {0}
    for a, b in native_filters["initial_filter_enable_pairs"]:
        require(type(a) is int and type(b) is int and 0 <= a < 32 and 0 <= b < 32,
                "invalid original filter layer")
        if a == 23:
            allowed.add(b)
        elif b == 23:
            allowed.add(a)
    filter_row23 = sum(1 << layer for layer in allowed)
    require(proof["row23"] == f"{filter_row23:#010x}", "contact filter row mismatch")
    require(static["map_name"] == source_index["map"] == map_name, "contact map mismatch")
    require(
        static["source_archive_sha256"] == source_index["level_archive_sha256"]
        and digest(static["source_elf_sha256"])
        and static["source_elf_sha256"] == native_filters["retail_sha256"],
        "contact archive or ELF lineage mismatch",
    )
    require(
        selected["static_receipt_sha256"] == sha(static_receipt)
        and selected["static_world_sha256"] == static["world_sha256"] == sha(static_world)
        and selected["sequence_index_sha256"] == sha(sequence_index)
        and selected["physics_sha256"] == source_index["physics_sha256"]
        and proof["dynamic_catalog_sha256"] == sha(dynamic_catalog),
        "contact source pins mismatch",
    )
    source_keys = {
        (row["source_sha256"], row["neutral_sha256"], row["collision_tag"])
        for group in source_index["resources"].values()
        for row in group
    }
    qualified = {}
    for pack in selected["sequence"]:
        source, neutral, root, parts = (
            pack[key] for key in ("source_sha256", "neutral_sha256", "root", "part_ids")
        )
        key = (source, neutral, root)
        require(
            digest(source) and digest(neutral) and root in (9, 11, 29)
            and parts and all(isinstance(part, str) and part.strip() for part in parts)
            and sorted(set(parts)) == parts and key not in qualified,
            "invalid qualified Sequence inventory",
        )
        qualified[key] = {
            "binding": {
                "source_sha256": source, "neutral_sha256": neutral,
                "collision_tag": root, "part_ids": parts,
            },
            "contact": {"root_filter_info": root, "child_filter_info": 0, "convex_radius": 0.0},
        }
    require(qualified and qualified.keys() == source_keys, "contact Sequence inventory mismatch")
    return {
        "schema": "mag_acquisition_projectile_contacts_v1",
        "mode": 1,
        "map_name": map_name,
        "retail_elf_sha256": static["source_elf_sha256"],
        "proof_sha256": sha(qualification),
        "static_world_sha256": static["world_sha256"],
        "dynamic_catalog_sha256": proof["dynamic_catalog_sha256"],
        "static_coverage_policy": "represented_reference_pose",
        "ground_phantom_roots": ["wheel"],
        "filter_row23": filter_row23,
        "sequence": [qualified[key] for key in sorted(qualified)],
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--map", required=True)
    for name in ("qualification", "filter-receipt", "static-world", "static-receipt", "dynamic-catalog",
                 "sequence-index", "output"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    result = project(args.map, args.qualification, args.filter_receipt, args.static_world,
                     args.static_receipt, args.dynamic_catalog, args.sequence_index)
    raw = (json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
    if len(raw) > 512 * 1024:
        raise ValueError("contact projection output too large")
    if args.check:
        require(args.output.read_bytes() == raw, "contact projection differs")
    else:
        with args.output.open("xb") as output:
            output.write(raw)
