#!/usr/bin/env python3
"""Reuse existing retail Cosmodrome neutral terrain with source-bound identity placement.

No mesh extraction, authored placement, native runtime or original-file writes.
This GroundOnly subset does not certify props/rocks or any horizontal domain.
"""
import argparse
import gzip
import hashlib
import importlib.util
import json
import math
import mmap
from pathlib import Path
import struct
from archive import Psarc

ROOT = Path(__file__).resolve().parents[2]
DEV_SHA = 'efea4d5ef0d14915c75382a56c23f5006fd6916b4080b8a386de01603358a09d'
RETAIL_SHA = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
MAP = 'mag_cosmodrome_final'
TERRAIN = 'e38ab022'
RESOURCE_SHA = '919552ee9cc79f43f71267e763340fce95e882a0abdfc343f4a3edeb56e6d3da'
PACK_SHAS = {'5559b92f110699afc038c0969fe3b49c1401c39a4383f7937a47601d8d36a4f1',
             '6e8abf37e28092b67bd8041052a6940285c18b55f06d265addea48c6266f7575'}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def require(value, message):
    if not value:
        raise ValueError(message)


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def branch_target(word, address):
    require(word >> 26 == 18 and word & 2 == 0, 'expected relative direct branch')
    displacement = word & 0x3fffffc
    if displacement & 0x2000000:
        displacement -= 0x4000000
    return address + displacement


def native_proof(proof_path, retail_path, *, map_name=MAP):
    """Validate identity of an external proof; do not claim to recreate that proof."""
    proof = json.loads(proof_path.read_text())
    require(proof.get('schema') == 'mag_retail_terrain_identity_v1', 'unsupported external terrain proof')
    require(proof.get('map') == map_name and proof.get('build') == 'retail-bcus98110-01.00', 'external proof map/build mismatch')
    require(sha(retail_path.read_bytes()) == RETAIL_SHA == proof.get('source_elf_sha256'), 'external proof ELF mismatch')
    require(proof.get('world_transform') == dict(translation_xyz=[0,0,0], rotation_xyzw=[0,0,0,1], scale_xyz=[1,1,1]), 'external proof is not identity placement')
    return proof


def assemble_ground(packs, catalog, provenance):
    instances = []
    removed = 0
    geometry_hashes = {}
    for ordinal, pack in enumerate(packs):
        binding = pack['resource_binding']
        require(binding['physics_type_raw'] == 1 and binding['resource_name_token'] == TERRAIN,
                'expected Cosmodrome type1 terrain')
        require(binding['parent_token'] == '00000000', 'terrain has an unproved parent')
        require(pack['unsupported_collision_objects'] == 0 and binding['declared_packfile_sha256'] == pack['source_sha256'], 'unsupported/unbound terrain geometry')
        entry = pack['neutral_output']
        path = (catalog / entry['file']).resolve()
        require(path.is_relative_to(catalog.resolve()), 'neutral path escapes catalog')
        compressed = path.read_bytes()
        require(sha(compressed) == entry['sha256'], 'neutral compressed hash mismatch')
        decoded = gzip.decompress(compressed)
        require(sha(decoded) == entry['decoded_sha256'], 'neutral decoded hash mismatch')
        geometry = json.loads(decoded)
        require(geometry['format'] == 'mag_collision_geometry' and geometry['schema_version'] == 1
                and geometry['packfile_sha256'] == pack['source_sha256'] and geometry['primitives'] == [], 'wrong neutral schema/binding')
        geometry_hashes[entry['file']] = dict(compressed_sha256=sha(compressed), decoded_sha256=sha(decoded))
        for index, mesh in enumerate(geometry['meshes']):
            require(mesh['coordinate_space'] == 'collision_shape_local' and mesh['extrusion_xyz'] == [0, 0, 0]
                    and mesh['collision_margin'] == 0, 'unproved coordinate space/extrusion/margin')
            vertices = mesh['vertices_xyz']
            require(vertices and all(len(v) == 3 and all(math.isfinite(x) for x in v) for v in vertices), 'invalid terrain vertices')
            triangles = []
            for face in mesh['triangles']:
                require(len(face) == 3 and all(isinstance(i, int) and 0 <= i < len(vertices) for i in face), 'invalid terrain triangle')
                a, b, c = [vertices[i] for i in face]
                u = [b[k] - a[k] for k in range(3)]
                v = [c[k] - a[k] for k in range(3)]
                cross = [u[1]*v[2]-u[2]*v[1], u[2]*v[0]-u[0]*v[2], u[0]*v[1]-u[1]*v[0]]
                if any(cross):
                    triangles.append(face)
                else:
                    removed += 1
            if triangles:
                instances.append(dict(id=f'terrain:{TERRAIN}:pack-{ordinal}:mesh-{index}', vertices_xyz=vertices, triangles=triangles))
    require(instances, 'empty terrain world')
    unknowns = ['GroundOnly subset: compiled props/rocks, buildings and movable collision are absent.',
                'No horizontal physics region is certified; physics_regions is empty.',
                'Terrain holes/outside triangle support remain unknown, not clear.',
                'Native live alignment and collision filter/material parity are not validated.']
    world = dict(format='mag_collision_world', schema_version=1, coordinate_space='world',
                 metadata=dict(map_name=MAP, build='retail-bcus98110-01.00', coverage='provisional',
                               physics_regions=[], provenance=provenance, unknowns=unknowns), instances=instances)
    report = dict(capability='GroundOnly', instances=len(instances),
                  triangles=sum(len(i['triangles']) for i in instances), exact_zero_area_removed=removed,
                  geometry_files=geometry_hashes, unknowns=unknowns)
    return world, report


def export(args):
    require(not args.output.exists(), 'refusing existing output directory')
    proof = native_proof(args.terrain_proof, args.retail_elf)
    manifest_bytes = args.map_manifest.read_bytes()
    manifest = json.loads(manifest_bytes)
    require(manifest['build'] == proof['build'] and manifest['map'] == MAP, 'wrong retail map manifest')
    members = [m for m in manifest['resource_members'] if m['sha256'] == RESOURCE_SHA]
    require(len(members) == 1, 'terrain resource member missing/duplicated')
    member = members[0]
    archive_bytes = args.archive.read_bytes()
    require(sha(archive_bytes) == manifest['source']['archive_sha256'], 'original map archive mismatch')
    archive = Psarc(archive_bytes)
    raw = archive.extract(archive.names.index(member['member']) + 1)
    require(sha(raw) == RESOURCE_SHA and len(raw) == member['bytes'], 'retained resource source mismatch')
    native = module('terrain_native_zrb', Path(__file__).with_name('zrb_records.py'))
    records = native.parse(raw, retail13_compat=True)['records']
    terrain = [r for r in records if r['name_token'] == TERRAIN]
    require(len(terrain) == 1 and terrain[0]['physics_resource_type'] == 1, 'wrong native terrain binding')
    native_packs = {p['sha256']: p for p in terrain[0]['packfiles']}
    require(set(native_packs) == PACK_SHAS, 'native terrain pack set changed')
    packs = [p for p in member['packfiles'] if p.get('resource_binding', {}).get('resource_name_token') == TERRAIN]
    require(len(packs) == 2 and {p['source_sha256'] for p in packs} == PACK_SHAS, 'catalog terrain set mismatch')
    for pack in packs:
        binding, actual = pack['resource_binding'], native_packs[pack['source_sha256']]
        for key, actual_key in [('source_offset', 'offset'), ('source_bytes', 'bytes'),
                                ('parent_token', 'parent_token'), ('collision_tag_raw', 'collision_tag'),
                                ('material_indices', 'material_indices')]:
            require(binding[key] == actual[actual_key], 'native/catalog binding mismatch:' + key)
        require(sha(raw[actual['offset']:actual['offset']+actual['bytes']]) == pack['source_sha256'], 'terrain source span mismatch')
    proof_bytes = (json.dumps(proof, indent=2) + '\n').encode()
    provenance = [f'Retail ELF SHA256 {RETAIL_SHA}', f'Map manifest SHA256 {sha(manifest_bytes)}',
                  f'Resource member SHA256 {RESOURCE_SHA}', f'Externally supplied native identity proof SHA256 {sha(proof_bytes)}',
                  'Existing neutral terrain vertices reused unchanged; no authored or guessed transforms.']
    world, report = assemble_ground(packs, args.catalog, provenance)
    world_bytes = (json.dumps(world, separators=(',', ':'), allow_nan=False) + '\n').encode()
    report.update(schema='mag_retail_terrain_reuse_v1', map=MAP, world_sha256=sha(world_bytes),
                  native_proof_sha256=sha(proof_bytes), map_manifest_sha256=sha(manifest_bytes),
                  source_resource_sha256=RESOURCE_SHA, source_packfiles=sorted(PACK_SHAS))
    args.output.mkdir(parents=True)
    (args.output / 'native-proof.json').write_bytes(proof_bytes)
    (args.output / 'world.json').write_bytes(world_bytes)
    (args.output / 'assembly.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ['capability', 'instances', 'triangles', 'exact_zero_area_removed', 'world_sha256']}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', required=True, type=Path)
    parser.add_argument('--terrain-proof', required=True, type=Path)
    parser.add_argument('--retail-elf', required=True, type=Path)
    parser.add_argument('--map-manifest', required=True, type=Path)
    parser.add_argument('--catalog', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    export(parser.parse_args())


if __name__ == '__main__':
    main()
