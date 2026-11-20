#!/usr/bin/env python3
"""Project source-bound Domination Structural sequence and blast inputs without running assets."""

import argparse
import hashlib
import json
import math
import re
from pathlib import Path
import runpy
import struct
import sys
import zlib


TOOLS = Path(__file__).resolve().parent
SEQUENCE_CLASS = 'f27e3dff'
MAIN_ARCHIVE_SHA256 = 'a2e8345cd9f5c650ac22544096ba83e8c9e6f0fffd0ef69eb8791fb265f9baad'
STATES = ('Inactive', 'Deactivated', 'Activating', 'Activated', 'Deactivating',
          'Completed', 'Repairing', 'Repaired', 'Locked', 'Retaking', 'Retaken', 'CancelRetaken')
BLAST_FIELDS = ('FullDamageRadius', 'HalfDamageRadius', 'DamageValue', 'MaterialPenetration',
                'CharacterDamageOverTimeRadius', 'CharacterDamageOverTimeValue')
SEQUENCE_CONTROL = {'PlayDependentSequenceEventDef', 'PlayIndependentSequenceEventDef', 'StopSequencesEventDef'}
VISUAL_EVENTS = {'PrintEventDef', 'ParticleEventDef', 'SoundEventDef', 'ExplosionEventDef'}
EVENT_CLASSES = SEQUENCE_CONTROL | VISUAL_EVENTS | {'AnimObjectCreationEventDef'}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def sequence_events(data):
    def word(offset):
        require(0 <= offset <= len(data) - 4, 'Truncated sequence')
        return struct.unpack_from('>I', data, offset)[0]

    def string(offset, limit):
        size = word(offset)
        start, end = offset + 4, offset + 4 + size
        require(1 < size <= 128 and end <= limit and data[end - 1] == 0, 'Invalid sequence string')
        return data[start:end - 1].decode('ascii'), end

    def typed(fields, key, kind, size, default=None):
        if key not in fields and default is not None:
            return default
        actual, raw = fields[key]
        require(actual == kind and len(raw) == size, f'Wrong sequence field type for {key}')
        return raw

    def scalar(fields, key):
        value, = struct.unpack('>f', typed(fields, key, 'float', 4))
        require(math.isfinite(value), f'Nonfinite sequence field {key}')
        return value

    def vector(fields, key):
        raw = typed(fields, key, 'SMath::Vec3', 16, bytes(16))
        xyz = struct.unpack('>3f', raw[:12])
        require(all(map(math.isfinite, xyz)) and raw[12:] == bytes(4), f'Invalid vector {key}')
        return xyz

    def token(fields, key, default=None):
        return typed(fields, key, 'util::Token', 4, default).hex()

    count = word(0)
    require(count <= 128, 'Excess sequence events')
    cursor, result = 4, []
    for _ in range(count):
        end = cursor + word(cursor)
        require(cursor + 21 < end <= len(data)
                and word(cursor + 4) == 3
                and data[cursor + 12:cursor + 20] == bytes.fromhex('deadbeef00000003'),
                'Invalid sequence event boundary')
        event, pos = string(cursor + 20, end)
        require(event in EVENT_CLASSES, f'Unsupported sequence event {event}')
        fields = {}
        while pos < end and data[pos] == 1:
            key, pos = string(pos + 1, end)
            kind, pos = string(pos, end)
            size = word(pos)
            require(key not in fields and 4 <= size <= end - pos, f'Invalid sequence field {key}')
            fields[key] = (kind, data[pos + 4:pos + size])
            pos += size
        require(pos == end - 1 and data[pos] == 0, 'Invalid sequence event terminator')
        if event == 'AnimObjectCreationEventDef':
            require(set(fields) <= {'objectName', 'geometry', 'collision', 'skeletonDef',
                                    'position', 'rotation', 'start', 'dur'}, 'Unsupported creation field')
            start, duration = scalar(fields, 'start'), scalar(fields, 'dur')
            require(start >= 0 and (duration == -1 or duration >= 0), 'Invalid creation event timing')
            result.append({
                'event_index': len(result),
                'kind': 'create_animated_object',
                'object_token': token(fields, 'objectName'),
                'geometry_token': token(fields, 'geometry'),
                'collision_token': token(fields, 'collision'),
                'skeleton_token': token(fields, 'skeletonDef', bytes(4)),
                'position': vector(fields, 'position'),
                'rotation': vector(fields, 'rotation'),
                'start_seconds': start,
                'duration_seconds': duration,
            })
        elif event in SEQUENCE_CONTROL:
            name_field = 'SequencesName' if event == 'StopSequencesEventDef' else 'SequenceName'
            result.append({'event_index': len(result), 'kind': event,
                           'sequence_token': token(fields, name_field)})
        else:
            result.append({'event_index': len(result), 'kind': event})
        cursor = end
    require(len(data) - cursor < 16 and not any(data[cursor:]), 'Unexpected sequence header tail')
    return result


def qualify_visual_sequence_controls(projected, sequence_records):
    # Follow every start and stop target by typed resource identity. Only the
    # top-level state sequences may contain geometry. Nested lifecycle can be
    # omitted from this collision projection only after that graph is checked.
    missing_stop_targets = set()
    visited = set()
    pending = [event for seq in projected.values() for event in seq['events']
               if event['kind'] in SEQUENCE_CONTROL]
    while pending:
        event = pending.pop()
        token = event['sequence_token']
        stop = event['kind'] == 'StopSequencesEventDef'
        if token not in sequence_records:
            require(stop, 'Missing original sequence start resource')
            missing_stop_targets.add(token)
            continue
        if token in visited:
            continue
        visited.add(token)
        record = sequence_records[token]
        require(record['resource_version'] == 3, 'Unsupported nested sequence version')
        nested = sequence_events(bytes.fromhex(record['unparsed_resource_header_hex']))
        require(all(event['kind'] in SEQUENCE_CONTROL | VISUAL_EVENTS for event in nested),
                'Nested sequence touches collision gameplay')
        pending.extend(event for event in nested if event['kind'] in SEQUENCE_CONTROL)
    for seq in projected.values():
        for event in seq['events']:
            if event['kind'] in SEQUENCE_CONTROL:
                event['kind'] = 'VisualSequenceControl'
                del event['sequence_token']

    return missing_stop_targets


def penetration_costs(main, game, header_parser):
    def member(archive, name):
        path = '/run/mag/ps3/' + name
        require(archive.names.count(path) == 1, f'Missing typed material source {name}')
        return archive.extract(archive.names.index(path) + 1)

    common = member(main, 'global_resources/global_common.zrb')
    properties = member(game, 'scripts/materialproperties.zpr')
    records = header_parser['parse'](common, retail13_compat=True)['records']
    physics = {}
    marker = b'\x00\x00\x00\x14MaterialPenetration\x00\x00\x00\x00\x06float\x00\x00\x00\x00\x08'
    for record in records:
        if record['class_token'] != 'd40a7556':
            continue
        source = bytes.fromhex(record['unparsed_resource_header_hex'])
        require(source.count(marker) == 1 and b'PhysicsProperties\x00' in source,
                'Invalid PhysicsProperties material record')
        at = source.index(marker) + len(marker)
        value, = struct.unpack_from('>f', source, at)
        require(record['name_token'] not in physics and math.isfinite(value) and value >= -1,
                'Invalid PhysicsProperties penetration value')
        physics[record['name_token']] = value

    rows = re.split(rb'(?m)^\s*MaterialProperties([0-9]+)\s*=', properties)
    result = {}
    for name, body in zip(rows[1::2], rows[2::2]):
        index = int(name)
        field = re.findall(rb'(?m)^\s*PhysicsProps\s*=\s*"([^"\r\n]+)",', body)
        require(index not in result and len(field) == int(index != 255),
                'Ambiguous requested original material slot')
        if field:
            key = f'{zlib.crc32(field[0].lower()):08x}'
            require(key in physics, 'Unknown original PhysicsProps reference')
            result[index] = physics[key]
        else:
            result[index] = -1.0
    require(set(result) == set(range(27)) | {255}, 'Unexpected original material inventory')
    return result


def project(archive_path, metadata_path, collision_output=None, main_archive=None, game_archive=None):
    metadata_raw = metadata_path.read_bytes()
    metadata = json.loads(metadata_raw)
    require(metadata['mode_enum'] == 'GameType256', 'Wrong authored mode')
    raw = archive_path.read_bytes()
    archive_digest = sha(raw)
    matches = [row for row in metadata['maps'] if row['source']['archive_sha256'] == archive_digest]
    require(len(matches) == 1, 'No unique source archive match')
    row, = matches
    builder = runpy.run_path(str(TOOLS / 'build-retail-spawn-catalog.py'))
    archive = builder['extractor']().Psarc(raw)
    source = row['source']
    clv_name = source['clv_member']
    block_root = clv_name.rsplit('/', 1)[0] + '/resource_blocks/'
    data_name, physics_name = (block_root + name for name in ('globaldata.zrb', 'physics.zrb'))
    require(all(archive.names.count(name) == 1 for name in (clv_name, data_name, physics_name)),
            'Missing source members')
    clv = archive.extract(archive.names.index(clv_name) + 1)
    require(sha(clv) == source['clv_sha256'], 'CLV hash mismatch')
    data = archive.extract(archive.names.index(data_name) + 1)
    header_parser = runpy.run_path(str(TOOLS.parent / 'collision/zrb_records.py'))
    helpers = runpy.run_path(str(TOOLS / 'retail_map_helpers.py'))
    graph = runpy.run_path(str(TOOLS / 'extract-retail-maps.py'))['objective_graph'](clv, helpers)
    records = header_parser['parse'](data)['records']
    sequence_records = {record['name_token']: record for record in records
                        if record['class_token'] == SEQUENCE_CLASS}
    require(len(sequence_records) == sum(record['class_token'] == SEQUENCE_CLASS for record in records),
            'Ambiguous sequence resource identity')
    projected = {}
    definitions = {}
    for definition in graph['definitions']:
        structural = definition['class_name'] == 'ObjectiveDemolitionStructuralProperties'
        if not structural and definition['class_name'] != 'ObjectiveSecureAndHoldProperties':
            continue
        fields = definition['fields']
        require(fields['SequenceOwnerObjective']['raw_hex'] == '00000000',
                'Linked sequence owner requires a separate root pose')
        table = bytes.fromhex(fields['Sequences']['raw_hex'])
        version, length = struct.unpack_from('>2I', table)
        require(version == 1 and length <= 127 and len(table) == 8 + 4 * length,
                'Invalid source sequence vector')
        tokens = [table[offset:offset + 4].hex() for offset in range(8, len(table), 4)]
        indices = {state: fields[state + 'SequenceIndex']['value'] for state in STATES}
        require(all(-2 <= index < len(tokens) for index in indices.values()), 'Invalid sequence index')
        require(all(token in sequence_records for token in tokens), 'Unresolved sequence resource')
        for token in tokens:
            if token in projected:
                continue
            record = sequence_records[token]
            require(record['resource_version'] == 3, 'Unsupported sequence version')
            sequence_raw = bytes.fromhex(record['unparsed_resource_header_hex'])
            projected[token] = {'source_sha256': sha(sequence_raw), 'events': sequence_events(sequence_raw)}
        blast_properties = None
        if structural:
            subtype = bytes.fromhex(fields['SubType']['raw_hex'])
            require(len(subtype) == 13 and subtype[:9] == bytes.fromhex('deadbeef0000000100'),
                    'Invalid Structural subtype enum')
            blast_properties = {
                **{key: fields[key]['value'] for key in BLAST_FIELDS},
                'SubType': int.from_bytes(subtype[9:], 'big'),
            }
        definitions[definition['definition_token']] = {
            'state_sequence_indices': indices,
            'sequence_tokens': tokens,
            'blast_properties': blast_properties,
            'definition_sha256': definition['source_sha256'],
        }

    missing_stop_targets = qualify_visual_sequence_controls(projected, sequence_records)

    physics = archive.extract(archive.names.index(physics_name) + 1)
    parsed_physics = header_parser['parse'](physics, retail13_compat=True)
    need_collision = {event['collision_token']
                      for sequence in projected.values() for event in sequence['events']
                      if event['kind'] == 'create_animated_object'} - {'00000000'}
    collisions = {}
    for record in parsed_physics['records']:
        token = record['name_token']
        if token not in need_collision or record['class_token'] != '2a8bdf5a':
            continue
        require(token not in collisions and record['packfiles'], 'Ambiguous or empty collision resource')
        collisions[token] = {'packfile_sha256': [pack['sha256'] for pack in record['packfiles']]}

    placements = []
    for obj in graph['instances']:
        if obj['definition_token'] not in definitions:
            continue
        fields = {entry['name']: entry['value'] for entry in obj['fields']}
        translation = [fields['Translate' + axis] for axis in 'XYZ']
        angles = [fields['Rotate' + axis] for axis in 'XYZ']
        scale = [fields['Scale' + axis] for axis in 'XYZ']
        require(scale == [1, 1, 1] and all(math.isfinite(v) for v in translation + angles),
                'Unqualified Structural instance transform')
        basis = helpers['euler_basis'](angles)
        placements.append({
            'objective_token': obj['name_token'],
            'definition_token': obj['definition_token'],
            'pose': {'translation': translation, 'rotation': list(zip(*basis))},
        })
    require(placements and len(placements) <= 128, 'Invalid structural placement count')
    result = {
        'schema': 'mag_domination_structural_effect_inputs_v1',
        'map': row['map']['map'],
        'metadata_sha256': sha(metadata_raw),
        'level_archive_sha256': archive_digest,
        'level_clv_sha256': source['clv_sha256'],
        'global_data_sha256': sha(data),
        'physics_sha256': parsed_physics['sha256'],
        'collision_resources': collisions,
        'unresolved_collision_tokens': sorted(need_collision - set(collisions)),
        'definitions': definitions,
        'placements': placements,
        'sequences': projected,
    }

    require(not missing_stop_targets or main_archive is not None and game_archive is not None,
            'Original shared archives are required for missing stop sequence targets')
    if main_archive is not None and game_archive is not None:
        shared = [(main_archive, MAIN_ARCHIVE_SHA256), (game_archive, builder['GAME_ARCHIVE_SHA256'])]
        archives = [archive]
        for path, digest in shared:
            payload = path.read_bytes()
            require(sha(payload) == digest, 'Wrong original supplemental PSARC')
            archives.append(builder['extractor']().Psarc(payload))
        require(set(absent_resources(missing_stop_targets, archives, SEQUENCE_CLASS)) == missing_stop_targets,
                'Stop sequence target outside the qualified level')
        unresolved = result['unresolved_collision_tokens']
        missing = absent_collisions(unresolved, archives)
        result['confirmed_absent_collision_tokens'] = missing
        result['unresolved_collision_tokens'] = sorted(set(unresolved) - set(missing))
        result['collision_lookup_archives'] = [digest for _, digest in shared]
        result['material_penetration_costs'] = penetration_costs(archives[1], archives[2], header_parser)

    if collision_output is not None:
        export_collision(result, data, records, physics, parsed_physics['records'], collision_output)
    return result


def absent_collisions(tokens, archives):
    return absent_resources(tokens, archives, '2a8bdf5a')


def absent_resources(tokens, archives, resource_class):
    candidates = {token: bytes.fromhex(resource_class + token) for token in tokens}
    if not candidates:
        return []
    for archive in archives:
        for index, name in enumerate(archive.names, 1):
            if name.endswith('.zrb'):
                data = archive.extract(index)
                candidates = {token: needle for token, needle in candidates.items() if needle not in data}
                if not candidates:
                    return []
    return sorted(candidates)


def export_collision(projection, globaldata, graphics, physics, records, output):
    """Convert the authored physics packs to neutral mesh-local BVHs."""
    utilities = TOOLS.parent / 'collision'
    sys.path.insert(0, str(utilities))
    try:
        from packfile import parse_packfile
        from world import nondegenerate_triangles
        from parent_transforms import CompactTransform, Hierarchy
        from rigid_hierarchy import parse_rigid_resource
    finally:
        sys.path.pop(0)

    output.mkdir(parents=True, exist_ok=False)
    exported = {}
    for row in records:
        token = row['name_token']
        if token not in projection['collision_resources'] or row['class_token'] != '2a8bdf5a':
            continue
        require(row['physics_resource_type'] == 0, 'Unsupported animated collision resource type')
        pack_rows = []
        for pack in row['packfiles']:
            start = pack['offset']
            raw = physics[start:start + pack['bytes']]
            parsed = parse_packfile(raw)
            digest = pack['sha256']
            require(parsed['packfile_sha256'] == sha(raw) == digest
                    and not parsed['unsupported'] and not parsed['primitives']
                    and parsed['meshes'], 'Incomplete animated collision mesh')
            instances = []
            for mesh in parsed['meshes']:
                require(mesh['coordinate_space'] == 'collision_shape_local'
                        and mesh['extrusion_xyz'] == [0, 0, 0]
                        and mesh['collision_margin'] == 0, 'Unqualified animated collision shape')
                vertices = mesh['vertices_xyz']
                triangles, _degenerate = nondegenerate_triangles(vertices, mesh['triangles'])
                require(triangles, 'Empty neutral mesh')
                instance = {'id': mesh['shape_id'], 'vertices_xyz': vertices, 'triangles': triangles}
                subpart = mesh['subpart_index']
                if subpart < len(pack['material_indices']):
                    instance['material_index'] = pack['material_indices'][subpart]
                instances.append(instance)
            mesh_document = {
                'format': 'mag_collision_world', 'schema_version': 1, 'coordinate_space': 'world',
                'metadata': {
                    'build': 'retail-bcus98110-01.00', 'map_name': projection['map'],
                    'coverage': 'provisional',
                    'provenance': ['standalone_collision_pack_coordinates', 'packfile_sha256=' + digest],
                    'unknowns': ['The scene is in collision pack coordinates. Runtime parent pose is supplied separately.'],
                },
                'instances': instances,
            }
            payload = (json.dumps(mesh_document, separators=(',', ':'), allow_nan=False) + '\n').encode()
            filename = digest + '.json'
            path = output / filename
            if path.exists():
                require(path.read_bytes() == payload, 'Conflicting collision pack conversion')
            else:
                with path.open('xb') as file:
                    file.write(payload)
            pack_rows.append({'source_sha256': digest, 'neutral_sha256': sha(payload),
                              'parent_token': pack['parent_token'], 'collision_tag': pack['collision_tag'],
                              'material_indices': pack['material_indices']})
        exported[token] = pack_rows
    rigid = {row['name_token']: row for row in graphics if row['class_token'] == 'a4f4d23c'}
    parent_poses = {}
    for sequence in projection['sequences'].values():
        for event in sequence['events']:
            if event['kind'] != 'create_animated_object':
                continue
            geom = event['geometry_token']
            for pack in exported.get(event['collision_token'], []):
                parent = pack['parent_token']
                if parent == '00000000' or parent in parent_poses.get(geom, {}):
                    continue
                hierarchy = parse_rigid_resource(globaldata, rigid[geom])
                nodes = {node.node_id: node for node in hierarchy['nodes']}
                transform = list(hierarchy['reference_transforms'])
                selected = [node for node in nodes.values() if node.name_token == int(parent, 16)]
                require(len(selected) == 1, 'Unresolved authored collision parent')
                current, = selected
                while current.parent_node_id != 0xffff:
                    require(transform[current.transform_index].uniform_scale == 1.0,
                            'Unsupported scale in selected animated collision chain')
                    current = nodes[current.parent_node_id]
                # CreateInstance copies the resource transform vector. The
                # sequence then sets its fresh root to the composed event pose.
                transform[current.transform_index] = CompactTransform((0, 0, 0, 1), (0, 0, 0))
                world = Hierarchy(nodes.values(), enumerate(transform)).world_matrix_for_token(int(parent, 16))
                parent_poses.setdefault(geom, {})[parent] = {
                    'source_sha256': hierarchy['source_sha256'],
                    'pose': {'translation': list(world[3][:3]),
                             'rotation': [[world[col][row] for col in range(3)] for row in range(3)]},
                }

    index = {
        'schema': 'mag_domination_structural_collision_v1',
        'map': projection['map'],
        'level_archive_sha256': projection['level_archive_sha256'],
        'physics_sha256': projection['physics_sha256'],
        'resources': exported,
        'geometry_sha256': projection['global_data_sha256'],
        'parent_reference_poses': parent_poses,
    }
    (output / 'index.json').write_text(json.dumps(index, indent=2, allow_nan=False) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('level-archive', 'metadata', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--collision-output', type=Path)
    parser.add_argument('--main-archive', type=Path)
    parser.add_argument('--game-archive', type=Path)
    args = parser.parse_args()
    if (args.main_archive is None) != (args.game_archive is None):
        parser.error('Supply both original shared archives to verify collision lookup absence')
    outputs = [path.resolve() for path in (args.output, args.collision_output) if path is not None]
    if any(path.exists() or path.is_relative_to(TOOLS.parents[1]) for path in outputs):
        parser.error('Output paths must be new and outside the source checkout')
    if len(outputs) == 2 and any(a.is_relative_to(b) for a, b in (outputs, outputs[::-1])):
        parser.error('Projection and collision output paths must be distinct and separate')
    result = project(args.level_archive, args.metadata, args.collision_output,
                     args.main_archive, args.game_archive)
    with args.output.open('x') as file:
        file.write(json.dumps(result, indent=2, allow_nan=False) + '\n')


if __name__ == '__main__':
    main()
