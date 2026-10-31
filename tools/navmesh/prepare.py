#!/usr/bin/env python3
"""Prepare private neutral world geometry for the pinned Recast bake CLI.

Preserves all faces and winding, including nonwalkable obstacles/undersides.
No welding, clipping, coordinate conversion, extraction or native execution.
"""
import argparse
from array import array
import copy
import hashlib
import json
import math
from pathlib import Path
import re
import struct
import sys

GEOMETRY_DOMAIN = b'MAG-NAV-GEOMETRY-v1\0'


def require(value, message):
    if not value:
        raise ValueError(message)


def point(value):
    return (isinstance(value, list) and len(value) == 3
            and all(type(v) in (float, int) and math.isfinite(v) and abs(v) <= 10_000_000 for v in value))


def bounds_valid(value):
    return (isinstance(value, dict) and point(value.get('min')) and point(value.get('max'))
            and all(value['min'][i] < value['max'][i] for i in range(3)))


def prepare(world, world_sha256, bounds=None):
    require(isinstance(world, dict) and world.get('format') == 'mag_collision_world'
            and type(world.get('schema_version')) is int and world['schema_version'] == 1
            and world.get('coordinate_space') == 'world', 'expected neutral world-space schema1')
    require(re.fullmatch('[0-9a-f]{64}', world_sha256) is not None, 'invalid world hash')
    metadata = world['metadata']
    require(isinstance(metadata.get('map_name'), str) and metadata['map_name'], 'missing map identity')
    regions = metadata.get('physics_regions', [])
    verified = [r for r in regions if r.get('ground_verified') is True and r.get('obstacles_verified') is True and bounds_valid(r)]
    require(verified, 'no verified static coverage region')
    bounds = bounds or {k: copy.deepcopy(verified[0][k]) for k in ('min', 'max')}
    require(bounds_valid(bounds) and any(all(bounds['min'][i] >= r['min'][i] and bounds['max'][i] <= r['max'][i]
                                           for i in range(3)) for r in verified), 'bake bounds exceed verified coverage')
    instances = world['instances']
    require(isinstance(instances, list) and 0 < len(instances) <= 100_000, 'invalid instance count')
    vertices, triangles, names = [], [], set()
    winding = {k: dict(up_45=0, down_45=0, steep=0) for k in ('terrain', 'compiled')}
    max_rounding = 0.0
    for instance in instances:
        name = instance['id']
        require(isinstance(name, str) and name and name not in names, 'duplicate/invalid instance ID')
        names.add(name)
        local = instance['vertices_xyz']
        require(isinstance(local, list) and local, 'empty vertices')
        offset = len(vertices)
        for vertex in local:
            require(point(vertex), 'invalid/nonfinite/out-of-range vertex')
            rounded = list(struct.unpack('<3f', struct.pack('<3f', *vertex)))
            max_rounding = max(max_rounding, *(abs(a-b) for a, b in zip(vertex, rounded)))
            vertices.append(rounded)
        for face in instance['triangles']:
            require(isinstance(face, list) and len(face) == 3
                    and all(type(i) is int and 0 <= i < len(local) for i in face), 'invalid triangle index')
            a, b, c = [vertices[offset+i] for i in face]
            u, v = [[right[i]-a[i] for i in range(3)] for right in (b, c)]
            normal = [u[1]*v[2]-u[2]*v[1], u[2]*v[0]-u[0]*v[2], u[0]*v[1]-u[1]*v[0]]
            length = math.sqrt(sum(x*x for x in normal))
            require(math.isfinite(length) and length > 0, 'zero-area/nonfinite triangle after f32 conversion')
            label = 'up_45' if normal[1]/length >= math.sqrt(.5) else 'down_45' if normal[1]/length <= -math.sqrt(.5) else 'steep'
            winding['terrain' if name.startswith('terrain:') else 'compiled'][label] += 1
            triangles.append([offset+i for i in face])
        require(len(vertices) <= 2_000_000 and len(triangles) <= 2_000_000, 'geometry exceeds existing runtime limits')
    require(triangles, 'empty triangle world')
    xyz = array('f', (v for vertex in vertices for v in vertex))
    indices = array('I', (v for triangle in triangles for v in triangle))
    require(xyz.itemsize == indices.itemsize == 4, 'unsupported host array widths')
    if sys.byteorder != 'little':
        xyz.byteswap(); indices.byteswap()
    digest = hashlib.sha256(GEOMETRY_DOMAIN + struct.pack('<II', len(vertices), len(triangles)))
    digest.update(xyz.tobytes()); digest.update(indices.tobytes())
    value = dict(format='mag_navmesh_bake_input_v1', map_name=metadata['map_name'], world_sha256=world_sha256,
                 geometry_sha256=digest.hexdigest(), bounds=bounds, physics_regions=copy.deepcopy(regions),
                 vertices=vertices, triangles=triangles)
    report = dict(instances=len(instances), vertices=len(vertices), triangles=len(triangles),
                  winding=winding, max_vertex_f32_rounding_m=max_rounding,
                  source_geometry_bounds=dict(min=[min(v[i] for v in vertices) for i in range(3)],
                                              max=[max(v[i] for v in vertices) for i in range(3)]),
                  geometry_hash_encoding='MAG-NAV-GEOMETRY-v1 NUL;LEu32 vertex_count,triangle_count;LEf32 XYZ;LEu32 triangle indices',
                  coordinate_policy='Native world Y-up; original winding, all faces and instance-local duplicates preserved',
                  metadata=copy.deepcopy(metadata))
    return value, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--world', required=True, type=Path)
    parser.add_argument('--expected-world-sha256', required=True)
    parser.add_argument('--output', required=True, type=Path, help='New private output directory')
    args = parser.parse_args()
    require(not args.output.exists(), 'refusing existing output directory')
    require(args.world.is_file() and args.world.stat().st_size <= 64*1024*1024, 'world file exceeds existing runtime limit')
    raw = args.world.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    require(digest == args.expected_world_sha256, 'world source hash mismatch')
    value, report = prepare(json.loads(raw), digest)
    encoded = (json.dumps(value, separators=(',', ':'), allow_nan=False)+'\n').encode()
    report.update(format='mag_navmesh_prep_manifest_v1', source_world=str(args.world.resolve()),
                  world_sha256=digest, geometry_sha256=value['geometry_sha256'],
                  input_sha256=hashlib.sha256(encoded).hexdigest(), input_bytes=len(encoded),
                  tool_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), bounds=value['bounds'])
    args.output.mkdir(parents=True)
    (args.output/'input.json').write_bytes(encoded)
    (args.output/'manifest.json').write_text(json.dumps(report, indent=2, allow_nan=False)+'\n')
    print(json.dumps({k: report[k] for k in ('world_sha256', 'geometry_sha256', 'input_sha256', 'vertices', 'triangles', 'max_vertex_f32_rounding_m')}))


if __name__ == '__main__':
    main()
