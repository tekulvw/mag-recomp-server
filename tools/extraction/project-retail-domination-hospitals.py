#!/usr/bin/env python3
"""Project a hospital world from qualified placement and allocation evidence.

The default mode projects Domination's 16 hospitals. ``--mode acquisition``
projects an Acquisition map's defending field hospitals (spawn type 8) into the
same placement schema, so the runtime seats their lives the same way.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path

SCOPE = 'successful_production_fresh_level_load'
PASSENGER_SCOPE = 'retail_unresolved_passenger_node_zero_local_pose'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def require(ok, message):
    if not ok:
        raise ValueError(message)


def token(value):
    return int(value, 0) if isinstance(value, str) else value


def rows_by_key(rows, key):
    result = {row[key]: row for row in rows}
    require(len(result) == len(rows), f'duplicate {key}')
    return result


def inverse_rotate(rotation, offset):
    x, y, z, w = rotation
    a, b, c = -x, -y, -z
    i, j, k = offset
    t = (2 * (b*k - c*j), 2 * (c*i - a*k), 2 * (a*j - b*i))
    return [i + w*t[0] + b*t[2] - c*t[1],
            j + w*t[1] + c*t[0] - a*t[2],
            k + w*t[2] + a*t[1] - b*t[0]]


def encode(value):
    return (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()


def promote_spawns(spawn_raw, authored_raw, allocation_raw):
    """Enable hospital passenger routing for an exact, already remapped spawn profile."""
    catalog, authored = map(json.loads, (spawn_raw, authored_raw))
    provenance = json.loads(catalog['source'])
    require(provenance['native_spawn_bindings_sha256'] == authored['spawn_binding_sha256'],
            'hospital spawn profile uses a different native pool')
    active = catalog['active']
    name = active['map_name']
    pool = rows_by_key(catalog['maps'], 'name')[name]
    level = rows_by_key(authored['maps'], 'map')[name]
    points = rows_by_key(pool['locations'], 'index')
    squads = rows_by_key(pool['squads'], 'index')
    hospital_indices = {row['spawn_index'] for row in level['hospitals']}
    unsupported = rows_by_key(provenance['unsupported_spawn_links'], 'index')
    require(catalog['match_profile']['has_bunkers'] is False and len(hospital_indices) == 16,
            'expected an unpromoted complete Domination hospital set')
    for row in level['hospitals']:
        point = points[row['spawn_index']]
        link = row['squad_link']
        squad_number = ('Alpha', 'Bravo', 'Charlie', 'Delta').index(link['squad'])
        address = 16 + 4 * link['platoon_id'] + squad_number
        mapping = unsupported[point['index']]
        require(mapping['spawn_type'] == 8 and mapping['user_class'] == 0
                and mapping['squad_links'] == [[1, link['platoon_id'], squad_number, link['slot']]]
                and row['team'] == link['team'] == 1
                and point['token'] == token(row['spawn_token']) and point.get('active') is True
                and point['kind'] == 'unsupported',
                'hospital promotion differs from original spawn membership')
        point['kind'] = 'bunker_passenger'
        available = squads[address]['available']
        require(point['index'] not in available and link['slot'] <= len(available),
                'invalid hospital spawn display order')
        available.insert(link['slot'], point['index'])
    active['available'] = squads[active['squad_index']]['available']
    provenance['unsupported_spawn_links'] = [row for row in provenance['unsupported_spawn_links']
                                             if row['index'] not in hospital_indices]
    provenance['hospital_allocation_sha256'] = sha(allocation_raw)
    catalog['source'] = json.dumps(provenance, sort_keys=True, separators=(',', ':'))
    catalog['match_profile']['has_bunkers'] = True
    return encode(catalog)


def project(spawn_raw, structure_raw, authored_raw, clearance_raw, allocation_raw, passenger_pose_raw):
    spawns, structures, authored, clearance, allocation, passenger_pose = map(
        json.loads, (spawn_raw, structure_raw, authored_raw, clearance_raw, allocation_raw, passenger_pose_raw)
    )
    policy = spawns['match_profile']
    require(spawns['format'] == 'mag_spawn_catalog' and policy['game_type'] == 2
            and policy['auto_assign_teams'] is True and policy['has_bunkers'] is True,
            'require a Domination hospital spawn profile')
    require(structures['application_id'] == 20231 and
            structures['source_archive_sha256'] == authored['retail_game_sha256'],
            'retail structure source differs')
    require(authored['format'] == 'mag_retail_domination_hospital_placer_inputs'
            and clearance['format'] == 'mag_retail_domination_hospital_static_clearance'
            and clearance['placer_inputs_sha256'] == sha(authored_raw)
            and clearance['retail_elf_sha256'] == authored['retail_elf_sha256'],
            'unbound hospital clearance')
    require(allocation['format'] == 'mag_domination_hospital_allocation_v1'
            and allocation['allocation_scope'] == SCOPE
            and allocation['retail_elf_sha256'] == authored['retail_elf_sha256']
            and allocation['placer_inputs_sha256'] == sha(authored_raw)
            and allocation['static_clearance_sha256'] == sha(clearance_raw)
            and allocation['fresh_family13_pool'] is True
            and allocation['all_initial_class4_creations_succeed'] is True
            and allocation['production_without_early_other_class4_creations'] is True,
            'unqualified initial hospital allocation')
    map_name = spawns['active']['map_name']
    pool = rows_by_key(spawns['maps'], 'name')[map_name]
    level = rows_by_key(authored['maps'], 'map')[map_name]
    static = rows_by_key(clearance['maps'], 'map')[map_name]
    allocation_level = rows_by_key(allocation['maps'], 'map')[map_name]
    require(allocation_level['level_archive_sha256'] == level['level_source']['archive_sha256']
            and allocation_level['level_clv_sha256'] == level['level_source']['clv_sha256']
            and pool['owner_faction'] == level['owner_faction_id']
            and pool['runtime_indices_verified'] is True,
            'hospital source level differs')
    variants = authored['faction_sources'][level['owner_faction']]
    require(passenger_pose['format'] == 'mag_domination_hospital_passenger_pose_v1'
            and passenger_pose['scope'] == PASSENGER_SCOPE
            and passenger_pose['retail_elf_sha256'] == authored['retail_elf_sha256']
            and passenger_pose['placer_inputs_sha256'] == sha(authored_raw),
            'unbound hospital passenger pose')
    seat_pose = rows_by_key(passenger_pose['models'], 'faction')[level['owner_faction']]
    require(seat_pose['properties_sha256'] == variants['sha256']
            and seat_pose['body_member_sha256'] == variants['neutral_body']['member_sha256']
            and seat_pose['local_position_xyz'] == [0., 0., 0.],
            'unsupported hospital passenger local pose')
    member = variants['source_member'].split('/scripts/', 1)[1]
    require(structures['source_sha256'][member] == variants['sha256'],
            'hospital vehicle properties differ')
    mounts = []
    for index, (key, point) in enumerate(variants['mount_points']['MountPointPropertiesList'].items()):
        require(key == f'MountPointProperties{index}', 'non-contiguous hospital mount list')
        row = point['MountPointProperties']
        mounts.append(dict(local_offset=[float(value) for value in row['PositionOffset'].split()],
                           radius=float(row['Radius'])))
    seat_properties = variants['seats']['SeatPropertiesList']
    require(list(seat_properties) == ['SeatProperties0', 'SeatProperties1']
            and seat_properties['SeatProperties0']['SeatProperties']['SeatType'] == 'TurretOperatorSeat'
            and seat_properties['SeatProperties1']['SeatProperties']['SeatType'] == 'PassengerSeat',
            'unsupported hospital seat configuration')
    max_occupancy = int(variants['seats']['MaxOccupancy'])
    passenger_seats = int(seat_properties['SeatProperties1']['SeatProperties']['SeatCount'])
    require(seat_pose['seats'] == passenger_seats and seat_pose['occupant_visible'] is False,
            'passenger seat pose does not match source seats')
    ranked = rows_by_key(variants['dismount_offsets'], 'index')
    points = rows_by_key([point for point in pool['locations'] if point['kind'] == 'bunker_passenger'], 'index')
    original = rows_by_key(level['hospitals'], 'spawn_token')
    clearances = rows_by_key(static['placements'], 'spawn_token')
    allocation_rows = rows_by_key(allocation_level['objects'], 'spawn_token')
    require(len(original) == len(points) == 16 and set(original) == set(clearances) == set(allocation_rows),
            'incomplete Domination hospital sources')
    require({row['object_index'] for row in allocation_rows.values()} == set(range(16)),
            'incomplete family13 allocation')
    placements = []
    for name, row in original.items():
        obj = allocation_rows[name]
        clear = clearances[name]
        index = row['spawn_index']
        point = points[index]
        require(point['token'] == token(name) and point.get('active', True)
                and clear['spawn_index'] == obj['spawn_index'] == index
                and obj['object_index'] == row['candidate_type13_index_if_all_initial_creations_succeed']
                == clear['initial_type13_source_model_index'],
                'hospital allocation moved a source identity')
        origin = clear['placement_xyz']
        rotation = clear['rotation_xyzw']
        require(len(origin) == 3 and len(rotation) == 4
                and abs(sum(v*v for v in rotation) - 1.) < 0.001
                and all(math.isfinite(v) for v in origin + rotation),
                'invalid qualified hospital transform')
        candidates = []
        for exit in rows_by_key(clear['neutral_exits'], 'index').values():
            rank = ranked[exit['index']]['local_xyz']
            world = exit['wire_feet_xyz']
            local = inverse_rotate(rotation, [a-b for a, b in zip(world, origin, strict=True)])
            require(all(math.isfinite(v) for v in local), 'nonfinite hospital exit')
            candidates.append(dict(ranking_offset=rank, position_offset=local))
        require(candidates, 'hospital requires at least one qualified exit')
        placements.append(dict(object_index=obj['object_index'], spawn_index=index,
                               spawn_token=token(name), properties_source=member,
                               mounts=mounts, max_occupancy=max_occupancy,
                               passenger_seats=passenger_seats,
                               exit=dict(transform=dict(position=dict(zip('xyz', origin, strict=True)),
                                                        rotation_xyzw=rotation),
                                         candidates=candidates)))
    tokens = {token(row['vehicle_type_token']) for row in original.values()}
    require(len(tokens) == 1 and 0 not in tokens, 'invalid hospital vehicle type token')
    return dict(format='mag_domination_hospital_placements_v1', map_name=map_name,
                spawn_catalog_sha256=sha(spawn_raw), structure_catalog_sha256=sha(structure_raw),
                source_proof_sha256=sha(allocation_raw), allocation_scope=SCOPE,
                passenger_pose_sha256=sha(passenger_pose_raw), passenger_pose_scope=PASSENGER_SCOPE,
                vehicle_type_token=tokens.pop(),
                placements=sorted(placements, key=lambda placement: placement['object_index']))


ACQUISITION_EXITS = 'private_acquisition_field_hospital_exits_v1'
ALLOCATION = 'mag_domination_hospital_allocation_v1'
PLACEMENTS = 'mag_domination_hospital_placements_v1'


def field_hospitals(exits, map_name):
    """Externally qualified Acquisition field-hospital rows for one map."""
    require(exits.get('format') == ACQUISITION_EXITS and exits.get('map') == map_name,
            'field-hospital rows describe a different map')
    rows = rows_by_key(exits['field_hospitals'], 'spawn_index')
    require(rows and len(rows) <= 16, 'unsupported field-hospital count')
    for row in rows.values():
        policy = row['bunker_exit_policy']
        position = policy['transform']['position']
        origin = [position[axis] for axis in 'xyz']
        rotation = policy['transform']['rotation_xyzw']
        require((row['spawn_type'], row['category'], row['team']) == (8, 2, 1)
                and row['active'] is True and row['vehicle_mountable'] is True
                and token(row['vehicle_type_token']) != 0
                and origin == row['emplacement_origin_xyz']
                and len(rotation) == 4 and abs(sum(v*v for v in rotation) - 1.) < 0.001
                and all(math.isfinite(v) for v in origin + rotation)
                and policy['candidates']
                and all(len(c['ranking_offset']) == len(c['position_offset']) == 3
                        and all(math.isfinite(v) for v in c['ranking_offset'] + c['position_offset'])
                        for c in policy['candidates']),
                'unqualified field-hospital row')
    return rows


def acquisition_allocation(spawn_raw, exits_raw, model_raw):
    """Apply the conditional fresh-load allocation model to one Acquisition map.

    The model's premises (a fresh family-13 pool in which every initial class-4
    creation succeeds and nothing else is created first) say each hospital
    takes the lowest free index in serialized spawn order. The model's own maps
    must already follow that order; the Acquisition map inherits it.
    """
    catalog, exits, model = map(json.loads, (spawn_raw, exits_raw, model_raw))
    provenance = json.loads(catalog['source'])
    map_name = catalog['active']['map_name']
    require(catalog['match_profile']['game_type'] == 1, 'require an Acquisition spawn profile')
    require(model['format'] == ALLOCATION and model['allocation_scope'] == SCOPE
            and model['fresh_family13_pool'] is True
            and model['all_initial_class4_creations_succeed'] is True
            and model['production_without_early_other_class4_creations'] is True
            and model['retail_elf_sha256'] == provenance['native_elf_sha256'],
            'unqualified hospital allocation model')
    for level in model['maps']:
        ordered = sorted(level['objects'], key=lambda row: row['spawn_index'])
        require([row['object_index'] for row in ordered] == list(range(len(ordered))),
                'allocation model does not follow spawn order')
    rows = field_hospitals(exits, map_name)
    result = {key: value for key, value in model.items() if key != 'maps'}
    result['description'] = ('Conditional source model applied to an Acquisition map: each defending '
                             'field hospital takes the lowest free family-13 index in spawn order.')
    result['maps'] = [dict(map=map_name, level_archive_sha256=provenance['level_archive_sha256'],
                           level_clv_sha256=provenance['level_clv_sha256'],
                           objects=[dict(object_index=rank, spawn_index=index,
                                         spawn_token='0x%08x' % token(rows[index]['spawn_token']))
                                    for rank, index in enumerate(sorted(rows))])]
    return encode(result)


def promote_acquisition_spawns(spawn_raw, exits_raw, allocation_raw):
    """Route an Acquisition profile's field-hospital points to their emplacements."""
    catalog, exits = map(json.loads, (spawn_raw, exits_raw))
    provenance = json.loads(catalog['source'])
    active = catalog['active']
    name = active['map_name']
    require(catalog['match_profile']['game_type'] == 1 and catalog['match_profile']['has_bunkers'] is False,
            'expected an unpromoted Acquisition spawn profile')
    pool = rows_by_key(catalog['maps'], 'name')[name]
    points = rows_by_key(pool['locations'], 'index')
    squads = rows_by_key(pool['squads'], 'index')
    rows = field_hospitals(exits, name)
    links = rows_by_key(provenance['unsupported_spawn_links'], 'index')
    type8 = {index for index, link in links.items() if link['spawn_type'] == 8}
    require(type8 == set(rows), 'field-hospital rows differ from the spawn type-8 inventory')
    for index, row in rows.items():
        point, link = points[index], links[index]
        require(point['token'] == token(row['spawn_token']) and point.get('active') is True
                and point['kind'] in ('alternate', 'unsupported')
                and link['user_class'] == 0 and link['category'] == 2
                and len(link['squad_links']) == 1 and link['squad_links'][0][0] == 1,
                'field-hospital promotion differs from original spawn membership')
        _, platoon, squad, slot = link['squad_links'][0]
        available = squads[16 + 4 * platoon + squad]['available']
        if index not in available:
            require(slot <= len(available), 'invalid field-hospital display order')
            available.insert(slot, index)
        require(available.index(index) == slot, 'field-hospital display order differs')
        point['kind'] = 'bunker_passenger'
    if active['squad_index'] in squads:
        active['available'] = squads[active['squad_index']]['available']
    provenance['unsupported_spawn_links'] = [row for row in provenance['unsupported_spawn_links']
                                             if row['index'] not in rows]
    provenance['hospital_allocation_sha256'] = sha(allocation_raw)
    catalog['source'] = json.dumps(provenance, sort_keys=True, separators=(',', ':'))
    catalog['match_profile']['has_bunkers'] = True
    return encode(catalog)


def project_acquisition(spawn_raw, structure_raw, exits_raw, allocation_raw, seat_raws):
    """Project bound field hospitals; seats and mounts come from qualified
    placement worlds built on the same structure catalog and properties."""
    spawns, structures, exits, allocation = map(
        json.loads, (spawn_raw, structure_raw, exits_raw, allocation_raw))
    references = [json.loads(raw) for raw in seat_raws]
    policy = spawns['match_profile']
    require(spawns['format'] == 'mag_spawn_catalog' and policy['game_type'] == 1
            and policy['auto_assign_teams'] is True and policy['has_bunkers'] is True,
            'require a promoted Acquisition spawn profile')
    require(structures['application_id'] == 20231 and references
            and all(seats['format'] == PLACEMENTS and seats['allocation_scope'] == SCOPE
                    and seats['passenger_pose_scope'] == PASSENGER_SCOPE
                    and seats['structure_catalog_sha256'] == sha(structure_raw)
                    and (seats['passenger_pose_sha256'], seats['vehicle_type_token'])
                    == (references[0]['passenger_pose_sha256'], references[0]['vehicle_type_token'])
                    for seats in references),
            'seat reference uses a different structure catalog')
    seats = references[0]
    require(allocation['format'] == ALLOCATION and allocation['allocation_scope'] == SCOPE
            and json.loads(spawns['source'])['hospital_allocation_sha256'] == sha(allocation_raw),
            'unbound field-hospital allocation')
    map_name = spawns['active']['map_name']
    pool = rows_by_key(spawns['maps'], 'name')[map_name]
    points = rows_by_key([point for point in pool['locations'] if point['kind'] == 'bunker_passenger'], 'index')
    rows = field_hospitals(exits, map_name)
    objects = rows_by_key(rows_by_key(allocation['maps'], 'map')[map_name]['objects'], 'spawn_index')
    require(set(points) == set(rows) == set(objects), 'incomplete Acquisition hospital sources')
    reference = {}
    for row in (row for seats in references for row in seats['placements']):
        value = dict(mounts=row['mounts'], max_occupancy=row['max_occupancy'],
                     passenger_seats=row['passenger_seats'])
        require(reference.setdefault(row['properties_source'], value) == value,
                'seat reference disagrees for one properties source')
    placements, tokens = [], set()
    for index, row in rows.items():
        member = 'vehicle/' + row['properties_file']
        require(member in structures['source_sha256'] and member in reference
                and points[index]['token'] == token(row['spawn_token'])
                and objects[index]['spawn_token'] == '0x%08x' % token(row['spawn_token']),
                'field hospital differs from its sources')
        tokens.add(token(row['vehicle_type_token']))
        placements.append(dict(object_index=objects[index]['object_index'], spawn_index=index,
                               spawn_token=token(row['spawn_token']), properties_source=member,
                               **reference[member], exit=row['bunker_exit_policy']))
    require(tokens == {seats['vehicle_type_token']}, 'field-hospital vehicle type differs')
    return dict(format=PLACEMENTS, map_name=map_name,
                spawn_catalog_sha256=sha(spawn_raw), structure_catalog_sha256=sha(structure_raw),
                source_proof_sha256=sha(allocation_raw), allocation_scope=SCOPE,
                passenger_pose_sha256=seats['passenger_pose_sha256'], passenger_pose_scope=PASSENGER_SCOPE,
                vehicle_type_token=seats['vehicle_type_token'],
                placements=sorted(placements, key=lambda placement: placement['object_index']))


def write_outputs(pairs, inputs, check):
    resolved = [path.resolve() for path, _ in pairs]
    require(len(set(resolved)) == len(pairs)
            and not set(resolved).intersection(path.resolve() for path in inputs),
            'hospital output is an input or duplicate path')
    for path, data in pairs:
        if check:
            require(path.read_bytes() == data, f'hospital output differs at {path}')
        else:
            with path.open('xb') as stream:
                stream.write(data)


def main_acquisition(args):
    required = ('spawn_catalog', 'structures', 'field_hospitals', 'allocation_model', 'seat_reference',
                'allocation_output', 'qualified_spawn_output', 'output')
    missing = [name for name in required if getattr(args, name) is None]
    require(not missing, 'acquisition mode requires ' + ', '.join('--' + m.replace('_', '-') for m in missing))
    inputs = (args.spawn_catalog, args.structures, args.field_hospitals, args.allocation_model,
              *args.seat_reference)
    spawn, structures, exits, model, *seats = (path.read_bytes() for path in inputs)
    allocation = acquisition_allocation(spawn, exits, model)
    spawn = promote_acquisition_spawns(spawn, exits, allocation)
    output = encode(project_acquisition(spawn, structures, exits, allocation, seats))
    write_outputs([(args.allocation_output, allocation), (args.qualified_spawn_output, spawn),
                   (args.output, output)], inputs, args.check)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=('domination', 'acquisition'), default='domination')
    for name in ('spawn-catalog', 'structures', 'authored-hospitals', 'clearance', 'allocation', 'passenger-pose',
                 'output', 'qualified-spawn-output', 'field-hospitals', 'allocation-model', 'allocation-output'):
        parser.add_argument('--' + name, type=Path)
    parser.add_argument('--seat-reference', type=Path, action='append',
                        help='qualified hospital placements on the same structure catalog (repeatable)')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if args.mode == 'acquisition':
        return main_acquisition(args)
    required = ('spawn_catalog', 'structures', 'authored_hospitals', 'clearance', 'allocation',
                'passenger_pose', 'output')
    missing = [name for name in required if getattr(args, name) is None]
    if missing:
        parser.error('domination mode requires ' + ', '.join('--' + m.replace('_', '-') for m in missing))
    paths = (args.spawn_catalog, args.structures, args.authored_hospitals, args.clearance, args.allocation,
             args.passenger_pose)
    spawn, structures, authored, clearance, allocation, passenger_pose = (path.read_bytes() for path in paths)
    outputs = [args.output] + ([args.qualified_spawn_output] if args.qualified_spawn_output else [])
    resolved = [path.resolve() for path in outputs]
    require(len(set(resolved)) == len(outputs)
            and not set(resolved).intersection(path.resolve() for path in paths),
            'hospital output is an input or duplicate path')
    if args.qualified_spawn_output:
        spawn = promote_spawns(spawn, authored, allocation)
    output = encode(project(spawn, structures, authored, clearance, allocation, passenger_pose))
    for path, data in zip(outputs, (output, spawn), strict=False):
        if args.check:
            require(path.read_bytes() == data, f'hospital output differs at {path}')
        else:
            with path.open('xb') as stream:
                stream.write(data)


if __name__ == '__main__':
    main()
