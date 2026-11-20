#!/usr/bin/env python3
"""Add source-qualified compiled/terrain ray metadata to an existing world export."""

import argparse
from collections import Counter, defaultdict
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

TOOLS = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS.parent / 'collision'))
from archive import Psarc
from packfile import parse_packfile
from world import decode_collision_record
from zrb_records import parse as parse_resources

RETAIL_ELF_SHA256 = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
PROVENANCE = 'Retail compiled root tag with qualified stored triangle child filter'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def unique(rows, key):
    result = {}
    for row in rows:
        name = key(row)
        require(name not in result, f'Duplicate source identifier {name}')
        result[name] = row
    return result


def authored_vertical_bounds(clv):
    version, _, columns, _, count = struct.unpack_from('>IIIfI', clv)
    header_end = 20 + count * 20
    require(version == 31 and 0 < columns <= 64 and count == columns * columns
            and struct.unpack_from('>H', clv, header_end)[0] == 128,
            'Unsupported authored level bounds header')
    _, center, _, _, extent, _ = struct.unpack_from('>6f', clv, header_end + 130)
    require(math.isfinite(center) and math.isfinite(extent) and extent > 0.,
            'Invalid authored vertical bounds')
    # The original world ray uses separate single precision additions.
    return [struct.unpack('>f', struct.pack('>f', value))[0]
            for value in (center - extent, center + extent)]


def enrich_instances(instances, source, removed):
    emitted = unique(instances, lambda value: value['id'])
    require(set(emitted) <= set(source) and set(removed) <= set(source),
            'Unknown source mesh in assembled world')
    for name, (root, child, count) in source.items():
        omitted = removed.get(name, [])
        require(all(type(index) is int and 0 <= index < count for index in omitted)
                and len(omitted) == len(set(omitted)),
                f'Invalid degenerate source triangle inventory {name}')
        instance = emitted.get(name)
        if instance is None:
            require(len(omitted) == count, f'Unexplained source-mesh omission {name}')
            continue
        require(len(instance['triangles']) > 0 and len(instance['triangles']) + len(omitted) == count,
                f'Assembled/source triangle count mismatch {name}')
        require('collision' not in instance, f'Existing collision metadata {name}')
        instance['collision'] = {
            'filter_info': root,
            'ray_child_filter_info': child,
            'convex_radius': 0.,
            'child_filter_policy': 'uniform',
            'provenance': PROVENANCE,
        }


def archive_member(archive, name):
    return archive.extract(archive.names.index(name) + 1)


def source_packs(archive, proof, roots):
    wanted = {row['resource_token'] for row in roots.values()}
    resources = defaultdict(list)
    cache = {}
    for header in proof['resource_headers']:
        raw = archive_member(archive, header['member'])
        require(sha(raw) == header['sha256'], 'Changed original physics archive member')
        parsed = parse_resources(raw, retail13_compat=True)
        require(parsed['counts'] == header['native_counts']
                and parsed['counts']['unparsed_resources'] == 0
                and parsed['signature_consistency'] == {'undeclared_offsets': [], 'declared_missing_from_scan': []},
                'Unqualified original physics pack inventory')
        for resource in parsed['records']:
            token = resource['name_token']
            if token not in wanted:
                continue
            for pack in resource['packfiles']:
                digest = pack['sha256']
                features = cache.get(digest)
                if features is None:
                    offset = pack['offset']
                    data = raw[offset:offset + pack['bytes']]
                    require(sha(data) == digest, 'Changed original collision pack')
                    decoded = parse_packfile(data, include_static_ray=True)
                    require(not decoded['unsupported'] and not decoded['primitives'],
                            'Unsupported source ray shape class')
                    features = []
                    for mesh in decoded['meshes']:
                        ray = mesh['static_ray']
                        require(not ray['has_material_indices']
                                and mesh['collision_margin'] == 0.
                                and mesh['extrusion_xyz'] == [0., 0., 0.],
                                'Unsupported child ray filter, margin or extrusion')
                        require(type(ray['default_filter_info']) is int
                                and 0 <= ray['default_filter_info'] < 32,
                                'Unsupported child ray filter group')
                        features.append((ray['default_filter_info'], len(mesh['triangles'])))
                    require(features, 'Empty root ray geometry')
                    cache[digest] = features
                resources[token].append((resource['physics_resource_type'], pack, features))
    require(set(resources) == wanted, 'Missing original collision resource')
    return resources


def bound_source_meshes(archive, clv, root_proof):
    roots = unique(root_proof['root_inventory'], lambda row: row['root_id'])
    resources = source_packs(archive, root_proof, roots)
    source = {}
    for name, row in roots.items():
        token, ordinal = row['resource_token'], row['pack_index']
        kind, pack, children = resources[token][ordinal]
        tag = pack['collision_tag']
        require(0 <= tag < 32 and tag == row['raw_mopp_tag']
                and (tag or 9) == row['filter_info'] and row['initial_motion_type'] == 7
                and pack['parent_token'] == row['parent_token']
                and pack['sha256'] == row['pack_sha256'],
                f'Original root binding differs {name}')
        if row['kind'] == 'compiled':
            prefix, offset, suffix = name.split(':')
            require(prefix == 'compiled' and suffix == f'pack-{ordinal}' and kind == 0,
                    f'Invalid compiled root identity {name}')
            placement = decode_collision_record(clv, int(offset, 16))
            require(placement is not None and placement['token'] == token,
                    f'Original compiled collision reference differs {name}')
        else:
            require(row['kind'] == 'terrain' and kind == 1
                    and name == f'terrain:{token}:pack-{ordinal}',
                    f'Invalid terrain root identity {name}')
        for index, (child, count) in enumerate(children):
            source[f'{name}:mesh-{index}'] = (row['filter_info'], child, count)
    return source


def project(archive_path, elf_path, world_path, assembly_path, root_inventory_path):
    world_raw = world_path.read_bytes()
    world = json.loads(world_raw)
    assembly_raw = assembly_path.read_bytes()
    assembly = json.loads(assembly_raw)
    proof_raw = root_inventory_path.read_bytes()
    inventory = json.loads(proof_raw)
    map_name = world['metadata']['map_name']
    require(assembly['map'] == map_name and assembly['missing_resources'] == []
            and sha(world_raw) == assembly['world_sha256']
            and len(world['instances']) == assembly['instances'],
            'Assembled input binding differs')
    proof = unique(inventory['maps'], lambda row: row['map'])[map_name]
    elf_sha256 = sha(elf_path.read_bytes())
    require(elf_sha256 == RETAIL_ELF_SHA256 == proof['native_qualification']['original_retail_sha256'],
            'Changed original retail executable')
    archive_raw = archive_path.read_bytes()
    archive_sha256 = sha(archive_raw)
    require(archive_sha256 == proof['proposed_runtime_contract']['source_archive_sha256'],
            'Changed original retail level archive')
    archive = Psarc(archive_raw)
    clv = archive_member(archive, f'/run/mag/ps3/levels/{map_name}/{map_name}.clv')
    clv_sha256 = sha(clv)
    require(clv_sha256 == proof['original_clv_sha256'], 'Changed original level CLV')
    vertical_bounds = authored_vertical_bounds(clv)
    source = bound_source_meshes(archive, clv, proof)
    removed = {key: value['source_triangle_indices'] for key, value in
               unique(assembly['exact_zero_area_removals'], lambda row: row['instance_id']).items()}
    require(len(source) == proof['source_membership']['mesh_instances']
            + len(proof['source_membership']['source_meshes_omitted']), 'Source mesh inventory differs')
    instances = world['instances']
    enrich_instances(instances, source, removed)
    world['metadata']['provenance'].append(PROVENANCE)
    output = (json.dumps(world, separators=(',', ':'), allow_nan=False) + '\n').encode()
    filter_counts = Counter((root, child) for root, child, _ in source.values())
    receipt = {
        'schema_version': 1,
        'map_name': map_name,
        'source_archive_sha256': archive_sha256,
        'source_clv_sha256': clv_sha256,
        'source_elf_sha256': elf_sha256,
        'source_world_sha256': sha(world_raw),
        'source_assembly_sha256': sha(assembly_raw),
        'source_root_inventory_sha256': sha(proof_raw),
        'world_sha256': sha(output),
        'world_y_bounds': vertical_bounds,
        'root_count': len(proof['root_inventory']),
        'source_mesh_count': len(source),
        'output_mesh_count': len(instances),
        'reference_parent_root_count': sum(row['parent_token'] != '00000000' for row in proof['root_inventory']),
        'source_mesh_filter_counts': [
            {'root_filter_info': root, 'child_filter_info': child, 'mesh_count': count}
            for (root, child), count in sorted(filter_counts.items())
        ],
    }
    return output, (json.dumps(receipt, indent=2) + '\n').encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('archive', 'elf', 'world', 'assembly', 'root-inventory', 'output-dir'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        output = args.output_dir.resolve()
        require(not output.is_relative_to(TOOLS.parents[1]),
                'Outputs must be outside the source checkout')
        if not args.check:
            require(not output.exists(), 'Refusing an existing output directory')
        world, receipt = project(args.archive, args.elf, args.world, args.assembly, args.root_inventory)
        if args.check:
            require((output / 'world.json').read_bytes() == world
                    and (output / 'static-ray-receipt.json').read_bytes() == receipt,
                    'Static ray output differs')
        else:
            output.mkdir()
            (output / 'world.json').write_bytes(world)
            (output / 'static-ray-receipt.json').write_bytes(receipt)
    except (IndexError, KeyError, OSError, TypeError, ValueError, struct.error) as error:
        parser.error(str(error))


if __name__ == '__main__':
    main()
