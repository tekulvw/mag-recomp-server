#!/usr/bin/env python3
"""Bind a bounded movement-only sidecar to complete retained fixed geometry."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS / 'navmesh'))
sys.path.insert(0, str(TOOLS / 'collision'))
from archive import Psarc
from retail_parents import elf_span
from prepare import REGIONAL_COVERAGE, REGIONAL_POL, prepare, prepare_regional, require, sha_value


def project(world, world_sha, assembled, assembled_sha, assembly, assembly_sha,
            inventory, inventory_sha, absence, absence_sha, archive_sha, bounds, *,
            level_clv_sha, main_archive_sha, retail_elf_sha):
    map_name = world['metadata']['map_name']
    require(assembled['metadata']['map_name'] == map_name
            and world['metadata']['build'] == assembled['metadata']['build'],
            'assembled map or build mismatch')
    require(assembly['schema'] == 'mag_retail_compiled_reuse_v1'
            and assembly['scope'] == 'initial_compiled_static_geometry'
            and assembly['map'] == map_name and assembly['world_sha256'] == assembled_sha
            and not assembly['missing_resources'] and not assembly.get('parent_resolution'),
            'invalid fixed assembly')
    require(inventory['schema'] == 'mag_private_acquisition_source_root_exclusions_v2',
            'unsupported root inventory')
    rows = [row for row in inventory['maps'] if row['map'] == map_name]
    require(len(rows) == 1, 'missing or ambiguous source map inventory')
    row, = rows
    require(row['original_clv_sha256'] == level_clv_sha
            and row['native_qualification']['original_retail_sha256'] == retail_elf_sha
            and absence['source_elf_sha256'] == retail_elf_sha, 'retail ELF or CLV lineage mismatch')
    membership = row['source_membership']
    require({archive_sha, assembled_sha, assembly_sha}
            <= {entry['sha256'] for entry in row['source_inputs']}, 'source inventory input hash mismatch')
    require(membership['source_meshes_omitted'] == [], 'root inventory omits fixed geometry')

    # Compare exact source order, original winding and all source coordinates.
    geometry = lambda instance: (instance['id'], instance['vertices_xyz'], instance['triangles'])
    meshes, original = world['instances'], assembled['instances']
    require(len(meshes) == len(original) == assembly['instances'] == membership['mesh_instances']
            and all(geometry(a) == geometry(b) for a, b in zip(meshes, original))
            and sum(len(mesh['triangles']) for mesh in meshes) == assembly['triangles'],
            'fixed source geometry mismatch')
    roots = row['root_inventory']
    require(len(roots) == membership['source_roots']
            and len({r['root_id'] for r in roots}) == len(roots)
            and all(r['kind'] in ('terrain', 'compiled') and r['parent_token'] == '00000000'
                    and r['initial_motion_type'] == 7 for r in roots)
            and {m['id'].rsplit(':mesh-', 1)[0] for m in meshes} == {r['root_id'] for r in roots},
            'fixed root coverage mismatch')

    excluded = absence['excluded_exact_references']
    scans = absence['archives']
    spans = absence['native_ranges']
    no_body = row['native_missing_resource_no_roots']
    counts = assembly['counts']
    require(absence['schema'] == 'mag_retail_absent_collision_resource_v1'
            and excluded == no_body['excluded_exact_references']
            and scans == no_body['archives_reproduced'] and spans == no_body['native_ranges_verified']
            and len(scans) == 2 and all(x['token_matches'] == 0 and sha_value(x['sha256']) for x in scans)
            and archive_sha != main_archive_sha
            and {archive_sha, main_archive_sha} == {x['sha256'] for x in scans}
            and spans and all(s['bytes'] > 0 and sha_value(s['sha256']) for s in spans)
            and len(excluded) == counts['source_bound_absent_resource_references']
            and len({r['collision_offset'] for r in excluded}) == len(excluded)
            and not {r['collision_offset'] for r in excluded}
                    & {r['collision_offset'] for r in assembly['bindings']}
            and counts['compiled_present'] == membership['native_present_compiled_records']
            and assembly['compiled_joined'] == len(assembly['bindings'])
            == membership['compiled_objects_with_resolved_resources']
            == counts['compiled_present'] - len(excluded), 'fixed root absence proof mismatch')

    original_input, _ = prepare(world, world_sha, reference=True)
    value = dict(schema=REGIONAL_COVERAGE, policy=REGIONAL_POL, map_name=map_name,
                 world_sha256=world_sha, geometry_sha256=original_input['geometry_sha256'],
                 bounds=bounds, source=dict(level_archive_sha256=archive_sha,
                    level_clv_sha256=row['original_clv_sha256'], assembly_sha256=assembly_sha,
                    root_inventory_sha256=inventory_sha, absence_sha256=absence_sha))
    # Exercise the same bounds/schema validation as preparation.
    prepare_regional(world, world_sha, value, '0' * 64)
    return value


def read_json(path, limit):
    require(path.is_file() and path.stat().st_size <= limit, f'{path}: input exceeds limit')
    raw = path.read_bytes()
    return json.loads(raw), hashlib.sha256(raw).hexdigest()


def archive_sha(path):
    require(path.is_file() and 0 < path.stat().st_size <= 2 * 1024**3, 'invalid map archive size')
    digest = hashlib.sha256()
    with path.open('rb') as file:
        for block in iter(lambda: file.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    for flag in ('world', 'assembled-world', 'assembly', 'root-inventory', 'absence',
                 'level-archive', 'main-archive', 'retail-elf', 'output'):
        cli.add_argument('--' + flag, type=Path, required=True)
    cli.add_argument('--min', nargs=3, type=float, required=True)
    cli.add_argument('--max', nargs=3, type=float, required=True)
    cli.add_argument('--check', action='store_true')
    args = cli.parse_args()
    inputs = [args.world, args.assembled_world, args.assembly, args.root_inventory, args.absence]
    require(args.output.resolve() not in [p.resolve() for p in inputs + [args.level_archive, args.main_archive, args.retail_elf]],
            'output would overwrite an input')
    data = [item for path, limit in zip(inputs, (64, 64, 8, 32, 1))
            for item in read_json(path, limit * 1024**2)]
    require(args.level_archive.is_file() and 0 < args.level_archive.stat().st_size <= 512 * 1024**2,
            'invalid regional level archive size')
    level_raw = args.level_archive.read_bytes()
    level = Psarc(level_raw)
    expected_name = data[0]['metadata']['map_name'] + '.clv'
    members = [i for i, name in enumerate(level.names, 1) if Path(name).name == expected_name]
    require(len(members) == 1, 'missing or ambiguous original CLV')
    clv_sha = hashlib.sha256(level.extract(members[0])).hexdigest()
    require(args.retail_elf.is_file() and args.retail_elf.stat().st_size <= 64 * 1024**2,
            'retail ELF source exceeds size limit')
    elf = args.retail_elf.read_bytes()
    absence = data[8]
    spans = absence['native_ranges']
    require(0 < len(spans) <= 64, 'invalid native absence span count')
    for span in spans:
        raw = elf_span(elf, int(span['start'], 16), span['bytes'])
        require(hashlib.sha256(raw).hexdigest() == span['sha256'], 'native absence span mismatch')
    value = project(*data, hashlib.sha256(level_raw).hexdigest(), dict(min=args.min, max=args.max),
                    level_clv_sha=clv_sha, main_archive_sha=archive_sha(args.main_archive),
                    retail_elf_sha=hashlib.sha256(elf).hexdigest())
    encoded = (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()
    if args.check:
        require(args.output.read_bytes() == encoded, 'regional walking capability differs')
    else:
        with args.output.open('xb') as file:
            file.write(encoded)


if __name__ == '__main__':
    main()
