#!/usr/bin/env python3
"""Project retail leadership ability map bindings from caller-supplied inputs."""

import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import runpy
import struct
import sys

TOOLS = Path(__file__).resolve().parent
RETAIL_SHA256 = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
TERRAIN_CLASS = '284c2f78'
TERRAIN_INSTANCE = 0x82BA3FF2
STRUCTURAL = 'ObjectiveDemolitionStructuralProperties'
LINKED_CLASSES = {STRUCTURAL, 'ObjectiveSecureAndHoldProperties', 'ObjectiveArrivalProperties'}
PHANTOM_TRIGGER_TYPES = {4, 5}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def json_bytes(value):
    return (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()


def unique(rows, key, name):
    result = {key(row): row for row in rows}
    require(len(result) == len(rows), f'Duplicate {name}')
    return result


def catalog_indices(catalog):
    rows = catalog['objects']
    indices = {int(row['token'], 16): row['object_index'] for row in rows}
    require(len(indices) == len(rows), 'Objective catalog has duplicate token')
    slots = set(indices.values())
    require(len(slots) == len(rows), 'Objective catalog has duplicate index')
    require(all(type(index) is int and 0 <= index < 128 for index in slots),
            'Objective index outside retail type-12 pool')
    return indices


def roster_tokens(value):
    raw = bytes.fromhex(value)
    require(len(raw) >= 8, 'Invalid platoon objective vector length')
    version, count = struct.unpack_from('>II', raw)
    require(version == 1 and count <= 128 and len(raw) == 8 + count * 4,
            'Unexpected platoon objective vector version or length')
    tokens = struct.unpack_from(f'>{count}I', raw, 8)
    require(len(tokens) == len(set(tokens)), 'Duplicate platoon objective token')
    return tokens


def platoon_membership(clv, mode, spawn, properties):
    parsed = spawn['retail_clv'](clv)
    properties['register'](parsed, 'PlatoonProperties')
    indices = set()
    membership = {}
    for table in parsed.tables('PlatoonProperties'):
        fields = unique(table['fields'], lambda row: row['name'], 'platoon property')
        team, local = (fields[key]['value'] for key in ('Team', 'ID'))
        require(team in (0, 1) and local in range(4), 'Invalid retail platoon identity')
        index = 4 * team + local
        require(index not in indices, 'Duplicate retail platoon index')
        indices.add(index)
        for token in roster_tokens(fields['ObjectiveLinks']['raw_hex']):
            membership.setdefault(token, []).append(index)
    expected = {0, 1, 4, 5} if mode == 1 else set(range(8))
    require(indices == expected, 'Incomplete authored platoon roster')
    return membership


def f32(value):
    return struct.unpack('>f', struct.pack('>f', value))[0]


def bounds(clv, terrain):
    require(len(clv) >= 20, 'Truncated compiled level')
    version, _, columns, _, count = struct.unpack_from('>IIIfI', clv)
    header = 20 + count * 20
    require(version == 31 and 0 < columns <= 64 and count == columns * columns
            and header + 158 <= len(clv) and struct.unpack_from('>H', clv, header)[0] == 128,
            'Unsupported compiled level bounds header')
    box = struct.unpack_from('>6f', clv, header + 130)
    require(struct.unpack_from('>I', clv, header + 154)[0] == 0,
            'Unsupported explicit level boundary polygon')
    require(tuple(terrain) == box, 'Compiled terrain AABB differs from level header')
    require(all(map(math.isfinite, box)) and all(value > 0 for value in box[3:]),
            'Invalid compiled terrain AABB')
    lo = [f32(center - half) for center, half in zip(box[:3], box[3:])]
    hi = [f32(center + half) for center, half in zip(box[:3], box[3:])]
    return dict(min_x=lo[0], min_z=lo[2], max_x=hi[0], max_z=hi[2]), [lo[1], hi[1]]


def terrain_extent(clv, records):
    terrain = [row for row in records if row['class_token'] == TERRAIN_CLASS]
    require(len(terrain) == 1 and terrain[0]['resource_version'] == 63,
            'Expected one retail GeometryBDAM v63 resource')
    row = terrain[0]
    token = int(row['name_token'], 16)
    match = struct.pack('>II', TERRAIN_INSTANCE, token)
    offset = clv.find(match)
    require(4 <= offset <= len(clv) - 20 and clv.find(match, offset + 1) == -1
            and struct.unpack_from('>I', clv, offset - 4)[0] == offset + 20
            and struct.unpack_from('>3I', clv, offset + 8) == (0, 0xFFFFFFFF, 5),
            'Unsupported or ambiguous compiled terrain instance')
    # GeometryBDAMInst returns identity for GetWorldTransform on this retail image.
    raw = bytes.fromhex(row['unparsed_resource_header_hex'])
    require(len(raw) >= 44, 'Truncated compiled terrain AABB')
    return struct.unpack_from('>6f', raw, 20)


def acquisition_inputs(receipt, receipt_sha, catalog, indices):
    require(receipt['original_elf_sha256']['retail'] == RETAIL_SHA256
            and receipt_sha == catalog['source']['binding_capture_sha256'],
            'Acquisition Objective identity source differs from catalog')
    level = unique(receipt['maps'], lambda row: row['map']['map'], 'source map')[catalog['map_name']]
    rows = unique(level['objectives'], lambda row: int(row['name_token'], 16), 'Objective identity')
    expected = {}
    for token, row in rows.items():
        binding = row['native_initial_client_binding']
        require(binding['network_type'] == 12 and binding['client_pool_slot'] == binding['network_index'],
                'Unqualified Acquisition Objective identity')
        expected[token] = binding['network_index']
    require(expected == indices, 'Acquisition Objective token/index join differs from catalog')
    return level['source'], rows


def domination_inputs(poses, catalog, indices):
    source = catalog['source']
    require(poses['retail_elf_sha256'] == RETAIL_SHA256 and poses['map'] == catalog['map_name']
            and poses['metadata_sha256'] == source['authored_catalog_sha256']
            and poses['client_source_bindings_sha256'] == source['binding_capture_sha256'],
            'Domination Objective pose source differs from catalog')
    rows = unique(poses['objective_poses'], lambda row: row['token'], 'Objective pose')
    require({token: row['index'] for token, row in rows.items()} == indices,
            'Domination Objective pose token/index join differs from catalog')
    return {'archive_sha256': poses['level_archive_sha256'],
            'clv_sha256': poses['level_clv_sha256']}, rows


def enum(fields, key):
    raw = bytes.fromhex(fields[key]['raw_hex'])
    require(len(raw) == 13 and raw[:9] == bytes.fromhex('deadbeef0000000100'),
            f'Unsupported Objective enum encoding for {key}')
    return struct.unpack_from('>I', raw, 9)[0]


def damage_trigger_rows(graph, volumes, indices, vector_slots):
    definitions = unique(graph['definitions'], lambda row: row['definition_token'], 'Objective definition')
    volumes = unique(volumes, lambda row: int(row['trigger_token'], 16), 'trigger volume')
    output = []
    used_slots = set()
    for instance in graph['instances']:
        definition = definitions[instance['definition_token']]
        if definition['class_name'] != 'ObjectiveTriggerProperties':
            continue
        fields = definition['fields']
        trigger_type = enum(fields, 'TriggerType')
        # The server LevelLoaded path only gives these property types a Havok phantom.
        if trigger_type not in PHANTOM_TRIGGER_TYPES:
            continue
        token = int(instance['name_token'], 16)
        require(token in vector_slots, 'Damage trigger has no qualified Objective vector index')
        require(token in volumes, 'Damage trigger has no supported linked volume')
        object_index, trigger_index = vector_slots[token]
        volume = volumes[token]
        owner_token = int(volume['objective_token'], 16)
        require(indices.get(owner_token) == object_index and type(trigger_index) is int
                and 0 <= trigger_index < 255 and (object_index, trigger_index) not in used_slots,
                'Damage trigger objective slot or trigger index differs from qualified binding')
        used_slots.add((object_index, trigger_index))
        active = fields['StartActive']['value']
        threshold = fields['DamageThreshold']['value']
        require(type(active) is bool and type(threshold) in (int, float) and math.isfinite(threshold),
                'Invalid damage trigger activity or threshold')
        box = {key: volume['box'][key] for key in ('centre', 'axes', 'half_extents')}
        # The original trigger position getter reads the linked box centre.
        output.append({
            'object_index': object_index, 'trigger_index': trigger_index, 'trigger_type': trigger_type,
            'start_active': active, 'damage_threshold': f32(threshold),
            'position': box['centre'], 'box': box,
        })
    return sorted(output, key=lambda row: (row['object_index'], row['trigger_index']))


def domination_trigger_slots(proof, catalog, poses, source, volumes):
    require(proof['schema'] == 'mag_domination_initial_trigger_bindings_v1'
            and proof['status'] == 'ORIGINAL_RETAIL_CLIENT_AND_SIM_INITIAL_TRIGGER_VECTOR_ORDER_BOUND'
            and proof['original_elf_sha256']['retail'] == RETAIL_SHA256
            and proof['authored_metadata_sha256'] == poses['metadata_sha256']
            and proof['objective_binding_sha256'] == poses['client_source_bindings_sha256'],
            'Unqualified Domination trigger-vector evidence')
    maps = unique(proof['maps'], lambda row: row['map']['map'], 'trigger source map')
    level = maps[catalog['map_name']]
    require(all(level['source'][key] == value for key, value in source.items()),
            'Trigger-vector evidence does not match the level')
    bindings = unique(level['triggers'], lambda row: int(row['trigger_token'], 16), 'trigger-vector binding')
    positions = unique(poses['trigger_positions'],
                       lambda row: (row['objective_index'], row['trigger_index']), 'trigger position')
    require(len(bindings) == len(positions) == len(volumes), 'Incomplete trigger-vector inventory')
    slots = {}
    for volume in volumes:
        token = int(volume['trigger_token'], 16)
        row = bindings[token]
        slot = (row['objective_initial_client_network_index'], row['sim_trigger_index'])
        require(row['authored_volume'] == volume and row['client_trigger_index'] == slot[1]
                and slot in positions and positions[slot]['position'] == volume['box']['centre'],
                'Trigger binding differs from linked native volume or position')
        slots[token] = slot
    require(set(slots.values()) == positions.keys(), 'Incomplete trigger-vector slot coverage')
    return slots


def precision_target_rows(graph, indices):
    """Project the native type-1 and nonzero-category receiver guard."""
    instances = unique(graph['instances'], lambda row: int(row['name_token'], 16), 'Objective instance')
    definitions = unique(graph['definitions'], lambda row: row['definition_token'], 'Objective definition')
    targets = []
    for token, index in indices.items():
        definition = definitions[instances[token]['definition_token']]
        if definition['class_name'] != STRUCTURAL:
            continue
        fields = definition['fields']
        if enum(fields, 'Category') == 0:
            continue
        owner = fields['OwnerTeam']['value']
        require(owner in (0, 1), 'Unsupported Precision Strike Objective owner team')
        targets.append({'object_index': index, 'team': owner})
    return sorted(targets, key=lambda row: row['object_index'])


def objective_rows(graph, indices, source_rows, membership, mode):
    instances = unique(graph['instances'], lambda row: int(row['name_token'], 16), 'Objective instance')
    definitions = unique(graph['definitions'], lambda row: row['definition_token'], 'Objective definition')
    assets = []
    objectives = []
    roots = []
    require(membership.keys() <= indices.keys(), 'A platoon links an unqualified Objective')
    for token in indices:
        instance = instances[token]
        definition = definitions[instance['definition_token']]
        fields = definition['fields']
        subtype = enum(fields, 'SubType')
        if token not in membership and subtype != 13:
            continue
        require(definition['class_name'] in LINKED_CLASSES, 'Unsupported Objective class at bound slot')
        axes = unique(instance['fields'], lambda row: row['name'], 'Objective instance property')
        coordinates = [axes[f'Translate{axis}'] for axis in 'XYZ']
        position = [field['value'] for field in coordinates]
        require(all(map(math.isfinite, position)), 'Invalid Objective position')
        index = indices[token]
        source = source_rows[token]
        if mode == 1:
            require(instance['definition_token'] == source['definition_token']
                    and instance['source'] == source['instance_source'],
                    'Acquisition Objective instance identity or source range differs')
            if token in membership:
                roots.append({
                    'object_index': index, 'token': f'{token:08x}', 'position': position,
                    'instance_source': instance['source'],
                    'xyz_source_offsets': [field['offset'] for field in coordinates],
                })
        else:
            require(position == source['pose']['translation'],
                    'Domination Objective position differs from qualified root')
        if subtype == 13:
            require(definition['class_name'] == STRUCTURAL and fields['Active']['value'],
                    'Unqualified anti-air asset')
            assets.append({'object_index': index, 'position': position})
        if token in membership:
            owner = fields['OwnerTeam']['value']
            require(owner in (0, 1), 'Unsupported Objective owner team')
            objectives.append({
                'object_index': index, 'position': position, 'category': enum(fields, 'Category'),
                'subtype': subtype, 'team': owner, 'platoons': sorted(membership[token]),
            })
    require(len(assets) == (2 if mode == 1 else 4), 'Incomplete retail anti-air roster')
    return tuple(sorted(rows, key=lambda row: row['object_index']) for rows in (assets, objectives, roots))


def project(archive_bytes, objective_bytes, evidence_bytes, spawn, properties, maps, helpers, psarc,
            parse_zrb, trigger_evidence=None):
    catalog = json.loads(objective_bytes)
    evidence = json.loads(evidence_bytes)
    name, mode = catalog['map_name'], catalog['game_type']
    require(re.fullmatch(r'mag_[a-z0-9_]+', name) and mode in (1, 2)
            and catalog['application_id'] == 20231 and catalog['network_type'] == 12
            and catalog['source']['source_image_sha256'] == RETAIL_SHA256,
            'Expected retail Domination or Acquisition Objective catalog')
    indices = catalog_indices(catalog)
    evidence_sha = digest(evidence_bytes)
    if mode == 1:
        source, source_rows = acquisition_inputs(evidence, evidence_sha, catalog, indices)
    else:
        source, source_rows = domination_inputs(evidence, catalog, indices)
    require(digest(archive_bytes) == source['archive_sha256'], 'Wrong original level archive SHA-256')
    archive = psarc(archive_bytes)
    def member(path):
        require(path in archive.names, f'Missing original level member {path}')
        return archive.extract(archive.names.index(path) + 1)
    clv = member(f'/run/mag/ps3/levels/{name}/{name}.clv')
    require(digest(clv) == source['clv_sha256'], 'Wrong original level member SHA-256')
    bank_names = [path for path in archive.names if path.endswith('/resource_blocks/globaldata.zrb')]
    require(len(bank_names) == 1, 'Missing or ambiguous globaldata resource bank')
    bank = member(bank_names[0])
    horizontal, vertical = bounds(clv, terrain_extent(clv, parse_zrb(bank, retail13_compat=True)['records']))
    membership = platoon_membership(clv, mode, spawn, properties)
    graph = maps['objective_graph'](clv, helpers)
    aaa, objectives, roots = objective_rows(graph, indices, source_rows, membership, mode)
    volume_tool = runpy.run_path(str(TOOLS / 'extract-retail-objective-volumes.py'))
    parsed = spawn['retail_clv'](clv)
    volume_projection = volume_tool['project_level'](
        graph, parsed.tables('InstanceProperties'), volume_tool['classes_by_token'](parsed),
        helpers['euler_basis'],
    )
    volumes = [trigger for row in volume_projection['objectives'] for trigger in row['triggers']]
    vector_slots = {}
    if trigger_evidence is not None:
        require(mode == 2, 'Trigger-vector evidence requires Domination')
        vector_slots = domination_trigger_slots(json.loads(trigger_evidence), catalog, evidence, source, volumes)
    damage_triggers = damage_trigger_rows(graph, volumes, indices, vector_slots)
    root_receipt = None
    if mode == 1:
        root_receipt = {
            'schema': 'mag_private_retail_acquisition_ability_roots_v1',
            'retail_elf_sha256': RETAIL_SHA256, 'objective_initial_binding_sha256': evidence_sha,
            'level_archive_sha256': source['archive_sha256'], 'level_clv_sha256': source['clv_sha256'],
            'map': name, 'roots': roots,
        }
    binding = {
        'schema_version': 3, 'application_id': 20231, 'map_name': name,
        'source': {
            'retail_elf_sha256': RETAIL_SHA256, 'level_archive_sha256': source['archive_sha256'],
            'level_clv_sha256': source['clv_sha256'], 'terrain_resource_bank_sha256': digest(bank),
            'gameplay_objectives_sha256': digest(objective_bytes),
            'qualified_objective_poses_sha256': digest(json_bytes(root_receipt)) if root_receipt else evidence_sha,
        },
        'boundary': horizontal, 'vertical_bounds': vertical, 'aaa': aaa, 'objectives': objectives,
        'damage_triggers': damage_triggers,
        'precision_targets': precision_target_rows(graph, indices),
    }
    return binding, root_receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--level-archive', required=True, type=Path, help='Original retail level PSARC')
    parser.add_argument('--gameplay-objectives', required=True, type=Path, help='Exact worker Objective catalog')
    evidence = parser.add_mutually_exclusive_group(required=True)
    evidence.add_argument('--domination-qualified-poses', type=Path, help='Qualified per-map Objective root receipt')
    evidence.add_argument('--acquisition-initial-bindings', type=Path, help='Retail initial Objective identity receipt')
    parser.add_argument('--domination-trigger-bindings', type=Path,
                        help='Qualified Domination Objective trigger-vector receipt')
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--acquisition-roots-output', type=Path, help='Optional generated Acquisition root receipt')
    args = parser.parse_args()
    sys.path.insert(0, str(TOOLS.parent / 'collision'))
    from archive import Psarc
    from zrb_records import parse

    def tools(name):
        return runpy.run_path(str(TOOLS / name))

    try:
        objectives = args.gameplay_objectives.read_bytes()
        expected_mode = 1 if args.acquisition_initial_bindings else 2
        require(json.loads(objectives)['game_type'] == expected_mode,
                'Objective catalog mode and evidence choice disagree')
        require(not args.acquisition_roots_output or expected_mode == 1,
                'Acquisition roots output requires Acquisition evidence')
        binding, roots = project(
            args.level_archive.read_bytes(), objectives,
            (args.domination_qualified_poses or args.acquisition_initial_bindings).read_bytes(),
            tools('extract-retail-spawns.py'), tools('project-retail-sabotage-objectives.py'),
            tools('extract-retail-maps.py'), tools('retail_map_helpers.py'), Psarc, parse,
            args.domination_trigger_bindings.read_bytes() if args.domination_trigger_bindings else None,
        )
        args.output.write_bytes(json_bytes(binding))
        if args.acquisition_roots_output:
            args.acquisition_roots_output.write_bytes(json_bytes(roots))
    except (OSError, KeyError, TypeError, ValueError, OverflowError, struct.error) as exc:
        parser.exit(1, f'{parser.prog}: {exc}\n')


if __name__ == '__main__':
    main()
