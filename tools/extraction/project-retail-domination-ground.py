#!/usr/bin/env python3
"""Project the source-bound APC and motor-pool ground world for one Domination profile."""
import argparse
import json
from pathlib import Path
import runpy

APC = runpy.run_path(str(Path(__file__).with_name('project-retail-apc-insertion.py')))
SCHEMA = 'native_domination_vehicle_source_points'
FACTIONS = dict(enumerate(APC['FACTIONS']))
SQUADS = APC['SQUADS']
require = APC['require']
sha = APC['sha']
encode = APC['encode']
position = APC['position']
quaternion = APC['quaternion']
f32 = APC['f32']
token = APC['token']


def unique(values, key):
    result = {key(row): row for row in values}
    require(len(result) == len(values), 'Duplicate Domination ground source')
    return result


def apply_apc_driver_policy(ground, source_raw, archive_sha256, read_member):
    """Qualify admitted roles from original members; limits are replacement policy."""
    visibility = runpy.run_path(str(Path(__file__).with_name('project-retail-ground-seat-visibility.py')))
    source = json.loads(source_raw)
    require(archive_sha256 == source['original_game_sha256'] == APC['GAME_ARCHIVE_SHA256'],
            'Wrong original game archive for APC driver opt-in')
    require(ground['source']['creation_scope'] == SCHEMA
            and ground['source']['native_elf_sha256'] == source['native_elf_sha256']
            == APC['SUPPORTED_NATIVE_ELF_SHA256']
            and ground['source']['native_binding_sha256'] == sha(source_raw)
            and 'driver_seats' not in ground and 'seat_visibility' not in ground,
            'Unbound APC driver ground source')
    registry_raw = read_member(visibility['REGISTRY'])
    require(sha(registry_raw) == source['original_registry_sha256'], 'Wrong original vehicle registry')
    parse = visibility['HELPERS']['parse_zpr']
    registry = visibility['HELPERS']['vehicle_registry'](parse(registry_raw.decode()))
    vehicles = unique(ground['vehicles'], lambda row: row['index'])
    bindings = unique(ground['vehicle_sources'], lambda row: row['vehicle_index'])
    require(len(vehicles) == len(bindings) == 16 and vehicles.keys() == bindings.keys(),
            'Incomplete APC driver source inventory')
    apcs = {index for index, row in bindings.items() if row['spawn_type'] == 3}
    require(len(apcs) == 8 and all(row['spawn_type'] in (3, 7) for row in bindings.values()),
            'Require eight original APC driver sources')
    output, drivers = [], []
    for vehicle in ground['vehicles']:
        index = vehicle['index']
        if index not in apcs:
            require(not vehicle.get('passenger_seats'), 'Motor pool has admitted seats')
            output.append(dict(vehicle))
            continue
        faction = APC['integer'](vehicle['faction'], 'APC faction', 0, 3)
        original = source['variants'][f'3:{FACTIONS[faction]}']
        require(original['faction'] == faction and original['type'] == 3
                and vehicle['modifier'] == 0, 'Wrong APC faction or modifier')
        key = f"0x{token(vehicle['vehicle_type_token']):08x}"
        variants = [row for row in registry.get(key, {}).get('variants', [])
                    if row['faction'] == FACTIONS[faction] and row['ModifierLevel'] == '0']
        require(len(variants) == 1, 'Require unique original APC registry variant')
        variant = variants[0]
        require(variant.get('Class') == 'VehicleClass_Wheeled', 'Wrong original APC vehicle class')
        filename = variant['PropertiesFileName']
        require(Path(filename).name == filename and filename.endswith('.zpr'), 'Invalid APC property member')
        member = '/run/mag/ps3/scripts/vehicle/' + filename
        require(member == original['source_member'], 'Wrong original APC property member')
        raw = read_member(member)
        require(sha(raw) == original['source_sha256'], 'Wrong original APC property digest')
        metadata, placement = APC['vehicle_properties'](parse(raw.decode()))
        require(metadata['passenger_seats'] == original['passenger_seats']
                == vehicle['passenger_seats'] == list(range(2, 10))
                and metadata['maximum_occupancy'] == original['maximum_occupancy']
                == vehicle['maximum_occupancy'] == 8
                and metadata['health_quarter_points'] == original['health_quarter_points']
                == vehicle['health_quarter_points']
                and placement == original['placement_y_offset'], 'APC original properties differ')
        # Explicit conservative replacement limits, not extracted retail constants.
        output.append(dict(vehicle, maximum_mount_distance_m=8., maximum_mount_position_age_ms=750))
        drivers.append(dict(vehicle_index=index, seat=0, maximum_speed_mps=25.))
    result = dict(ground, vehicles=output, driver_seats=drivers)
    # Verify the original operator/passenger physical roles and occupant visibility
    # together, before any output is written. The gunner slot is never admitted.
    return visibility['project'](result, archive_sha256, read_member)


def driver_catalog(ground, source_raw, game_raw):
    digest = sha(game_raw)
    require(digest == APC['GAME_ARCHIVE_SHA256'], 'Wrong original game archive for APC driver opt-in')
    psarc = runpy.run_path(str(Path(__file__).with_name('extract-retail-spawns.py')))['Psarc'](game_raw)

    def read_member(name):
        require(psarc.names.count(name) == 1, 'Missing or ambiguous original vehicle member ' + name)
        return psarc.extract(psarc.names.index(name) + 1)

    return apply_apc_driver_policy(ground, source_raw, digest, read_member)


def project(spawn_raw, ground_raw, exits_raw, *, enable_apc_driver=False, game_raw=None):
    require(enable_apc_driver == (game_raw is not None), 'APC driver opt-in requires exactly one game archive')
    spawns, source, exits = map(json.loads, (spawn_raw, ground_raw, exits_raw))
    policy = spawns['match_profile']
    pool = spawns['maps'][0]
    name = pool['name']
    require(spawns['format'] == 'mag_spawn_catalog' and policy['game_type'] == 2
            and policy['auto_assign_teams'] is True and len(spawns['maps']) == 1
            and pool['runtime_indices_verified'] is True and name == spawns['active']['map_name'],
            'Require native Domination spawn profile')
    provenance = json.loads(spawns['source'])
    require(source['format'] == 'mag_domination_ground_sources_v1'
            and source['allocation_scope'] == 'successful_production_fresh_level_load_type1'
            and source['spawn_bindings_sha256'] == provenance['native_spawn_bindings_sha256']
            and source['original_game_sha256'] == provenance['game_archive_sha256']
            and exits['format'] == 'mag_domination_ground_exits_v1'
            and exits['ground_source_sha256'] == sha(ground_raw),
            'Unbound native ground allocation or exit source')
    map_ground = unique(source['maps'], lambda row: row['map'])[name]
    require(map_ground['owner_faction'] == FACTIONS[pool['owner_faction']]
            and map_ground['source']['archive_sha256'] == provenance['level_archive_sha256']
            and map_ground['source']['clv_sha256'] == provenance['level_clv_sha256']
            and len(map_ground['rows']) == 16, 'Domination ground level differs')
    locations = unique(pool['locations'], lambda row: row['index'])
    squads = unique(pool['squads'], lambda row: row['index'])
    exit_table = unique(exits['rows'], lambda row: (row['map'], token(row['spawn_token']), row['faction']))
    unsupported = unique(provenance['unsupported_spawn_links'], lambda row: row['index'])
    fallbacks = unique(provenance['default_spawn_fallbacks'], lambda row: row['squad_index'])
    vehicles, passenger, bindings, restored, properties = [], [], [], set(), {}
    ground_points = set()
    previous_spawn_index = -1
    for expected_vehicle_index, row in enumerate(map_ground['rows']):
        kind, point_index, point_token = row['spawn_type'], row['spawn_point_index'], token(row['spawn_point_token'])
        require(row['object_index'] == expected_vehicle_index and point_index > previous_spawn_index
                and row['source_family'] == 35 and row['authored_active'] is True
                and kind in (3, 7), 'Invalid native first-free ground order')
        previous_spawn_index = point_index
        point = locations[point_index]
        original = unsupported[point_index]
        is_apc = kind == 3
        team = 0 if is_apc else 1
        user_class = 0 if is_apc else 1
        require(row['team'] == team and row['user_class'] == user_class
                and point['token'] == point_token and point['kind'] == 'unsupported'
                and point['active'] and original['spawn_type'] == kind and original['category'] == 0
                and original['user_class'] == user_class,
                'Ground point differs from original spawn source')
        faction = policy['attacking_faction'] if is_apc else pool['owner_faction']
        variant = source['variants'][f'{kind}:{FACTIONS[faction]}']
        require(variant['faction'] == faction and variant['type'] == kind,
                'Ground vehicle variant does not match its source')
        properties[variant['source_member']] = variant['source_sha256']
        p = row['source_transform']
        origin = [f32(v) for v in p['position_xyz']]
        origin[1] = f32(origin[1] + variant['placement_y_offset'])
        rotation = quaternion(p['rotation_xyzw'])
        vehicle = dict(index=expected_vehicle_index, spawn_point_index=point_index,
                       spawn_point_token=point_token, vehicle_type_token=token(row['vehicle_type_token']),
                       faction=faction, team=team, modifier=0, position=position(origin),
                       rotation=[f32(v) for v in rotation], health_quarter_points=variant['health_quarter_points'],
                       initial_state=2)
        if is_apc:
            require(len(row['squad_links']) == 1, 'APC requires exactly one original squad')
            link = row['squad_links'][0]
            address = link['platoon_id'] * 4 + SQUADS[link['squad']]
            squad = squads[address]
            require(link['team'] == link['slot'] == 0 and address not in restored
                    and original['squad_links'] == [[0, link['platoon_id'], SQUADS[link['squad']], 0]]
                    and fallbacks[address]['default_spawn'] == squad['default_spawn']
                    and point_index not in squad['available'], 'APC does not restore an original category0')
            candidates = APC['exit_policy'](exit_table[(name, point_token, faction)], name,
                                            point_token, faction, origin, rotation)
            vehicle.update(passenger_seats=variant['passenger_seats'], maximum_occupancy=variant['maximum_occupancy'],
                           passenger_exit_candidates=candidates)
            point['kind'] = 'vehicle_passenger'
            squad['default_spawn'] = point_index
            squad['available'].insert(0, point_index)
            passenger.append(dict(spawn_point_index=point_index, spawn_point_token=point_token,
                                  vehicle_index=expected_vehicle_index))
            restored.add(address)
            ground_points.add(point_index)
        else:
            require(not row['squad_links'] and not original['squad_links'],
                    'Motor pool must have no player squad links')
        vehicles.append(vehicle)
        bindings.append(dict(vehicle_index=expected_vehicle_index, spawn_point_index=point_index,
                             spawn_point_token=point_token, vehicle_type_token=vehicle['vehicle_type_token'],
                             spawn_type=kind, user_class=user_class, source_family=35,
                             authored_active=True, team=team))
    require(len(passenger) == 8 and restored == set(fallbacks),
            'Expected every native APC fallback to be restored')
    active = squads[spawns['active']['squad_index']]
    spawns['active'].update(available=active['available'], default_spawn=active['default_spawn'])
    provenance['default_spawn_fallbacks'] = []
    provenance['unsupported_spawn_links'] = [row for row in provenance['unsupported_spawn_links']
                                             if row['index'] not in ground_points]
    provenance['domination_ground_sources_sha256'] = sha(ground_raw)
    provenance['domination_ground_exits_sha256'] = sha(exits_raw)
    spawns['source'] = json.dumps(provenance, sort_keys=True, separators=(',', ':'), allow_nan=False)
    require(len(spawns['source'].encode()) <= 8192, 'Ground spawn source exceeds bound')
    spawn = encode(spawns)
    ground = dict(format='mag_ground_vehicles', schema_version=1,
                  source=dict(spawn_catalog_sha256=sha(spawn), native_elf_sha256=source['native_elf_sha256'],
                              native_binding_sha256=sha(ground_raw), vehicle_properties_sha256=sha(encode(properties)),
                              creation_scope=SCHEMA),
                  vehicles=vehicles, passenger_spawns=passenger, vehicle_sources=bindings)
    if enable_apc_driver:
        ground = driver_catalog(ground, ground_raw, game_raw)
    return spawn, encode(ground)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('spawn-catalog', 'ground-sources', 'exits', 'spawn-output', 'ground-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--enable-apc-driver', action='store_true',
                        help='Opt in to source-qualified driver/manual passenger seats with 8m/750ms/25mps replacement limits')
    parser.add_argument('--game-archive', type=Path)
    args = parser.parse_args()
    inputs = [args.spawn_catalog, args.ground_sources, args.exits]
    outputs = [args.spawn_output, args.ground_output]
    require(args.enable_apc_driver == (args.game_archive is not None),
            'APC driver opt-in requires exactly one game archive')
    all_inputs = inputs + ([args.game_archive] if args.game_archive else [])
    require(len({path.resolve() for path in all_inputs + outputs}) == len(all_inputs + outputs),
            'Input and output paths must be distinct')
    if args.enable_apc_driver:
        require(all(not path.resolve().is_relative_to(Path(__file__).resolve().parents[2]) for path in outputs),
                'APC driver output must be outside the source checkout')
    spawn, ground = project(*(path.read_bytes() for path in inputs),
                           enable_apc_driver=args.enable_apc_driver,
                           game_raw=args.game_archive.read_bytes() if args.game_archive else None)
    for path, data in zip(outputs, (spawn, ground), strict=True):
        if args.check:
            require(path.read_bytes() == data, 'Domination ground output differs')
        else:
            with path.open('xb') as stream:
                stream.write(data)


if __name__ == '__main__':
    main()
