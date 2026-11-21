#!/usr/bin/env python3
"""Project source-qualified retail Acquisition Escort gunner rules offline."""
import argparse
import math
from pathlib import Path
import re
import runpy


HERE = Path(__file__).resolve().parent
APC = runpy.run_path(str(HERE / 'project-retail-apc-turrets.py'))
COMMON, MAP, NATIVE = (APC[name] for name in ('COMMON', 'MAP', 'NATIVE'))
sha, encode, parse, require, name_token, indexed, token, f32 = (
    APC[name] for name in ('sha', 'encode', 'parse', 'require', 'name_token', 'indexed', 'token', 'f32'))
GAME_ARCHIVE_SHA256 = APC['GAME_ARCHIVE_SHA256']
MAIN_ARCHIVE_SHA256 = APC['MAIN_ARCHIVE_SHA256']
RETAIL_ELF_SHA256 = APC['RETAIL_ELF_SHA256']
FACTIONS = APC['FACTIONS']
ESCORT_TYPE = name_token('Escort')
SOURCE_POSES_SHA256 = '8ea758aeb8d979db3f117d5cf7f587b966a3294c1758c2933d5cfce806ef8382'
VEHICLE_WAVES_SHA256 = '55d747e11a943a2fd5461c50e8d7baf3f014c1e0693896455a714ccac8d1593b'
PASSENGER_SEATS = APC['PASSENGER_SEATS']


def escorts(level):
    return [row for row in level['descriptors'] if row['spawn_type'] == 9]


def source_properties(source):
    properties = {}
    keys = ('properties_member', 'properties_sha256', 'placement_y_offset', 'health_quarter_points')
    for level in source['maps']:
        rows = escorts(level)
        require(len(rows) == 4 and all(token(row['vehicle_type']) == ESCORT_TYPE for row in rows),
                'wrong original Escort source inventory')
        for row in rows:
            variants = indexed(row['variants'], 'faction')
            require(variants.keys() == set(FACTIONS), 'unbound Escort source factions')
            for label, variant in variants.items():
                property_values = tuple(variant[key] for key in keys)
                require(properties.setdefault(label, property_values) == property_values,
                        'inconsistent original Escort properties')
    require(properties.keys() == set(FACTIONS), 'missing original Escort properties')
    return properties


def named_pose(model, model_token):
    require(model['name_token'] == model_token, 'wrong Escort render model identity')
    nodes = {node.node_id: node for node in model['nodes']}
    require(len(nodes) == len(model['nodes']) and {0, 1} <= nodes.keys(),
            'incomplete or duplicate Escort model nodes')

    def named(name):
        matches = [node for node in nodes.values() if node.name_token == name_token(name)]
        require(len(matches) == 1, f'missing or ambiguous named Escort node {name}')
        return matches[0]

    def prove(node):
        transform = model['reference_transforms'][node.transform_index]
        return {'node_id': node.node_id, 'parent_node_id': node.parent_node_id,
                'name_token': node.name_token, 'transform_index': node.transform_index,
                'translation_xyz': tuple(transform.translation_xyz),
                'quaternion_xyzw': tuple(transform.quaternion_xyzw), 'scale': transform.uniform_scale}

    seat = named('turret_01')
    yaw, pitch = named('turret_yaw'), named('turret_pitch')
    root, chassis = nodes[0], nodes[1]
    require(root.parent_node_id == 0xffff and chassis.parent_node_id == root.node_id
            and chassis.name_token == name_token('chassis') and seat.parent_node_id == chassis.node_id,
            'wrong original Escort seat parent chain')
    require(yaw.parent_node_id == seat.node_id and pitch.parent_node_id == yaw.node_id,
            'wrong original Escort turret pivot chain')
    chain = [prove(seat), prove(chassis), prove(root)]
    require(all(row['quaternion_xyzw'] == (0., 0., 0., 1.) and row['scale'] == 1. for row in chain)
            and all(row['translation_xyz'] == (0., 0., 0.) for row in chain[1:]),
            'original Escort seat parents must transmit the named reference offset unchanged')
    local_position = [f32(value) for value in chain[0]['translation_xyz']]
    require(any(local_position) and all(map(math.isfinite, local_position)),
            'original Escort seat requires a finite nonzero reference offset')
    return local_position, {'model_token': model_token, 'model_header_sha256': model['source_sha256'],
                            'seat_chain': chain, 'aim_nodes': [prove(yaw), prove(pitch)]}


def project_variants(source, catalog, ids, read_game, load_models):
    original = source_properties(source)
    parse_zpr = MAP['parse_zpr']
    registry_raw = read_game(MAP['VEHICLES_MEMBER'])
    registry = MAP['vehicle_registry'](parse_zpr(registry_raw.decode()))[f'0x{ESCORT_TYPE:08x}']
    require(registry['type'] == 'Escort', 'original Escort registry type differs')
    by_faction = indexed([row for row in registry['variants'] if row['ModifierLevel'] == '0'], 'faction')
    require(by_faction.keys() == set(FACTIONS), 'missing original Escort variants')
    model_tokens = [name_token(by_faction[name]['ModelFileName']) for name in FACTIONS]
    require(len(set(model_tokens)) == len(FACTIONS), 'duplicate original Escort render model')
    models = load_models(model_tokens)
    require(models.keys() == set(model_tokens), 'incomplete original Escort render model coverage')
    variants, poses, equipment = [], [], []
    for faction, label in enumerate(FACTIONS):
        row = by_faction[label]
        filename = row['PropertiesFileName']
        member, property_hash, offset, hp = original[label]
        require(row['Class'] == 'VehicleClass_Wheeled' and Path(filename).name == filename
                and filename.endswith('.zpr') and member == f'/run/mag/ps3/scripts/vehicle/{filename}',
                f'wrong original Escort property identity for {label}')
        raw = read_game(member)
        require(sha(raw) == property_hash, f'original Escort property digest differs for {label}')
        props = parse_zpr(raw.decode())['GroundVehicleProperties']
        seats = props['Seats']
        physical = []
        for ordinal, (key, wrapper) in enumerate(seats['SeatPropertiesList'].items()):
            require(key == f'SeatProperties{ordinal}', 'invalid original Escort physical seat order')
            authored = wrapper['SeatProperties']
            count = APC['GROUND']['APC']['property_integer'](
                authored.get('SeatCount', '1'), 'SeatCount', 1, 17)
            require(len(physical) + count <= 16, 'original Escort physical seat count exceeds bound')
            physical.extend([authored] * count)
        require(seats['MaxOccupancy'] == '8' and len(physical) == 10
                and physical[0]['SeatType'] == 'VehicleOperatorSeat'
                and physical[1] == {'Name': 'turret_01', 'SeatType': 'TurretOperatorSeat',
                                    'occupantIsVisible': 'false', 'TurretIndex': '0'}
                and [i for i, seat in enumerate(physical) if seat['SeatType'] == 'PassengerSeat']
                == PASSENGER_SEATS and f32(float(props['Placement_Y_Offset'])) == offset
                and float(props['DamageProperties']['MaxHitpoints']) * 4 == hp,
                f'wrong original Escort physical roles or properties for {label}')
        turrets = props['Turrets']['TurretProps']
        turret = turrets['TurretProperties0']['TurretProperties']
        require(list(turrets) == ['TurretProperties0'] and turret['Weapon1'] and turret['Weapon2'] == '',
                f'expected original Escort primary and absent secondary for {label}')
        weapon = APC['weapon_binding'](catalog, ids, faction, turret['Weapon1'], vehicle='Escort')
        model_token = model_tokens[faction]
        model_member, model_hash, model = models[model_token]
        local_position, pose = named_pose(model, model_token)
        variants.append({'faction': faction, 'primary_item_id': weapon['wire_id'], 'seat_node': 'turret_01',
                         'seat_index': 1, 'turret_index': 0, 'occupant_visible': False,
                         'local_position_xyz': local_position})
        poses.append(dict(pose, faction=faction, model_member=model_member, model_member_sha256=model_hash,
                          properties_member=member, properties_sha256=property_hash))
        equipment.append(weapon)
    return variants, poses, equipment, ESCORT_TYPE


def project_profiles(source, pairs, owners, registry_hash, ground_binding):
    levels = indexed(source['maps'], 'map')
    require(len(levels) == 3 and levels.keys() == owners.keys() and set(owners.values()) == {0, 1, 2},
            'expected three original Acquisition source levels')
    expected = {(name, faction) for name, owner in owners.items() for faction in range(3) if faction != owner}
    require(len(pairs) == 6, 'expected six Acquisition spawn/Ground profile pairs')
    profiles, seen = [], set()
    for spawn_raw, ground_raw in pairs:
        spawns, ground = map(parse, (spawn_raw, ground_raw))
        pool, = spawns['maps']
        name = spawns['active']['map_name']
        policy = spawns['match_profile']
        attacker, owner = policy['attacking_faction'], pool['owner_faction']
        key = (name, attacker)
        require(type(attacker) is int and type(owner) is int and key in expected and key not in seen,
                'duplicate or invalid Acquisition Escort profile')
        seen.add(key)
        level = levels[name]
        provenance = parse(spawns['source'])
        require(spawns['format'] == 'mag_spawn_catalog' and spawns['schema_version'] == 1
                and policy['game_type'] == 1 and policy['auto_assign_teams'] is True
                and policy['has_bunkers'] is False and policy['has_objectives'] is True
                and owner == owners[name] and pool['name'] == provenance['map'] == name
                and pool['runtime_indices_verified'] is True
                and provenance['game_archive_sha256'] == GAME_ARCHIVE_SHA256
                and provenance['native_elf_sha256'] == RETAIL_ELF_SHA256
                and provenance['native_binding_sha256'] == source['initial_bindings_proof_sha256']
                and provenance['level_archive_sha256'] == level['source']['archive_sha256']
                and provenance['level_clv_sha256'] == level['source']['clv_sha256'],
                f'unbound original Escort spawn profile for {name}')
        ground_source, visibility = ground['source'], ground['seat_visibility']
        spawn_hash = sha(spawn_raw)
        require(ground['format'] == 'mag_ground_vehicles' and ground['schema_version'] == 1
                and ground_source['creation_scope'] == 'native_vehicle_source_points'
                and ground_source['spawn_catalog_sha256'] == spawn_hash
                and ground_source['native_elf_sha256'] == RETAIL_ELF_SHA256
                and ground_source['native_binding_sha256'] == ground_binding
                and visibility['archive_sha256'] == GAME_ARCHIVE_SHA256
                and visibility['registry_member_sha256'] == registry_hash,
                f'unbound original Escort Ground profile for {name}')
        rows = project_placements(level, ground, pool, attacker, owner)
        profiles.append({'map_name': name, 'spawn_catalog_sha256': spawn_hash,
                         'ground_catalog_sha256': sha(ground_raw), 'vehicles': rows})
    require(seen == expected, 'incomplete Acquisition Escort profile coverage')
    return sorted(profiles, key=lambda row: (row['map_name'], row['spawn_catalog_sha256']))


def project_placements(level, ground, pool, attacker, owner):
    original = indexed(level['descriptors'], 'owned_selected_vehicle_index')
    bindings = indexed(ground['vehicle_sources'], 'vehicle_index')
    vehicles = indexed(ground['vehicles'], 'index')
    drivers = indexed(ground['driver_seats'], 'vehicle_index')
    passenger_spawns = indexed(ground['passenger_spawns'], 'vehicle_index')
    visibility = indexed(ground['seat_visibility']['vehicles'], 'vehicle_index')
    expected_escorts = {idx for idx, row in original.items() if row['spawn_type'] == 9}
    require(vehicles.keys() == bindings.keys() == set(range(16))
            and original.keys() == set(range(4, 16)) and expected_escorts == set(range(4, 8))
            and drivers.keys() == expected_escorts and passenger_spawns.keys() == set(range(4))
            and visibility.keys() == set(range(8)), 'wrong original or admitted Escort Ground roster')
    points = indexed(pool['locations'], 'index')
    require(len({row['spawn_point_index'] for row in bindings.values()}) == len(bindings)
            and len({row['spawn_point_token'] for row in bindings.values()}) == len(bindings),
            'duplicate original Escort Ground source point')
    for index, passenger in passenger_spawns.items():
        binding, vehicle = bindings[index], vehicles[index]
        fields = ('spawn_point_index', 'spawn_point_token')
        point = points[passenger['spawn_point_index']]
        require(all(passenger[field] == binding[field] == vehicle[field] for field in fields)
                and point['kind'] == 'vehicle_passenger' and point['active'] is True
                and point['token'] == passenger['spawn_point_token']
                and binding['vehicle_type_token'] == vehicle['vehicle_type_token'] == APC['APC_TYPE']
                and binding['spawn_type'] == 3 and binding['authored_active'] is True
                and binding['source_family'] == 35 and binding['user_class'] == 0
                and binding['team'] == vehicle['team'] == 0 and vehicle['faction'] == attacker,
                'Escort aliases a passenger reinforcement APC')

    rows = []
    for index, authored in sorted(original.items()):
        point_index = authored['source_point_index']
        point_token = token(authored['source_point_token'])
        type_token = token(authored['vehicle_type'])
        is_escort = index in expected_escorts
        state = 0 if is_escort else 2
        spawn_type = 9 if is_escort else 7
        active = not is_escort
        team = 0 if is_escort else 1
        faction = attacker if is_escort else owner
        vehicle, point = vehicles[index], points[point_index]
        require(authored['spawn_type'] == spawn_type and authored['team'] == team
                and authored['initial_network_state'] == state and authored['authored_source_active'] is active
                and authored['source_point_family'] == 35
                and token(authored['source_point_object_id']) == (0x23800000 | point_index)
                and authored['ground_snap'] is False and authored['radius_evidence']['raw_hex'] == '00000000'
                and authored['radius_evidence']['value'] == 0.
                and (type_token == ESCORT_TYPE) == is_escort
                and bindings[index] == {'vehicle_index': index, 'spawn_point_index': point_index,
                                        'spawn_point_token': point_token, 'vehicle_type_token': type_token,
                                        'spawn_type': spawn_type, 'source_family': 35, 'user_class': 1,
                                        'team': team, 'authored_active': active}
                and (vehicle['spawn_point_index'], vehicle['spawn_point_token'], vehicle['vehicle_type_token'])
                == (point_index, point_token, type_token)
                and vehicle['team'] == team and vehicle['faction'] == faction
                and vehicle['modifier'] == 0 and vehicle['initial_state'] == state
                and point['kind'] == 'unsupported' and point['token'] == point_token
                and point['active'] is active,
                f'Escort source identity differs at allocation {index}')
        variant = indexed(authored['variants'], 'faction')[FACTIONS[faction]]
        require(point['position'] == dict(zip('xyz', authored['source_position_xyz']))
                and vehicle['position'] == dict(zip('xyz', variant['initial_creation_position_xyz']))
                and variant['initial_creation_position_xyz'] == variant['respawn_position_xyz']
                and vehicle['rotation'] == authored['rotation_xyzw_source_convention']
                and vehicle['health_quarter_points'] == variant['health_quarter_points'],
                f'Escort source pose or health differs at allocation {index}')
        if not is_escort:
            require(not vehicle.get('passenger_seats'), 'motor pool aliases admitted Escort seats')
            continue
        occupant = visibility[index]
        require(drivers[index]['seat'] == 0 and vehicle['passenger_seats'] == PASSENGER_SEATS
                and vehicle['maximum_occupancy'] == 8
                and occupant['properties_member_sha256'] == variant['properties_sha256']
                and [row['seat'] for row in occupant['seats']] == [0, *PASSENGER_SEATS],
                f'Escort seat admission differs at allocation {index}')
        rows.append({'vehicle_index': index, 'spawn_point_index': point_index,
                     'spawn_point_token': point_token, 'faction': attacker})
    return rows


def original_models(archive, tokens):
    from rigid_hierarchy import parse_rigid_resource
    from zrb_records import parse as parse_zrb

    names = {f'{token:08x}': token for token in tokens}
    models = {}
    for member in archive.names:
        if not re.fullmatch(r'/run/mag/ps3/global_resources/vehicles_shared_[0-9]+\.zrb', member):
            continue
        raw = COMMON['archive_member'](archive, member)
        records = parse_zrb(raw, retail13_compat=True)['records']
        for record in records:
            if record['class_token'] != 'a4f4d23c' or record['name_token'] not in names:
                continue
            token = names[record['name_token']]
            require(token not in models, 'ambiguous original Escort model member')
            models[token] = (member, sha(raw), parse_rigid_resource(raw, record))
    return models


def project(source_raw, catalog_raw, binding_raw, pairs, read_game, load_models, native):
    source = parse(source_raw)
    source_hash = sha(source_raw)
    require(source_hash == SOURCE_POSES_SHA256 and source['schema'] == 'mag_acquisition_vehicle_source_pose_inputs_v2'
            and source['native_elf_sha256'] == native['retail_elf_sha256'] == RETAIL_ELF_SHA256,
            'wrong original Acquisition Escort source poses')
    population = sha((source['initial_bindings_proof_sha256'] + source_hash).encode())
    ground_binding = sha((population + VEHICLE_WAVES_SHA256).encode())
    levels = runpy.run_path(str(HERE / 'extract-retail-spawns.py'))['catalog_settings'](
        read_game('/run/mag/ps3/settings/gameproperties.zpr').decode())
    owners = {name: FACTIONS.index(row['owner_faction']) for name, row in
              indexed([row for row in levels if row['mode_enum'] == 'GameType128'], 'map').items()
              if name in {level['map'] for level in source['maps']}}
    registry_hash = sha(read_game(MAP['VEHICLES_MEMBER']))
    catalog, ids, item_members = COMMON['require_item_binding'](catalog_raw, binding_raw, read_game)
    variants, models, equipment, vehicle_type = project_variants(source, catalog, ids, read_game, load_models)
    profiles = project_profiles(source, pairs, owners, registry_hash, ground_binding)
    visibility = runpy.run_path(str(HERE / 'project-retail-ground-seat-visibility.py'))['project']
    for _, ground_raw in pairs:
        ground = parse(ground_raw)
        retained = ground.pop('seat_visibility')
        require(visibility(ground, GAME_ARCHIVE_SHA256, read_game)['seat_visibility'] == retained,
                'Escort profile visibility differs from the original admitted seats')

    pose = {'scope': 'retail_named_escort_seat_reference', 'models': models,
            'ground_source_sha256': source_hash, 'retail_main_sha256': MAIN_ARCHIVE_SHA256,
            'seat_lookup_span': native['spans']['named_seat_lookup']}
    wire = APC['wire_proof'](native, equipment, item_members, sha(binding_raw))
    rules = {'schema': 'mag_acquisition_escort_turrets_v1',
             'source': {'item_catalog_sha256': sha(catalog_raw), 'retail_elf_sha256': RETAIL_ELF_SHA256,
                        'retail_archive_sha256': GAME_ARCHIVE_SHA256, 'retail_main_sha256': MAIN_ARCHIVE_SHA256,
                        'pose_proof_sha256': sha(encode(pose)), 'wire_proof_sha256': sha(encode(wire)),
                        'vehicle_type_token': vehicle_type},
             'variants': variants, 'profiles': profiles}
    proof = {'format': 'mag_retail_acquisition_escort_turret_proof_v1', 'ground_sources_sha256': source_hash,
             'vehicle_registry_sha256': registry_hash, 'ground_binding_sha256': ground_binding,
             'pose': pose, 'wire': wire, 'profiles': profiles}
    return rules, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('elf', 'game-archive', 'main-archive', 'ground-sources', 'item-catalog',
                 'item-bindings', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--profile', metavar=('SPAWNS', 'GROUND'), type=Path, nargs=2,
                        action='append', required=True, help='repeat once per Acquisition profile')
    parser.add_argument('--proof-output', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [args.elf, args.game_archive, args.main_archive, args.ground_sources,
              args.item_catalog, args.item_bindings, *(path for pair in args.profile for path in pair)]
    outputs = [args.output] + ([args.proof_output] if args.proof_output else [])
    try:
        COMMON['output_paths'](inputs, outputs)
        require(len(args.profile) == 6, 'expected six Acquisition spawn/Ground profile pairs')
        elf = NATIVE['Elf'](args.elf, 'retail')
        game_raw, main_raw = args.game_archive.read_bytes(), args.main_archive.read_bytes()
        require(sha(game_raw) == GAME_ARCHIVE_SHA256 and sha(main_raw) == MAIN_ARCHIVE_SHA256,
                'wrong original retail game or main archive')
        game, archive = map(NATIVE['Psarc'], (game_raw, main_raw))
        rules, proof = project(
            args.ground_sources.read_bytes(), args.item_catalog.read_bytes(), args.item_bindings.read_bytes(),
            [(spawns.read_bytes(), ground.read_bytes()) for spawns, ground in args.profile],
            lambda name: COMMON['archive_member'](game, name), lambda tokens: original_models(archive, tokens),
            APC['native_witness'](elf))
        rendered = [(args.output, encode(rules))]
        if args.proof_output:
            rendered.append((args.proof_output, encode(proof)))
        COMMON['write_outputs'](inputs, rendered, check=args.check)
    except (KeyError, OSError, TypeError, ValueError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')
    print('Verified 6 retail Acquisition profiles, 24 Escort turret placements and 3 authored primaries')


if __name__ == '__main__':
    main()
