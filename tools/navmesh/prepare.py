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
COVERAGE_FORMAT = 'mag_collision_coverage_v1'
COVERAGE_KEYS = {'format', 'map_name', 'world_sha256', 'basis', 'provenance', 'physics_regions'}
COVERAGE_BASES = {'operator_assertion', 'source_verified'}
REGION_KEYS = {'min', 'max', 'ground_verified', 'obstacles_verified', 'provenance'}
MAX_COVERAGE_BYTES = 64 * 1024
MAX_COVERAGE_REGIONS = 64
MAX_COVERAGE_COORDINATE = 100_000
TILE_WIDTH_M = 51.2
RUNTIME_TILES = 256
WIDE_RUNTIME_TILES = 512


def require(value, message):
    if not value:
        raise ValueError(message)


def point(value):
    return (isinstance(value, list) and len(value) == 3
            and all(type(v) in (float, int) and math.isfinite(v) and abs(v) <= 10_000_000 for v in value))


def bounds_valid(value):
    return (isinstance(value, dict) and point(value.get('min')) and point(value.get('max'))
            and all(value['min'][i] < value['max'][i] for i in range(3)))


def text_list(value):
    return isinstance(value, list) and value and all(isinstance(v, str) and v.strip() for v in value)


def coverage_region(value):
    return (isinstance(value, dict) and set(value) == REGION_KEYS and bounds_valid(value)
            and all(abs(v) <= MAX_COVERAGE_COORDINATE for k in ('min', 'max') for v in value[k])
            and type(value['ground_verified']) is bool and type(value['obstacles_verified']) is bool
            and text_list(value['provenance']))


def validate_coverage_sidecar(sidecar, world_sha256, map_name):
    """Check a mag_collision_coverage_v1 document and its binding to one world.

    Returns the sidecar's physics regions. The gameplay loader repeats every check.
    """
    require(isinstance(sidecar, dict) and set(sidecar) == COVERAGE_KEYS
            and sidecar['format'] == COVERAGE_FORMAT and sidecar['basis'] in COVERAGE_BASES
            and isinstance(sidecar['map_name'], str) and sidecar['map_name'].strip()
            and len(sidecar['map_name']) <= 128 and sha_value(sidecar['world_sha256'])
            and text_list(sidecar['provenance'])
            and isinstance(sidecar['physics_regions'], list)
            and 0 < len(sidecar['physics_regions']) <= MAX_COVERAGE_REGIONS
            and all(map(coverage_region, sidecar['physics_regions'])), 'invalid collision coverage sidecar')
    require(sidecar['world_sha256'] == world_sha256 and sidecar['map_name'] == map_name,
            'collision coverage world mismatch')
    return copy.deepcopy(sidecar['physics_regions'])


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def grid_tiles(bounds):
    """Tiles in the runtime's 51.2 m grid, in the native baker's f32 arithmetic."""
    width = f32(TILE_WIDTH_M)
    axis = [math.ceil(f32(f32(f32(bounds['max'][i]) - f32(bounds['min'][i])) / width)) for i in (0, 2)]
    return axis[0] * axis[1]


def find_filter_rows(document):
    """The 32 retail collision filter rows, given directly or under a 'filter_rows' key."""
    if isinstance(document, dict):
        if 'filter_rows' in document:
            return find_filter_rows(document['filter_rows'])
        for value in document.values():
            if isinstance(value, (dict, list)):
                found = find_filter_rows(value)
                if found is not None:
                    return found
        return None
    if (isinstance(document, list) and len(document) == 32
            and all(type(v) is int and 0 <= v < 2**32 for v in document)):
        return document
    if isinstance(document, list):
        for value in document:
            if isinstance(value, (dict, list)):
                found = find_filter_rows(value)
                if found is not None:
                    return found
    return None


def instance_layer(instance):
    """Collision layer (low five filter bits), or None for an instance without filter metadata."""
    collision = instance.get('collision')
    if not isinstance(collision, dict) or 'filter_info' not in collision:
        return None
    require(type(collision['filter_info']) is int and collision['filter_info'] >= 0, 'invalid filter_info')
    return collision['filter_info'] & 31


def prepare(world, world_sha256, bounds=None, *, reference=False, coverage=None, movement_layers=None):
    require(isinstance(world, dict) and world.get('format') == 'mag_collision_world'
            and type(world.get('schema_version')) is int and world['schema_version'] == 1
            and world.get('coordinate_space') == 'world', 'expected neutral world-space schema1')
    require(re.fullmatch('[0-9a-f]{64}', world_sha256) is not None, 'invalid world hash')
    metadata = world['metadata']
    require(isinstance(metadata.get('map_name'), str) and metadata['map_name'], 'missing map identity')
    regions = metadata.get('physics_regions', [])
    if coverage is not None:
        # The sidecar's regions join the world's own; the world bytes stay unchanged.
        require(not reference, 'reference preparation never claims coverage')
        regions = regions + validate_coverage_sidecar(coverage, world_sha256, metadata['map_name'])
    if reference:
        require(bounds is None, 'reference bounds derive from all source geometry')
    else:
        verified = [r for r in regions if r.get('ground_verified') is True and r.get('obstacles_verified') is True and bounds_valid(r)]
        require(verified, 'no verified static coverage region')
        bounds = bounds or {k: copy.deepcopy(verified[0][k]) for k in ('min', 'max')}
        require(bounds_valid(bounds) and any(all(bounds['min'][i] >= r['min'][i] and bounds['max'][i] <= r['max'][i]
                                               for i in range(3)) for r in verified), 'bake bounds exceed verified coverage')
    instances = world['instances']
    require(isinstance(instances, list) and 0 < len(instances) <= 100_000, 'invalid instance count')
    excluded = {}
    if movement_layers is not None:
        # Keep what character movement collides with; instances without filter
        # metadata stay, as the runtime keeps them solid too.
        require(not reference and type(movement_layers) is int and 0 <= movement_layers < 2**32,
                'invalid character movement layers')
        kept = []
        for instance in instances:
            layer = instance_layer(instance)
            if layer is None or movement_layers >> layer & 1:
                kept.append(instance)
            else:
                entry = excluded.setdefault(str(layer), dict(instances=0, triangles=0))
                entry['instances'] += 1
                entry['triangles'] += len(instance['triangles'])
        instances = kept
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
    geometry_bounds = dict(min=[min(v[i] for v in vertices) for i in range(3)],
                           max=[max(v[i] for v in vertices) for i in range(3)])
    if reference:
        bounds = copy.deepcopy(geometry_bounds)
        bounds['min'][1] -= 3.0
        bounds['max'][1] += 3.0
        require(bounds_valid(bounds), 'invalid full reference bounds')
    input_format = 'mag_navmesh_reference_bake_input_v1' if reference else 'mag_navmesh_bake_input_v1'
    value = dict(format=input_format, map_name=metadata['map_name'], world_sha256=world_sha256,
                 geometry_sha256=digest.hexdigest(), bounds=bounds, physics_regions=copy.deepcopy(regions),
                 vertices=vertices, triangles=triangles)
    if movement_layers is not None:
        value['character_movement_layers'] = movement_layers
    report = dict(instances=len(instances), vertices=len(vertices), triangles=len(triangles),
                  winding=winding, max_vertex_f32_rounding_m=max_rounding,
                  source_geometry_bounds=geometry_bounds,
                  geometry_hash_encoding='MAG-NAV-GEOMETRY-v1 NUL;LEu32 vertex_count,triangle_count;LEf32 XYZ;LEu32 triangle indices',
                  coordinate_policy='Native world Y-up; original winding, all faces and instance-local duplicates preserved',
                  metadata=copy.deepcopy(metadata))
    if movement_layers is not None:
        report['character_movement_layers'] = movement_layers
        report['excluded_by_layer'] = excluded
    if reference:
        report['scope'] = 'offline_reference_geometry_only'
        report['runtime_movement_authority'] = False
    else:
        tiles = grid_tiles(bounds)
        require(tiles <= WIDE_RUNTIME_TILES, 'bake bounds exceed the 512-tile runtime grid')
        report['runtime_grid_tiles'] = tiles
        report['runtime_layout'] = 'tiled_256' if tiles <= RUNTIME_TILES else 'wide_512'
        report['coverage_sidecar_regions'] = 0 if coverage is None else len(coverage['physics_regions'])
    return value, report



REGIONAL_COVERAGE = 'mag_bot_regional_walk_coverage_v1'
REGIONAL_POL = 'represented_fixed_triangles_walk'
REGIONAL_INPUT = 'mag_navmesh_regional_walk_input_v1'
EVIDENCE_KEYS = {'level_archive_sha256', 'level_clv_sha256', 'assembly_sha256',
                 'root_inventory_sha256', 'absence_sha256'}


def sha_value(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None


def prepare_regional(world, digest, coverage, coverage_sha256):
    require(isinstance(coverage, dict) and set(coverage) == {
        'schema', 'policy', 'map_name', 'world_sha256', 'geometry_sha256', 'bounds', 'source',
    } and coverage['schema'] == REGIONAL_COVERAGE and coverage['policy'] == REGIONAL_POL,
        'invalid regional walking coverage')
    require(coverage['world_sha256'] == digest
            and coverage['map_name'] == world['metadata']['map_name'], 'regional walking world mismatch')
    require(sha_value(coverage_sha256) and isinstance(coverage['source'], dict)
            and set(coverage['source']) == EVIDENCE_KEYS
            and all(map(sha_value, coverage['source'].values())), 'invalid regional walking evidence digests')
    bounds = coverage['bounds']
    require(bounds_valid(bounds) and set(bounds) == {'min', 'max'}
            and all(bounds['max'][i] - bounds['min'][i] <= 128 for i in range(3)),
            'invalid regional walking bounds')
    geometry, report = prepare(world, digest, reference=True)
    require(geometry['geometry_sha256'] == coverage['geometry_sha256'], 'regional walking geometry mismatch')
    geometry.update(format=REGIONAL_INPUT, bounds=copy.deepcopy(bounds), physics_regions=[])
    report.update(scope='regional_represented_fixed_triangles_walk', coverage_sha256=coverage_sha256)
    report.pop('runtime_movement_authority', None)
    return dict(schema=REGIONAL_INPUT, coverage_sha256=coverage_sha256, mesh=geometry), report

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--world', required=True, type=Path)
    parser.add_argument('--expected-world-sha256', required=True)
    parser.add_argument('--output', required=True, type=Path, help='New private output directory')
    scopes = parser.add_mutually_exclusive_group()
    scopes.add_argument('--regional-walk-coverage', type=Path)
    scopes.add_argument('--collision-coverage', type=Path,
                        help='mag_collision_coverage_v1 sidecar bound to this world; its regions join the world\'s')
    scopes.add_argument('--reference', action='store_true',
                        help='Prepare full geometry for offline reference baking without certifying movement')
    parser.add_argument('--filter-rows', type=Path,
                        help='JSON with the 32 retail collision filter rows (a list, or under "filter_rows")')
    parser.add_argument('--character-layer', type=int,
                        help='Layer whose filter row is character movement; drops static layers it passes through')
    args = parser.parse_args()
    require((args.filter_rows is None) == (args.character_layer is None),
            '--filter-rows and --character-layer go together')
    movement_layers = rows_sha256 = None
    if args.filter_rows is not None:
        require(args.filter_rows.is_file() and args.filter_rows.stat().st_size <= 64 * 1024 * 1024,
                'filter rows file size')
        raw_rows = args.filter_rows.read_bytes()
        rows = find_filter_rows(json.loads(raw_rows))
        require(rows is not None, 'no 32 filter rows found')
        require(0 <= args.character_layer < 32, 'character layer outside 0..31')
        movement_layers, rows_sha256 = rows[args.character_layer], hashlib.sha256(raw_rows).hexdigest()
        require(not args.regional_walk_coverage and not args.reference,
                'character movement layers apply to runtime meshes only')
    require(not args.output.exists(), 'refusing existing output directory')
    require(args.world.is_file() and args.world.stat().st_size <= 64*1024*1024, 'world file exceeds existing runtime limit')
    raw = args.world.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    require(digest == args.expected_world_sha256, 'world source hash mismatch')
    if args.regional_walk_coverage:
        path = args.regional_walk_coverage
        require(path.is_file() and path.stat().st_size <= 16 * 1024, 'regional walking coverage size')
        cap = path.read_bytes()
        value, report = prepare_regional(json.loads(raw), digest, json.loads(cap), hashlib.sha256(cap).hexdigest())
        mesh = value['mesh']
    else:
        coverage = coverage_sha256 = None
        if args.collision_coverage:
            path = args.collision_coverage
            require(path.is_file() and path.stat().st_size <= MAX_COVERAGE_BYTES, 'collision coverage size')
            cap = path.read_bytes()
            coverage, coverage_sha256 = json.loads(cap), hashlib.sha256(cap).hexdigest()
        value, report = prepare(json.loads(raw), digest, reference=args.reference, coverage=coverage,
                                movement_layers=movement_layers)
        if coverage is not None:
            report['coverage_sidecar_sha256'] = coverage_sha256
        if movement_layers is not None:
            report.update(filter_rows_sha256=rows_sha256, character_layer=args.character_layer)
        mesh = value
    encoded = (json.dumps(value, separators=(',', ':'), allow_nan=False)+'\n').encode()
    report.update(format='mag_navmesh_prep_manifest_v1', source_world=str(args.world.resolve()),
                  world_sha256=digest, geometry_sha256=mesh['geometry_sha256'],
                  input_sha256=hashlib.sha256(encoded).hexdigest(), input_bytes=len(encoded),
                  tool_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), bounds=mesh['bounds'])
    args.output.mkdir(parents=True)
    (args.output/'input.json').write_bytes(encoded)
    (args.output/'manifest.json').write_text(json.dumps(report, indent=2, allow_nan=False)+'\n')
    print(json.dumps({k: report[k] for k in ('world_sha256', 'geometry_sha256', 'input_sha256', 'vertices', 'triangles', 'max_vertex_f32_rounding_m')}))


if __name__ == '__main__':
    main()
