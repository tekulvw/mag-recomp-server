#!/usr/bin/env python3
"""Project original operational Ground chassis bounds for hospital rocket area selection."""
import argparse
import math
from pathlib import Path
import runpy
import struct
import sys
import zlib


HERE = Path(__file__).resolve().parent
HOSPITAL = runpy.run_path(str(HERE / 'project-retail-hospital-turrets.py'))
NATIVE = HOSPITAL['NATIVE']
sha, encode, require = (HOSPITAL[name] for name in ('sha', 'encode', 'require'))
RETAIL_MAIN_SHA256 = HOSPITAL['MAIN_ARCHIVE_SHA256']
ROLE = {'apc': 3, 'truck': 7}


def source_token(role):
    return zlib.crc32(role.encode('ascii'))


def f32(value):
    return struct.unpack('>f', struct.pack('>f', value))[0]


def thresholds(center_z, half_extent_z):
    require(math.isfinite(center_z) and abs(center_z) <= 1000
            and math.isfinite(half_extent_z) and 0 < half_extent_z <= 1000,
            'invalid original cached chassis bounds')
    inflated = f32(half_extent_z + f32(.01))
    lo, hi = f32(center_z - inflated), f32(center_z + inflated)
    model_center = f32(f32(lo + hi) * .5)
    model_radius = f32(f32(hi - lo) * .5)
    coefficient = f32(.33)
    return (f32(f32(model_center - model_radius) * coefficient),
            f32(f32(model_center + model_radius) * coefficient))


def select_variants(source_rows, bindings, *, roles=ROLE):
    tokens = {source_token(role): role for role in roles}
    source_by_template = {}
    for row in source_rows:
        if row['vehicle_type'] not in roles:
            continue
        template = Path(row['PropertiesFileName']).stem
        require(template and template not in source_by_template, 'duplicate Ground chassis source')
        source_by_template[template] = row
    result = {}
    projected = set()
    for binding in bindings:
        role = tokens.get(binding['vehicle_type_token'])
        if role is None:
            continue
        key = (roles[role], binding['faction'])
        template = binding['template']
        source = source_by_template.get(template)
        require(key not in result and template not in projected and source is not None
                and source['vehicle_type'] == role, 'duplicate or mismatched Ground chassis binding')
        projected.add(template)
        result[key] = (key[0], binding, source)
    require(set(result) == {(kind, faction) for kind in roles.values() for faction in range(3)}
            and set(source_by_template) == projected, 'incomplete Ground rocket chassis inventory')
    return [result[key] for key in sorted(result)]


def verify_source_projection(raw, dynamic):
    require(dynamic['source']['shape_data_sha256'] == sha(raw),
            'Ground catalog uses a different source projection')


def ground_chassis_entry(entries, expected_sha):
    selected = [entry for entry in entries if entry['collision_tag'] in (0, 9)]
    require(len(selected) == 1 and selected[0]['sha256'] == expected_sha,
            'unqualified original Ground chassis selection')
    return selected[0]


def read_chassis(archive, source, *, select_operational=False):
    sys.path.insert(0, str(HERE.parents[1] / 'tools/collision'))
    from packfile import parse_packfile
    from zrb_records import parse

    member = source['archive_member']
    raw = HOSPITAL['archive_member'](archive, member)
    require(sha(raw) == source['member_sha256'], f'Ground original member mismatch: {member}')
    rows = [record for record in parse(raw, retail13_compat=True)['records']
            if record['class_token'] == '2a8bdf5a'
            and record['name_token'] == source['resource_token']]
    require(len(rows) == 1 and rows[0]['physics_resource_type'] == 0,
            'expected one original Ground chassis resource')
    entries = rows[0]['packfiles']
    if select_operational:
        entry = ground_chassis_entry(entries, source['packfile_sha256'])
    else:
        require(len(entries) == 1, 'expected one original Ground chassis pack')
        entry = entries[0]
    offset = entry['offset']
    pack = raw[offset:offset + entry['bytes']]
    require(sha(pack) == entry['sha256'] == source['packfile_sha256'],
            'Ground original pack mismatch')
    mesh = parse_packfile(pack)
    classes = mesh['shape_class_coverage']
    require(not mesh['unsupported'] and not mesh['primitives']
            and classes.get('hkpMoppBvTreeShape') == 1
            and classes.get('hkpStorageExtendedMeshShape') == 1,
            'unqualified original Ground chassis shape')
    shape, = [row for row in mesh['source_objects'] if row['class'] == 'hkpStorageExtendedMeshShape']
    section, _, fixup_start, fixup_end, *_ = struct.unpack_from('>7I', pack, 64 + 48 * 2 + 20)
    fixups = [struct.unpack_from('>III', pack, section + i)
              for i in range(fixup_start, fixup_end - 11, 12)]
    require((0x34, 2, shape['offset']) in fixups, 'Ground MOPP shape join mismatch')
    address = section + shape['offset']
    half = struct.unpack_from('>3f', pack, address + 0x90)
    center = struct.unpack_from('>3f', pack, address + 0xa0)
    points = [vertex for part in mesh['meshes'] for vertex in part['vertices_xyz']]
    for axis in range(3):
        lo, hi = min(p[axis] for p in points), max(p[axis] for p in points)
        require(abs(center[axis] - f32((lo + hi) * .5)) < 1e-6
                and abs(half[axis] - f32((hi - lo) * .5)) < 1e-6,
                'original Ground cached bounds differ from observed shape')
    return {'member': member, 'member_sha256': sha(raw),
            'packfile_offset': offset, 'packfile_sha256': sha(pack),
            'cached_shape_center_xyz': center, 'cached_shape_half_extents_xyz': half}


def native_witness(elf):
    ranges = {'classifier': (0x870a74, 0x870ce8), 'model_bounds': (0x87a9c4, 0x87aad0),
              'mopp_get_aabb': (0x1330008, 0x133004c), 'extended_mesh_get_aabb': (0x1329fe8, 0x132a0a0)}
    require(elf.read(0x16778e0, 4)[0] == struct.pack('>f', .33)
            and elf.read(0x1b525b0, 4)[0] == struct.pack('>f', .01),
            'Ground bounds/classification source constants changed')
    return {name: {'start': hex(start), 'end_exclusive': hex(end),
                   'sha256': sha(elf.read(start, end - start)[0])}
            for name, (start, end) in ranges.items()}


def project(args):
    import json

    elf = NATIVE['Elf'](args.elf, 'retail')
    main_raw = args.main_archive.read_bytes()
    source_raw = args.vehicle_sources.read_bytes()
    dynamic_raw = args.dynamic_catalog.read_bytes()
    policy_raw = args.ground_damage_policy.read_bytes()
    sources = json.loads(source_raw)
    dynamic = json.loads(dynamic_raw)
    verify_source_projection(source_raw, dynamic)
    require(sha(main_raw) == sources['sources']['main_archive']['sha256'] == RETAIL_MAIN_SHA256,
            'Ground original main archive mismatch')
    archive = NATIVE['Psarc'](main_raw)
    policy_rows = json.loads(policy_raw)['profiles']
    source = {'retail_elf_sha256': HOSPITAL['RETAIL_ELF_SHA256'],
              'retail_main_sha256': sha(main_raw), 'dynamic_catalog_sha256': sha(dynamic_raw),
              'ground_damage_policy_sha256': sha(policy_raw)}
    proof = {'schema': 'mag_domination_hospital_rocket_ground_proof_v1', 'source': dict(source),
             'vehicle_sources_sha256': sha(source_raw), 'original_spans': native_witness(elf),
             'variants': []}
    variants = []
    for spawn_type, binding, source_vehicle in select_variants(
            sources['vehicles'], dynamic['runtime_bindings']['vehicles']):
        require(sum((row['vehicle_type_token'], row['faction'])
                    == (binding['vehicle_type_token'], binding['faction']) for row in policy_rows) == 1,
                'Ground damage policy does not bind the chassis variant')
        chassis = read_chassis(archive, source_vehicle['physics']['chassis'])
        rear, front = thresholds(chassis['cached_shape_center_xyz'][2],
                                 chassis['cached_shape_half_extents_xyz'][2])
        variant = {**binding, 'spawn_type': spawn_type, 'packfile_sha256': chassis['packfile_sha256'],
                   'rear_threshold_z': rear, 'front_threshold_z': front}
        variants.append(variant)
        proof['variants'].append({'binding': variant, 'chassis': chassis})
    proof_raw = encode(proof)
    return encode({'schema': 'mag_domination_hospital_rocket_ground_v1',
                   'source': {**source, 'bounds_proof_sha256': sha(proof_raw)},
                   'variants': variants}), proof_raw


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('elf', 'main-archive', 'vehicle-sources', 'dynamic-catalog', 'ground-damage-policy',
                 'output', 'proof-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    HOSPITAL['output_paths'](
        [args.elf, args.main_archive, args.vehicle_sources, args.dynamic_catalog, args.ground_damage_policy],
        [args.output, args.proof_output],
    )
    for path, raw in zip((args.output, args.proof_output), project(args), strict=True):
        if args.check:
            require(path.read_bytes() == raw, f'Ground rocket projection differs: {path}')
        else:
            with path.open('xb') as output:
                output.write(raw)


if __name__ == '__main__':
    main()
