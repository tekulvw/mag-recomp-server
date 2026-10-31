#!/usr/bin/env python3
"""Place existing retail neutral meshes using bound compiled collision records.

Static export by default. Parent attachments require an explicit, source-bound
reference-pose export; those outputs never certify a physics region.
"""
import argparse
import collections
import gzip
import json
import math
from pathlib import Path
import struct

from archive import Psarc
from world import decode_collision_record, bake_vertex, nondegenerate_triangles
from retail_terrain import ROOT, MAP, TERRAIN, module, require, sha, native_proof
from retail_placement import verify_placement


def static_region(lower, upper, provenance):
    require(len(lower) == len(upper) == 3 and all(math.isfinite(x) for x in lower + upper), 'nonfinite/invalid region')
    require(all(0 < upper[i] - lower[i] <= 128 for i in range(3)), 'invalid/unbounded region')
    return dict(min=lower, max=upper, ground_verified=True, obstacles_verified=True,
                provenance=['scope=initial_compiled_static_geometry',
                            'All present compiled map collision records resolve; existing terrain and static meshes included.',
                            'Dynamic/entity-created/moving collision is outside this capability; terrain holes remain query failures.'] + provenance)


def compiled_records(join, clv):
    records = []
    counts = collections.Counter()
    seen = set()
    for row in join['instances']:
        offset = int(row['instance_offset'], 16)
        require(offset not in seen and 12 <= offset < len(clv) - 12, 'duplicate/invalid instance offset')
        seen.add(offset)
        end = struct.unpack_from('>I', clv, offset + 4)[0]
        require(offset < end < len(clv) - 17 and sha(clv[offset-4:end]) == row['instance_sha256'], 'instance property source mismatch')
        require(struct.unpack_from('>I', clv, offset - 4)[0] == 7, 'unsupported property version')
        require(clv[end+8] == 1, 'unsupported embedded WorldGeometry presence')
        start = end + 9
        version, collision = struct.unpack_from('>II', clv, start)
        require(version == 0x10001 and start+8 < collision < len(clv), 'invalid compiled WorldGeometry boundary')
        placement = decode_collision_record(clv, collision)
        if placement is None:
            counts['compiled_absent'] += 1
            if row['collision_mesh_token'] != '00000000':
                counts['compiled_absent_with_authored_reference'] += 1
            continue
        require(placement['token'] == row['collision_mesh_token'], 'compiled/native instance resource token mismatch')
        counts['compiled_present'] += 1
        records.append(dict(name=row['name'], instance_properties_offset=offset,
                            world_geometry_offset=start,
                            collision_offset=collision, placement=placement))
    return records, dict(counts)


def load_meshes(pack, catalog, cache):
    entry = pack['neutral_output']
    key = entry['file']
    identity = (entry['sha256'], entry['decoded_sha256'], pack['source_sha256'])
    if key in cache:
        require(cache[key][0] == identity, 'conflicting cached neutral binding')
        return cache[key][1]
    path = (catalog / key).resolve()
    require(path.is_relative_to(catalog.resolve()), 'neutral path escapes catalog')
    compressed = path.read_bytes()
    require(sha(compressed) == entry['sha256'], 'neutral compressed hash mismatch')
    decoded = gzip.decompress(compressed)
    require(sha(decoded) == entry['decoded_sha256'], 'neutral decoded hash mismatch')
    value = json.loads(decoded)
    require(value['format'] == 'mag_collision_geometry' and value['schema_version'] == 1
            and value['packfile_sha256'] == pack['source_sha256'] and value['primitives'] == [], 'wrong neutral geometry schema/binding')
    for mesh in value['meshes']:
        require(mesh['coordinate_space'] == 'collision_shape_local' and mesh['extrusion_xyz'] == [0, 0, 0]
                and mesh['collision_margin'] == 0, 'unproved shape transform/extrusion/margin')
    cache[key] = (identity, value['meshes'])
    return value['meshes']


def export(args):
    require(not args.output.exists(), 'refusing existing output')
    map_name = getattr(args, 'map_name', None) or MAP
    terrain_token = getattr(args, 'terrain_token', None) or TERRAIN
    require(len(terrain_token) == 8 and all(c in '0123456789abcdef' for c in terrain_token),
            'terrain token must be eight lowercase hexadecimal digits')
    terrain_proof = native_proof(args.terrain_proof, args.retail_elf, map_name=map_name)
    placement_proof = verify_placement(args.placement_proof, args.retail_elf)
    manifest_bytes = args.map_manifest.read_bytes()
    manifest = json.loads(manifest_bytes)
    require(manifest['build'] == 'retail-bcus98110-01.00' and manifest['map'] == map_name, 'wrong retail map')
    join_bytes = args.instance_join.read_bytes()
    join = json.loads(gzip.decompress(join_bytes))
    require(join['build'] == manifest['build'] and join['map'] == map_name, 'wrong instance join')
    archive_bytes = args.archive.read_bytes()
    require(sha(archive_bytes) == join['source']['archive_sha256'] == manifest['source']['archive_sha256'], 'original map archive mismatch')
    archive = Psarc(archive_bytes)
    clv = archive.extract(archive.names.index(join['source']['clv_member']) + 1)
    require(sha(clv) == join['source']['clv_sha256'], 'compiled source CLV mismatch')
    import instance_linkage
    regenerated = instance_linkage.inspect(archive_bytes, {}, {}, manifest['build'], map_name, None)
    require(join['instances'] == regenerated['instances'], 'external instance join differs from complete source regeneration')
    records, counts = compiled_records(join, clv)
    native = module('compiled_native_zrb', Path(__file__).with_name('zrb_records.py'))
    packs = collections.defaultdict(list)
    for member in manifest['resource_members']:
        bound = [p for p in member['packfiles'] if p.get('resource_binding') and p.get('neutral_output')]
        if not bound:
            continue
        raw = archive.extract(archive.names.index(member['member']) + 1)
        require(sha(raw) == member['sha256'] and len(raw) == member['bytes'], 'retained resource member mismatch')
        parsed = native.parse(raw, retail13_compat=True)
        actual = {p['offset']: (r, p) for r in parsed['records'] for p in r.get('packfiles', [])}
        for pack in bound:
            binding = pack['resource_binding']
            require(binding['source_offset'] in actual, 'unowned collision pack')
            resource, source = actual[binding['source_offset']]
            require(binding['resource_name_token'] == resource['name_token'] and binding['physics_type_raw'] == resource['physics_resource_type'], 'native resource token/type mismatch')
            require(binding['physics_type_raw'] in (0, 1), 'unsupported resource type')
            for key, source_key in [('source_offset', 'offset'), ('source_bytes', 'bytes'), ('parent_token', 'parent_token'),
                                    ('collision_tag_raw', 'collision_tag'), ('material_indices', 'material_indices'),
                                    ('declared_packfile_sha256', 'sha256')]:
                require(binding[key] == source[source_key], 'native pack binding mismatch:' + key)
            require(pack['source_sha256'] == source['sha256'] and pack['unsupported_collision_objects'] == 0
                    and sha(raw[source['offset']:source['offset']+source['bytes']]) == source['sha256'], 'partial/unverified source pack')
            packs[binding['resource_name_token']].append(pack)
    require(terrain_token in packs and all(p['resource_binding']['physics_type_raw'] == 1 for p in packs[terrain_token]), 'terrain binding absent')
    parented = any(pack['resource_binding']['parent_token'] != '00000000'
                   for record in records for pack in packs.get(record['placement']['token'], []))
    resolver = None
    if parented:
        require(getattr(args, 'reference_pose', False) and getattr(args, 'parent_proof', None),
                'parented collision requires --reference-pose and --parent-proof; live animation is not reconstructed')
        require(args.physics_region_min is None and args.physics_region_max is None,
                'reference-pose attachments cannot certify a physics region')
        from retail_parents import ParentResolver, verify_native_proof
        resolver = ParentResolver(archive, clv, verify_native_proof(args.parent_proof, args.retail_elf))
    instances, bindings, missing, removed = [], [], [], []
    cache = {}
    def add(name, resource, placement, record=None):
        for ordinal, pack in enumerate(packs[resource]):
            parent_matrix = None
            if pack['resource_binding']['parent_token'] != '00000000':
                require(resolver is not None and record is not None,
                        'parented terrain or unbound parent is unsupported')
                parent_matrix = resolver.resolve(record, pack)
            for index, mesh in enumerate(load_meshes(pack, args.catalog, cache)):
                if parent_matrix is not None:
                    from retail_parents import bake_parent_vertex
                    vertices = [bake_parent_vertex(v, parent_matrix) for v in mesh['vertices_xyz']]
                else:
                    vertices = [bake_vertex(v, placement) for v in mesh['vertices_xyz']] if placement else mesh['vertices_xyz']
                identity = f'{name}:pack-{ordinal}:mesh-{index}'
                triangles, deleted = nondegenerate_triangles(vertices, mesh['triangles'])
                if deleted:
                    removed.append(dict(instance_id=identity, source_triangle_indices=deleted))
                if triangles:
                    instances.append(dict(id=identity, vertices_xyz=vertices, triangles=triangles))
    for record in records:
        placement = record['placement']
        token = placement['token']
        require(token != terrain_token, 'terrain unexpectedly embedded as placed collision')
        if token not in packs:
            missing.append(dict(collision_offset=record['collision_offset'], token=token))
            continue
        require(all(p['resource_binding']['physics_type_raw'] == 0 for p in packs[token]), 'compiled resource is not ordinary type0')
        add(f"compiled:{record['collision_offset']:x}", token, placement, record)
        bindings.append(dict(collision_offset=record['collision_offset'], resource_token=token,
                             record_sha256=placement['sha256'], matrix=placement['matrix'], scale_already_in_matrix=placement['scale']))
    add('terrain:' + terrain_token, terrain_token, None)
    require(not missing, 'unresolved compiled resources; cannot silently omit initial static obstacles')
    terrain_bytes = (json.dumps(terrain_proof, indent=2)+'\n').encode()
    placement_bytes = (json.dumps(placement_proof, indent=2)+'\n').encode()
    unknowns = ['scope=initial_compiled_static_geometry; later destructible/moving/disabled state is not synchronized.',
                'Material/filter semantics remain unvalidated; development queries treat these triangles as opaque.',
                'Other entity-created collision and global dynamic resources are outside this adapter.',
                'Live bot physics and obstacle alignment remain unaccepted.']
    scope = 'initial_compiled_static_geometry'
    if resolver is not None:
        scope = 'compiled_reference_pose'
        unknowns[0] = 'scope=compiled_reference_pose; parent attachments use frozen compiled poses, not current animation.'
        unknowns.append('Reference-pose output has no verified physics regions and cannot establish current collision authority.')
    provenance = [f'Original CLV SHA256 {sha(clv)}', f'Existing map catalog SHA256 {sha(manifest_bytes)}',
                  f'Existing instance join SHA256 {sha(join_bytes)}', f'Externally supplied terrain proof SHA256 {sha(terrain_bytes)}',
                  f'Externally supplied placement proof SHA256 {sha(placement_bytes)}',
                  'Final compiled collision matrices; no authored Euler/SnapToTerrain or second scale application.']
    regions = []
    if args.physics_region_min is not None or args.physics_region_max is not None:
        require(args.physics_region_min is not None and args.physics_region_max is not None, 'require both region bounds')
        regions.append(static_region(args.physics_region_min, args.physics_region_max, provenance))
    if resolver is not None:
        provenance.append('Parent tokens resolved through typed retail hierarchy nodes; child-to-root transforms applied once; native collision quaternion extraction modeled before compiled shape scale.')
    world = dict(format='mag_collision_world', schema_version=1, coordinate_space='world',
                 metadata=dict(map_name=map_name, build=manifest['build'], coverage='provisional',
                               physics_regions=regions, provenance=provenance, unknowns=unknowns), instances=instances)
    world_bytes = (json.dumps(world, separators=(',', ':'), allow_nan=False)+'\n').encode()
    require(len(world_bytes) <= 64*1024*1024, 'world exceeds existing loader default bytes')
    report = dict(schema='mag_retail_compiled_reuse_v1', map=map_name, counts=counts,
                  scope=scope, physics_regions=regions,
                  compiled_joined=len(bindings), instances=len(instances), triangles=sum(len(i['triangles']) for i in instances),
                  world_bytes=len(world_bytes), world_sha256=sha(world_bytes), missing_resources=missing,
                  bindings=bindings, exact_zero_area_removals=removed,
                  geometry_files={k: list(v[0]) for k, v in cache.items()}, unknowns=unknowns)
    if resolver is not None:
        report['parent_resolution'] = resolver.report()
    args.output.mkdir(parents=True)
    (args.output/'world.json').write_bytes(world_bytes)
    (args.output/'terrain-native-proof.json').write_bytes(terrain_bytes)
    (args.output/'placement-native-proof.json').write_bytes(placement_bytes)
    (args.output/'assembly.json').write_text(json.dumps(report, separators=(',', ':'))+'\n')
    print(json.dumps({k: report[k] for k in ['counts', 'compiled_joined', 'instances', 'triangles', 'world_bytes', 'world_sha256']}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', required=True, type=Path)
    parser.add_argument('--terrain-proof', required=True, type=Path)
    parser.add_argument('--placement-proof', required=True, type=Path)
    parser.add_argument('--retail-elf', required=True, type=Path)
    parser.add_argument('--map-manifest', required=True, type=Path)
    parser.add_argument('--instance-join', required=True, type=Path)
    parser.add_argument('--catalog', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--map-name', help='Retail map identity; defaults to the existing export target')
    parser.add_argument('--terrain-token', help='Source-bound type1 terrain token, eight lowercase hex digits')
    parser.add_argument('--reference-pose', action='store_true', help='Allow frozen compiled parent poses without verified physics regions')
    parser.add_argument('--parent-proof', type=Path, help='Externally reviewed, ELF-bound retail parent witness')
    parser.add_argument('--physics-region-min', type=lambda s: [float(v) for v in s.split(',')])
    parser.add_argument('--physics-region-max', type=lambda s: [float(v) for v in s.split(',')])
    export(parser.parse_args())


if __name__ == '__main__':
    main()
