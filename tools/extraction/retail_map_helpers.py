#!/usr/bin/env python3
"""Retail exit helpers adapted from the original retail exit helper.

Original source SHA256: bb365d17c924b47b746c69a486f2eed1fefc291d9679b72c22f5f4613de4bb30
Import roots adapted; ordered accumulation retains legacy floating-point results.
Transform formulas and provisional-placement limits are unchanged.
No game-server runtime imports or original native resources.
"""
import argparse
import collections
import csv
import json
import math
from pathlib import Path
import re
import struct
import zlib

import runpy

# Self-contained helper: no external source checkout dependency.
_spawn = runpy.run_path(str(Path(__file__).with_name('extract-retail-spawns.py')))
evidence, retail_clv, sha = (_spawn[name] for name in ('evidence', 'retail_clv', 'sha'))

ROOT = Path(__file__).resolve().parents[2]
VEHICLES_MEMBER = '/run/mag/ps3/scripts/vehicles.zpr'
FACTIONS = {'Faction_Green': 'Valor', 'Faction_Red': 'SVER', 'Faction_Black': 'Raven'}
TRANSFORM_ASSUMPTION = ('Provisional column-vector Rz*Ry*Rx Euler rotation in degrees; '
                        'origin = authored translation + world-Y Placement_Y_Offset; '
                        'no terrain snapping. Retail runtime convention/placement not verified.')


def parse_zpr(text):
    """Parse the literal assignment/table subset, without evaluating Lua or comments."""
    pattern = re.compile(r'\s+|--[^\n]*|"(?:\\.|[^"\\])*"|[A-Za-z_]\w*|[{}=,]')
    tokens = []
    pos = 0
    while pos < len(text):
        m = pattern.match(text, pos)
        if not m:
            raise ValueError(f'Unsupported ZPR syntax at character {pos}')
        token = m[0]
        if not token.isspace() and not token.startswith('--'):
            tokens.append(token)
        pos = m.end()
    i = 0

    def table(nested=False):
        nonlocal i
        result = {}
        while i < len(tokens):
            if nested and tokens[i] == '}':
                i += 1
                return result
            key = tokens[i]
            if not re.fullmatch(r'[A-Za-z_]\w*', key) or key in result:
                raise ValueError(f'Invalid/duplicate ZPR key {key}')
            i += 1
            if i >= len(tokens) or tokens[i] != '=':
                raise ValueError(f'Missing assignment after {key}')
            i += 1
            if i >= len(tokens):
                raise ValueError(f'Missing value for {key}')
            token = tokens[i]
            i += 1
            if token == '{':
                value = table(True)
            elif token.startswith('"'):
                value = json.loads(token)
            else:
                raise ValueError(f'Nonliteral ZPR value for {key}')
            result[key] = value
            if i < len(tokens) and tokens[i] == ',':
                i += 1
            elif nested and i < len(tokens) and tokens[i] != '}':
                raise ValueError(f'Missing table separator after {key}')
        if nested:
            raise ValueError('Unclosed ZPR table')
        return result

    return table()


def finite(value):
    result = float(value)
    if not math.isfinite(result):
        raise ValueError('Nonfinite coordinate')
    return result


def vec3(value):
    values = value.split() if isinstance(value, str) else value
    if len(values) != 3:
        raise ValueError('Expected three vector components')
    return [finite(v) for v in values]


def dismount_properties(tree):
    props = tree['DefensiveEmplacementProperties']
    points = props['DisMountPoints']['DisMountPointPropertiesList']
    exits = []
    for key, point in points.items():
        if key != f'DisMountPointProperties{len(exits)}':
            raise ValueError('Dismount indices must be contiguous and in asset order')
        exits.append({'index': len(exits), 'property_path': f'DisMountPoints.DisMountPointPropertiesList.{key}.DisMountPointProperties.PositionOffset',
                      'local_xyz': vec3(point['DisMountPointProperties']['PositionOffset'])})
    if not exits:
        raise ValueError('Empty dismount list')
    return {'placement_y_offset': finite(props['Placement_Y_Offset']), 'exits': exits}


def vehicle_registry(tree):
    result = {}
    for key, wrapper in tree['VehicleTable']['Vehicle_Types'].items():
        data = wrapper['VehicleTypeData']
        name = data['Type']
        token = f'0x{zlib.crc32(name.lower().encode("ascii")):08x}'
        if token in result:
            raise ValueError('Ambiguous vehicle type token')
        variants = []
        for faction_key, faction in FACTIONS.items():
            for variant_key, variant in data.get(faction_key, {}).get('Vehicle_Data', {}).items():
                variants.append({'faction': faction, 'property_path': f'VehicleTable.Vehicle_Types.{key}.VehicleTypeData.{faction_key}.Vehicle_Data.{variant_key}.VehicleData', **variant['VehicleData']})
        result[token] = {'type': name, 'variants': variants}
    return result


def euler_basis(degrees):
    """Return matrix columns under the explicitly provisional Rz*Ry*Rx convention."""
    x, y, z = (math.radians(v) for v in vec3(degrees))
    sx, cx, sy, cy, sz, cz = math.sin(x), math.cos(x), math.sin(y), math.cos(y), math.sin(z), math.cos(z)
    return [[cz*cy, sz*cy, -sy],
            [cz*sy*sx-sz*cx, sz*sy*sx+cz*cx, cy*sx],
            [cz*sy*cx+sz*sx, sz*sy*cx-cz*sx, cy*cx]]


def euler_quaternion(degrees):
    x, y, z = (math.radians(v)/2 for v in vec3(degrees))
    sx, cx, sy, cy, sz, cz = math.sin(x), math.cos(x), math.sin(y), math.cos(y), math.sin(z), math.cos(z)
    return [sx*cy*cz-cx*sy*sz, cx*sy*cz+sx*cy*sz, cx*cy*sz-sx*sy*cz, cx*cy*cz+sx*sy*sz]


def objective_links(data):
    """Join version-7 objective definitions' ForwardSpawnPoint through real instances.

    Preserve enum bytes uninterpreted: these objectives contain enum formats the
    spawn-only decoder intentionally does not accept. No enum meaning is needed.
    """
    clv = retail_clv(data)
    definitions = {}
    marker = rb'\xde\xad\xbe\xef\x00\x00\x00\x03....(Objective[A-Za-z_0-9]*Properties)\x00'
    for match in re.finditer(marker, data):
        header = match.start()
        if header < 16 or clv.u32(header-16) != 7:
            continue
        classname, cursor = clv.string(header+8)
        end = clv.u32(header-8)
        fields = {}
        while cursor < end and data[cursor] == 1:
            name, cursor = clv.string(cursor+1)
            kind, cursor = clv.string(cursor)
            size = clv.u32(cursor)
            if name in fields or size < 4 or cursor+size > end:
                raise ValueError('Malformed objective field boundary or duplicate name')
            raw = data[cursor+4:cursor+size]
            fields[name] = {'type': kind, 'raw_hex': raw.hex(), 'offset': hex(cursor+4)}
            cursor += size
        if cursor+1 != end or data[cursor] != 0:
            raise ValueError('Malformed objective absolute record boundary')
        ref = fields.get('ForwardSpawnPoint')
        if not ref or ref['type'] != 'util::Token' or len(ref['raw_hex']) != 8 or ref['raw_hex'] == '00000000':
            continue
        token = clv.u32(header-4)
        if token in definitions:
            raise ValueError('Duplicate objective definition token')
        selected = ('ForwardSpawnPoint', 'NameTag', 'SubType', 'Category', 'OwnerTeam', 'Active', 'FSPType', 'DisplayName', 'DisplayID')
        definitions[token] = {'objective_class': classname, 'definition_token': f'0x{token:08x}',
                              'forward_spawn_token': '0x'+ref['raw_hex'],
                              'definition_evidence': {'offset': hex(header-12), 'end': hex(end), 'source_sha256': sha(data[header-16:end])},
                              'fields': {key: fields[key] for key in selected if key in fields}}
    result = collections.defaultdict(list)
    for inst in clv.tables('InstanceProperties'):
        definition = definitions.get(inst['preceding_token'])
        if definition:
            fields = {f['name']: f['value'] for f in inst['fields']}
            result[definition['forward_spawn_token']].append({**definition, 'objective_name_token': fields['Name'],
                                                             'instance_evidence': evidence(inst)})
    return dict(result)


def transform_candidate(origin, basis, offset):
    # Python3.12+ compensates sum(float); preserve the recorded legacy order.
    result = []
    for axis in range(3):
        total = 0.0
        for column in range(3):
            total += basis[column][axis] * offset[column]
        result.append(origin[axis] + total)
    return result


def catalog_point(point, registry, properties):
    vehicle = registry.get(point['vehicle_type_token'])
    if not vehicle or not any(v['Class'] == 'VehicleClass_DefensiveEmplacement' for v in vehicle['variants']):
        return None
    result = {'bunker_id': point['catalog_id'], 'spawn': point, 'vehicle_type': vehicle['type'],
              'base_position_xyz': point['position_xyz'], 'current_runtime_transform': None,
              'runtime_validated': False, 'transform_assumption': TRANSFORM_ASSUMPTION, 'candidates': []}
    variants = [v for v in vehicle['variants'] if v['faction'] == point['faction'] and v.get('ModifierLevel') == '0']
    if len(variants) != 1:
        result['coverage_gap'] = 'Unresolved faction or ambiguous vehicle modifier; no world candidates generated'
        return result
    variant = variants[0]
    if point['scale_xyz'] != [1, 1, 1]:
        result['coverage_gap'] = 'Nonunit authored scale needs runtime transform verification'
        return result
    props = properties[variant['PropertiesFileName']]
    origin = list(point['position_xyz'])
    origin[1] += props['placement_y_offset']
    basis = euler_basis(point['rotation_xyz'])
    result.update({'vehicle_variant': variant, 'properties': props, 'model_y_offset': props['placement_y_offset'],
                   'calculated_vehicle_origin_xyz': origin, 'provisional_basis_columns_xyz': basis,
                   'provisional_basis_rows_xyz': [list(row) for row in zip(*basis)],
                   'provisional_quaternion_xyzw': euler_quaternion(point['rotation_xyz']),
                   'coverage_gap': 'Runtime current transform, terrain snapping, floor ray, clearance and candidate selection unknown'})
    for exit in props['exits']:
        world = transform_candidate(origin, basis, exit['local_xyz'])
        result['candidates'].append({'index': exit['index'], 'local_offset_xyz': exit['local_xyz'],
                                    'provisional_world_xyz': world, 'world_ungrounded_xyz': world,
                                    'floor_hit_y': None, 'final_grounded_xyz': None, 'validated_feet_xyz': None,
                                    'character_clearance': None, 'selected_at_runtime': None})
    return result
