#!/usr/bin/env python3
"""Project source-qualified retail Domination APC gunner rules offline."""
import argparse
from pathlib import Path
import runpy


HERE = Path(__file__).resolve().parent
COMMON = runpy.run_path(str(HERE / 'project-retail-hospital-turrets.py'))
GROUND = runpy.run_path(str(HERE / 'project-retail-domination-ground.py'))
NATIVE, MAP, FACTIONS, SQUADS = (COMMON[name] for name in ('NATIVE', 'MAP', 'FACTIONS', 'SQUADS'))
sha, encode, parse, require, name_token, indexed = (
    COMMON[name] for name in ('sha', 'encode', 'parse', 'require', 'name_token', 'indexed'))
digest, token, f32 = COMMON['digest'], GROUND['token'], GROUND['f32']
RETAIL_ELF_SHA256 = COMMON['RETAIL_ELF_SHA256']
GAME_ARCHIVE_SHA256 = COMMON['GAME_ARCHIVE_SHA256']
MAIN_ARCHIVE_SHA256 = COMMON['MAIN_ARCHIVE_SHA256']
APC_TYPE = name_token('APC')
PASSENGER_SEATS = list(range(2, 10))


def native_witness(elf):
    ranges = {
        'ground_fields': (0x44a85c, 0x44b0e0),
        'inherited_vehicle_fields': (0x445e9c, 0x446478),
        'common_markup': (0x43fc18, 0x43fd88),
        'client_allocations': (0x71d58, 0x71d80),
        'server_allocations': (0x909078, 0x909098),
        'gunner_owner_selection': (0x872bf0, 0x872d7c),
        'turret_owner_setter': (0x43d42c, 0x43d530),
        'named_seat_lookup': (0x701300, 0x701408),
        'named_seat_dispatch': (0x701a98, 0x701ad8),
        'primary_inventory_setter': (0x87c2a0, 0x87c300),
        'empty_inventory_init': (0xfcd488, 0xfcd4cc),
        'equipped_version_gate': (0x6dbeb8, 0x6dc088),
        'mounted_shooter_item': (0x7c7c14, 0x7c7ef0),
        'mounted_gear_slot': (0x86df20, 0x86df88),
    }
    branches = {0x44a8a0: 0x445e9c, 0x872d74: 0xe57620, 0xe5762c: 0x43d42c,
                0x87c2fc: 0xe57760, 0xe5776c: 0x43b2a8, 0x71d7c: 0xe4aa20,
                0x909094: 0xe58400, 0x7c7e30: 0x86fab4}
    for address, target in branches.items():
        instruction = elf.word(address)
        displacement = instruction & 0x03fffffc
        if displacement & 0x02000000:
            displacement -= 0x04000000
        require(instruction >> 26 == 18 and instruction & 2 == 0
                and address + displacement == target, f'original APC branch differs at {address:#x}')
    spans = {name: {'start': hex(start), 'end_exclusive': hex(end),
                    'sha256': sha(elf.read(start, end - start)[0])}
             for name, (start, end) in ranges.items()}
    return {'retail_elf_sha256': sha(elf.raw), 'spans': spans,
            'branches': {hex(address): hex(target) for address, target in branches.items()}}


def weapon_binding(catalog, ids, faction, name, vehicle='APC'):
    namespace = f'FactionItemList{faction + 1}:'
    refs = [ref for ref, item in catalog['items'].items()
            if ref.startswith(namespace) and item['name'] == name]
    require(len(refs) == 1, f'missing or ambiguous original {vehicle} primary item')
    ref = refs[0]
    item = catalog['items'][ref]
    require(item['authored_ref'] == ref and item['item_type'] == 'kWeapon'
            and item['definition'] == {'group': 'weapons', 'name': name}
            and 0 < ids[ref] < 4096, f'wrong original {vehicle} item identity')
    definitions = catalog['definitions']
    weapon = definitions['weapons'][name]['parameters']
    ammo_name = weapon['ammo_name']
    modes = weapon['fire_mode']['fire_modes']
    require(weapon['category'] == 'kWepCategoryVehicleWeapon'
            and weapon['initial_fire_mode'] == 'kAuto'
            and list(modes) == ['FireModeTypeClass0']
            and modes['FireModeTypeClass0']['FireModeTypeClass']['fire_mode_value'] == 'kAuto'
            and ammo_name in definitions['hitscan_ammo'], f'wrong original {vehicle} primary hitscan mode')
    return {'faction': faction, 'name': name, 'authored_ref': ref, 'wire_id': ids[ref],
            'ammo_name': ammo_name, 'ammo_group': 'hitscan_ammo',
            'definition_slots': [ids[ref], None, None]}


def named_pose(raw, model_token, decode_model):
    model = decode_model(raw, model_token)
    require(model['name_token'] == model_token, 'wrong APC render model identity')
    nodes = {node.node_id: node for node in model['nodes']}
    require(len(nodes) == len(model['nodes']) and {0, 2, 3, 4} <= nodes.keys(),
            'incomplete or duplicate APC model nodes')
    seat = [node for node in nodes.values() if node.name_token == name_token('turret_01')]
    require(len(seat) == 1 and seat[0].node_id == 2, 'missing or ambiguous named APC turret seat')

    def prove(node):
        transform = model['reference_transforms'][node.transform_index]
        return {'node_id': node.node_id, 'parent_node_id': node.parent_node_id,
                'name_token': node.name_token, 'transform_index': node.transform_index,
                'translation_xyz': tuple(transform.translation_xyz),
                'quaternion_xyzw': tuple(transform.quaternion_xyzw), 'scale': transform.uniform_scale}

    root = nodes[0]
    parent = seat[0].parent_node_id
    require(root.parent_node_id == 0xffff
            and (parent == 0 or (parent == 1 and 1 in nodes and nodes[1].parent_node_id == 0
                                and nodes[1].name_token == name_token('chassis'))),
            'wrong original APC seat parent chain')
    chain = [prove(seat[0]), *([prove(nodes[1])] if parent == 1 else []), prove(root)]
    require(all(row['quaternion_xyzw'] == (0., 0., 0., 1.) and row['scale'] == 1. for row in chain)
            and all(row['translation_xyz'] == (0., 0., 0.) for row in chain[1:]),
            'original APC seat parents must transmit the named reference offset unchanged')
    local_position = [f32(value) for value in chain[0]['translation_xyz']]
    require(any(local_position), 'original APC seat requires a nonzero reference offset')
    pivots = [prove(nodes[index]) for index in (3, 4)]
    require([(row['parent_node_id'], row['name_token']) for row in pivots]
            == [(2, name_token('turret_yaw')), (3, name_token('turret_pitch'))],
            'wrong original APC turret pivot chain')
    return local_position, {'model_token': model_token, 'model_header_sha256': model['source_sha256'],
                            'seat_chain': chain, 'aim_nodes': pivots}


def project_variants(source, catalog, ids, read_game, read_main, decode_model,
                     bind_weapon=weapon_binding):
    registry_raw = read_game(MAP['VEHICLES_MEMBER'])
    require(sha(registry_raw) == source['original_registry_sha256'], 'wrong original APC registry digest')
    parse_zpr = MAP['parse_zpr']
    registry = MAP['vehicle_registry'](parse_zpr(registry_raw.decode()))[f'0x{APC_TYPE:08x}']
    require(registry['type'] == 'APC', 'original APC registry type differs')
    variants, poses, equipment = [], [], []
    for faction, name in enumerate(FACTIONS):
        row = source['variants'][f'3:{name}']
        matches = [v for v in registry['variants'] if v['faction'] == name and v['ModifierLevel'] == '0']
        require(len(matches) == 1 and matches[0]['Class'] == 'VehicleClass_Wheeled',
                f'expected a unique original wheeled APC variant for {name}')
        variant, = matches
        filename = variant['PropertiesFileName']
        member = f'/run/mag/ps3/scripts/vehicle/{filename}'
        require(row['type'] == 3 and row['faction'] == faction and Path(filename).name == filename
                and filename.endswith('.zpr') and member == row['source_member'],
                f'wrong archived APC property identity for {name}')
        raw = read_game(member)
        require(sha(raw) == row['source_sha256'], f'original APC property digest differs for {name}')
        props = parse_zpr(raw.decode())['GroundVehicleProperties']
        seats = props['Seats']
        physical = []
        for ordinal, (key, wrapper) in enumerate(seats['SeatPropertiesList'].items()):
            require(key == f'SeatProperties{ordinal}', 'invalid APC physical seat order')
            authored = wrapper['SeatProperties']
            count = GROUND['APC']['property_integer'](authored.get('SeatCount', '1'), 'SeatCount', 1, 17)
            require(len(physical) + count <= 16, 'original APC physical seat count exceeds bound')
            physical.extend([authored] * count)
        require(len(physical) == 10 and seats['MaxOccupancy'] == '8' and row['maximum_occupancy'] == 8
                and physical[0]['SeatType'] == 'VehicleOperatorSeat'
                and physical[1] == {'Name': 'turret_01', 'SeatType': 'TurretOperatorSeat',
                                    'occupantIsVisible': 'false', 'TurretIndex': '0'}
                and [i for i, seat in enumerate(physical) if seat['SeatType'] == 'PassengerSeat']
                == row['passenger_seats'] == PASSENGER_SEATS, f'wrong original APC physical roles for {name}')
        turrets = props['Turrets']['TurretProps']
        turret = turrets['TurretProperties0']['TurretProperties']
        require(list(turrets) == ['TurretProperties0'] and turret['Weapon1'] and turret['Weapon2'] == '',
                f'expected original primary and absent secondary for {name}')
        weapon = bind_weapon(catalog, ids, faction, turret['Weapon1'])
        body = row['body']
        model_raw = read_main(body['member'])
        require(sha(model_raw) == body['member_sha256'], f'wrong original APC model member for {name}')
        local_position, pose = named_pose(model_raw, name_token(variant['ModelFileName']), decode_model)
        variants.append({'faction': faction, 'primary_item_id': weapon['wire_id'], 'seat_node': 'turret_01',
                         'seat_index': 1, 'turret_index': 0, 'occupant_visible': False,
                         'local_position_xyz': local_position})
        poses.append(dict(pose, faction=faction, model_member=body['member'],
                          model_member_sha256=body['member_sha256'], properties_member=member,
                          properties_sha256=row['source_sha256']))
        equipment.append(weapon)
    return variants, poses, equipment, APC_TYPE


def project_profiles(source, source_hash, pairs, vehicle_type):
    levels = indexed(source['maps'], 'map')
    require(len(levels) == 3 and {level['owner_faction'] for level in levels.values()} == set(FACTIONS),
            'expected three original Domination level sources')
    expected = {(name, faction) for name, level in levels.items() for faction in range(3)
                if FACTIONS[faction] != level['owner_faction']}
    require(len(pairs) == 6, 'expected six Domination spawn/Ground profile pairs')
    profiles, seen = [], set()
    for spawn_raw, ground_raw in pairs:
        spawns, ground = map(parse, (spawn_raw, ground_raw))
        pool, = spawns['maps']
        name = spawns['active']['map_name']
        policy = spawns['match_profile']
        attacker, owner = policy['attacking_faction'], pool['owner_faction']
        key = (name, attacker)
        require(type(attacker) is int and type(owner) is int and key in expected and key not in seen,
                'duplicate or invalid Domination APC profile')
        seen.add(key)
        level = levels[name]
        provenance = parse(spawns['source'])
        require(spawns['format'] == 'mag_spawn_catalog' and spawns['schema_version'] == 1
                and policy['game_type'] == 2 and policy['auto_assign_teams'] is True
                and policy['has_bunkers'] is True and owner == FACTIONS.index(level['owner_faction'])
                and pool['name'] == provenance['map'] == name and pool['runtime_indices_verified'] is True
                and provenance['game_archive_sha256'] == GAME_ARCHIVE_SHA256
                and provenance['native_spawn_bindings_sha256'] == source['spawn_bindings_sha256']
                and provenance['domination_ground_sources_sha256'] == source_hash
                and provenance['level_archive_sha256'] == level['source']['archive_sha256']
                and provenance['level_clv_sha256'] == level['source']['clv_sha256'],
                f'unbound original APC spawn profile for {name}')
        ground_source = ground['source']
        visibility = ground['seat_visibility']
        spawn_hash = sha(spawn_raw)
        require(ground['format'] == 'mag_ground_vehicles' and ground['schema_version'] == 1
                and ground_source['creation_scope'] == GROUND['SCHEMA']
                and ground_source['spawn_catalog_sha256'] == spawn_hash
                and ground_source['native_elf_sha256'] == RETAIL_ELF_SHA256
                and ground_source['native_binding_sha256'] == source_hash
                and visibility['archive_sha256'] == GAME_ARCHIVE_SHA256
                and visibility['registry_member_sha256'] == source['original_registry_sha256'],
                f'unbound original APC Ground profile for {name}')
        original = indexed(level['rows'], 'object_index')
        bindings = indexed(ground['vehicle_sources'], 'vehicle_index')
        vehicles = indexed(ground['vehicles'], 'index')
        drivers = indexed(ground['driver_seats'], 'vehicle_index')
        passenger_spawns = indexed(ground['passenger_spawns'], 'vehicle_index')
        occupant_visibility = indexed(visibility['vehicles'], 'vehicle_index')
        expected_apcs = {i for i, row in original.items() if row['spawn_type'] == 3}
        require(vehicles.keys() == bindings.keys() == original.keys() == set(range(16))
                and len(expected_apcs) == 8
                and drivers.keys() == passenger_spawns.keys() == occupant_visibility.keys() == expected_apcs,
                f'wrong original or admitted APC ground roster for {name}')
        points = indexed(pool['locations'], 'index')
        squads = indexed(pool['squads'], 'index')
        variant = source['variants'][f'3:{FACTIONS[attacker]}']
        rows = []
        for index, authored in sorted(original.items()):
            point_index = authored['spawn_point_index']
            point_token = token(authored['spawn_point_token'])
            source_type = authored['spawn_type']
            type_token = token(authored['vehicle_type_token'])
            binding = bindings[index]
            vehicle = vehicles[index]
            require(source_type in (3, 7) and authored['source_family'] == 35
                    and authored['authored_active'] is True and binding == {
                        key: authored[key] for key in ('spawn_type', 'team', 'user_class',
                                                      'source_family', 'authored_active')
                    } | {'vehicle_index': index, 'spawn_point_index': point_index,
                         'spawn_point_token': point_token, 'vehicle_type_token': type_token}
                    and (vehicle['spawn_point_index'], vehicle['spawn_point_token'], vehicle['vehicle_type_token'])
                    == (point_index, point_token, type_token), f'APC source identity differs at allocation {index}')
            if source_type != 3:
                require(type_token != vehicle_type and not vehicle.get('passenger_seats'),
                        'APC class or admitted seats alias a motor-pool source')
                continue
            link, = authored['squad_links']
            address = link['platoon_id'] * 4 + SQUADS.index(link['squad'])
            squad = squads[address]
            point = points[point_index]
            visibility_row = occupant_visibility[index]
            transform = authored['source_transform']
            origin = [f32(value) for value in transform['position_xyz']]
            origin[1] = f32(origin[1] + variant['placement_y_offset'])
            rotation = [f32(value) for value in GROUND['quaternion'](transform['rotation_xyzw'])]
            require(type_token == vehicle_type and type(vehicle['faction']) is int and vehicle['faction'] == attacker
                    and vehicle['team'] == authored['team'] == link['team'] == link['slot'] == 0
                    and vehicle['position'] == GROUND['position'](origin) and vehicle['rotation'] == rotation
                    and vehicle['modifier'] == 0 and drivers[index]['seat'] == 0
                    and vehicle['passenger_seats'] == PASSENGER_SEATS and vehicle['maximum_occupancy'] == 8
                    and passenger_spawns[index] == {
                        'vehicle_index': index, 'spawn_point_index': point_index, 'spawn_point_token': point_token}
                    and visibility_row['properties_member_sha256'] == variant['source_sha256']
                    and [row['seat'] for row in visibility_row['seats']] == [0, *PASSENGER_SEATS]
                    and point['kind'] == 'vehicle_passenger' and point['active'] is True
                    and point['token'] == point_token and squad['team'] == 0
                    and squad['default_spawn'] == point_index and squad['available'][0] == point_index,
                    f'APC passenger, driver or spawn identity differs at allocation {index}')
            rows.append({'vehicle_index': index, 'spawn_point_index': point_index,
                         'spawn_point_token': point_token, 'faction': attacker})
        profiles.append({'map_name': name, 'spawn_catalog_sha256': spawn_hash,
                         'ground_catalog_sha256': sha(ground_raw), 'vehicles': rows})
    require(seen == expected, 'incomplete Domination APC profile coverage')
    return sorted(profiles, key=lambda row: (row['map_name'], row['spawn_catalog_sha256']))


def wire_proof(native, equipment, item_members, binding_hash):
    return {'native': native, 'family': 1, 'allocations': 32, 'row_fields': 30, 'index_bits': 16,
            'owner_fields': [1, 6], 'owner_bits': 16, 'yaw_field_bits': [8, 11],
            'pitch_field_bits': [9, 10], 'overheat_field_bits': [21, 8],
            'inventory_definition': {'field': 10, 'version_bits': 8, 'id_bits': 12, 'slots': 3,
                                     'first_populated_version': 2},
            'equipment': {'fields': [11, 12], 'version_bits': 8, 'gear_bits': 5,
                          'ammo_pool_bits': 2, 'ammo_count_bits': 8, 'reloading_bits': 1},
            'dynamics_fields': [25, 26, 27, 28, 29],
            'mounted_primary': {'gear': 0, 'pool': 0, 'mode': 0, 'variants': equipment},
            'item_members_sha256': item_members, 'item_bindings_sha256': binding_hash}


def project(source_raw, catalog_raw, binding_raw, pairs, read_game, read_main, decode_model, native):
    source = parse(source_raw)
    require(source['format'] == 'mag_domination_ground_sources_v1'
            and source['allocation_scope'] == 'successful_production_fresh_level_load_type1'
            and source['native_elf_sha256'] == native['retail_elf_sha256'] == RETAIL_ELF_SHA256
            and source['original_game_sha256'] == GAME_ARCHIVE_SHA256
            and source['original_main_sha256'] == MAIN_ARCHIVE_SHA256
            and digest(source['spawn_bindings_sha256']), 'wrong original Domination APC source')
    catalog, ids, item_members = COMMON['require_item_binding'](catalog_raw, binding_raw, read_game)
    variants, models, equipment, vehicle_type = project_variants(
        source, catalog, ids, read_game, read_main, decode_model)
    source_hash = sha(source_raw)
    profiles = project_profiles(source, source_hash, pairs, vehicle_type)
    pose = {'scope': 'retail_named_apc_seat_reference', 'models': models,
            'ground_source_sha256': source_hash, 'retail_main_sha256': MAIN_ARCHIVE_SHA256,
            'seat_lookup_span': native['spans']['named_seat_lookup']}
    wire = wire_proof(native, equipment, item_members, sha(binding_raw))
    rules = {'schema': 'mag_domination_apc_turrets_v1',
             'source': {'item_catalog_sha256': sha(catalog_raw), 'retail_elf_sha256': RETAIL_ELF_SHA256,
                        'retail_archive_sha256': GAME_ARCHIVE_SHA256, 'retail_main_sha256': MAIN_ARCHIVE_SHA256,
                        'pose_proof_sha256': sha(encode(pose)), 'wire_proof_sha256': sha(encode(wire)),
                        'vehicle_type_token': vehicle_type},
             'variants': variants, 'profiles': profiles}
    proof = {'format': 'mag_retail_domination_apc_turret_proof_v1', 'ground_sources_sha256': source_hash,
             'vehicle_registry_sha256': source['original_registry_sha256'], 'pose': pose, 'wire': wire,
             'profiles': profiles}
    return rules, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('elf', 'game-archive', 'main-archive', 'ground-sources', 'item-catalog',
                 'item-bindings', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--profile', metavar=('SPAWNS', 'GROUND'), type=Path, nargs=2,
                        action='append', required=True, help='repeat once per Domination profile')
    parser.add_argument('--proof-output', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [args.elf, args.game_archive, args.main_archive, args.ground_sources,
              args.item_catalog, args.item_bindings, *(path for pair in args.profile for path in pair)]
    outputs = [args.output] + ([args.proof_output] if args.proof_output else [])
    COMMON['output_paths'](inputs, outputs)
    require(len(args.profile) == 6, 'expected six Domination spawn/Ground profile pairs')
    elf = NATIVE['Elf'](args.elf, 'retail')
    game_raw, main_raw = args.game_archive.read_bytes(), args.main_archive.read_bytes()
    require(sha(game_raw) == GAME_ARCHIVE_SHA256 and sha(main_raw) == MAIN_ARCHIVE_SHA256,
            'wrong original retail game or main archive')
    game, archive = map(NATIVE['Psarc'], (game_raw, main_raw))
    rules, proof = project(
        args.ground_sources.read_bytes(), args.item_catalog.read_bytes(), args.item_bindings.read_bytes(),
        [(spawns.read_bytes(), ground.read_bytes()) for spawns, ground in args.profile],
        lambda name: COMMON['archive_member'](game, name), lambda name: COMMON['archive_member'](archive, name),
        COMMON['read_model'], native_witness(elf))
    rendered = [(args.output, encode(rules))]
    if args.proof_output:
        rendered.append((args.proof_output, encode(proof)))
    COMMON['write_outputs'](inputs, rendered, check=args.check)
    print('Verified 6 retail Domination profiles, 48 APC turret placements and 3 authored primaries')


if __name__ == '__main__':
    main()
