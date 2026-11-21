#!/usr/bin/env python3
"""Project operational hospital damage areas from original retail physics."""
import argparse
import json
from pathlib import Path
import runpy
import struct


HERE = Path(__file__).resolve().parent
GROUND = runpy.run_path(str(HERE / 'project-retail-hospital-rocket-ground-bounds.py'))
HOSPITAL = GROUND['HOSPITAL']
NATIVE = GROUND['NATIVE']
encode, sha, require = (GROUND[name] for name in ('encode', 'sha', 'require'))
FACTIONS = ('Valor', 'Raven', 'SVER')
WORDS = {
    0x108C154: 0x4849978D, 0x15258EC: 0x4B34B188,
    0x1A77370: 0x1B02F90, 0x1B02F90: 0x890838,
    0x1A773B8: 0x1B02F70, 0x1B02F70: 0x88E424,
    0x890898: 0x4BFE9FD1, 0x87A994: 0x812B00B8,
    0x87A9B0: 0x4E800421, 0x87A9F0: 0x80780010,
    0x87AA24: 0x812B0020, 0x87AA38: 0x4E800421,
    0x87AAA4: 0x485DCC3D, 0x87AAB0: 0x397913A0,
    0x87AAC4: 0x7C0059CE, 0x87AACC: 0x7C2A01CE,
    0x88E7DC: 0x485C8125,
}
BODY_WORDS = {
    0x88E6D0: 0x5520801E, 0x88E6D8: 0x60000016, 0x88E6E0: 0x900101A0,
    0x88E744: 0x38C00016, 0x88E748: 0x485C78C9,
}
SPANS = {
    'family13_hook': (0x890838, 0x8908B4),
    'family13_body_builder': (0x88E424, 0x88E810),
    'common_hook_and_cache': (0x87A868, 0x87AAD0),
    'debris_vehicle_area_call': (0x108C148, 0x108C16C),
    'absolute_area_thunk': (0x15258E0, 0x15258F0),
}


def operational_source(body, neutral):
    meshes = neutral['geometry']['meshes']
    require(len(meshes) == 1, 'expected one operational hospital mesh')
    mesh = meshes[0]
    require(body['layer'] == 22 and body['motion_type'] == 6
            and body['collision_tag'] == 9 and body['material_index'] == 8
            and body['packfile_sha256'] == neutral['packfile_sha256']
            and body['source_member_sha256'] == neutral['member_sha256']
            and mesh['subpart_index'] == 0 and mesh['extrusion_xyz'] == [0, 0, 0]
            and mesh['collision_margin'] == 0
            and all(body['body_local'].get(field) == mesh[field] for field in ('vertices_xyz', 'triangles')),
            'operational hospital collision source mismatch')
    return {'archive_member': neutral['member'], 'member_sha256': neutral['member_sha256'],
            'resource_token': neutral['physics_token'].removeprefix('0x'),
            'packfile_sha256': neutral['packfile_sha256']}


def verify_operational_mesh(archive, source, body, offset):
    from packfile import parse_packfile
    from zrb_records import parse

    raw = HOSPITAL['archive_member'](archive, source['archive_member'])
    record, = (row for row in parse(raw, retail13_compat=True)['records']
               if row['class_token'] == '2a8bdf5a' and row['name_token'] == source['resource_token'])
    pack, = record['packfiles']
    require(pack['offset'] == offset and pack['material_indices'] == [body['material_index']]
            and pack['collision_tag'] == body['collision_tag'],
            'original operational hospital material or pack changed')
    mesh, = parse_packfile(raw[offset:offset + pack['bytes']])['meshes']
    require(mesh['subpart_index'] == 0 and mesh['extrusion_xyz'] == [0, 0, 0]
            and mesh['collision_margin'] == 0
            and all(body['body_local'][field] == mesh[field] for field in ('vertices_xyz', 'triangles')),
            'operational hospital body differs from original pack')


def native_witness(elf):
    for address, expected in (WORDS | BODY_WORDS).items():
        actual, = struct.unpack('>I', elf.read(address, 4)[0])
        require(actual == expected, f'original hospital instruction changed: {address:#x}')
    return {
        name: {'start': hex(start), 'end_exclusive': hex(end),
               'sha256': sha(elf.read(start, end - start)[0])}
        for name, (start, end) in SPANS.items()
    } | GROUND['native_witness'](elf)


def project(args):
    elf = NATIVE['Elf'](args.elf, 'retail')
    main_raw = args.main_archive.read_bytes()
    collision_raw = args.hospital_collision.read_bytes()
    placement_raw = args.placement_inputs.read_bytes()
    collision, placements = map(json.loads, (collision_raw, placement_raw))
    elf_sha, main_sha = sha(args.elf.read_bytes()), sha(main_raw)
    require(elf_sha == HOSPITAL['RETAIL_ELF_SHA256'] == collision['retail_elf_sha256']
            == placements['retail_elf_sha256']
            and main_sha == HOSPITAL['MAIN_ARCHIVE_SHA256'] == collision['main_archive_sha256']
            == placements['retail_main_sha256']
            and collision['schema'] == 'mag_private_retail_hospital_hulks_v1'
            and collision['operational_source']['placement_sha256'] == sha(placement_raw),
            'hospital collision source or placement provenance mismatch')
    require(collision['factions'].keys() == {'0', '1', '2'}
            and placements['faction_sources'].keys() == set(FACTIONS),
            'incomplete hospital faction inventory')

    spans = native_witness(elf)
    archive = NATIVE['Psarc'](main_raw)
    rows, variants = {}, []
    for faction, name in enumerate(FACTIONS):
        body = collision['factions'][str(faction)]['operational']
        neutral = placements['faction_sources'][name]['neutral_body']
        source = operational_source(body, neutral)
        row = GROUND['read_chassis'](archive, source)
        verify_operational_mesh(archive, source, body, row['packfile_offset'])
        rear, front = GROUND['thresholds'](
            row['cached_shape_center_xyz'][2], row['cached_shape_half_extents_xyz'][2])
        row['thresholds'] = [rear, front]
        rows[name] = row
        variants.append({'faction': faction, 'packfile_sha256': row['packfile_sha256'],
                         'rear_threshold_z': rear, 'front_threshold_z': front})

    # Keep the existing operational bounds receipt reproducible.
    proof = (json.dumps({
        'retail_elf_sha256': elf_sha,
        'retail_main_sha256': main_sha,
        'placement_sha256': sha(placement_raw),
        'words': {hex(address): f'{value:08x}' for address, value in WORDS.items()},
        'spans': spans,
        'operational_hospitals': rows,
    }, indent=2, sort_keys=True, allow_nan=False) + '\n').encode()
    catalog = encode({
        'schema': 'mag_domination_hospital_blast_areas_v1',
        'source': {'retail_elf_sha256': elf_sha, 'retail_main_sha256': main_sha,
                   'hospital_collision_sha256': sha(collision_raw), 'bounds_proof_sha256': sha(proof)},
        'variants': variants,
    })
    return catalog, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('elf', 'main-archive', 'hospital-collision', 'placement-inputs', 'output', 'proof-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        HOSPITAL['output_paths'](
            [args.elf, args.main_archive, args.hospital_collision, args.placement_inputs],
            [args.output, args.proof_output],
        )
        for path, raw in zip((args.output, args.proof_output), project(args), strict=True):
            if args.check:
                require(path.read_bytes() == raw, f'hospital blast areas differ: {path}')
            else:
                with path.open('xb') as output:
                    output.write(raw)
    except (KeyError, TypeError, ValueError, OSError) as error:
        parser.error(str(error))


if __name__ == '__main__':
    main()
