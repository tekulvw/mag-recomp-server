#!/usr/bin/env python3
"""Join one Domination profile to original Helo spline, factory and seat sources."""
import argparse
import json
import math
from pathlib import Path
import runpy

HELPERS = runpy.run_path(str(Path(__file__).with_name('project-retail-apc-insertion.py')))
require, sha, encode = (HELPERS[key] for key in ('require', 'sha', 'encode'))


def project(spawns_raw, links_raw, paths_raw, models_raw):
    spawns, links, paths, models = map(json.loads, (spawns_raw, links_raw, paths_raw, models_raw))
    policy = spawns['match_profile']
    require(spawns['format'] == 'mag_spawn_catalog' and policy['game_type'] == 2
            and policy['auto_assign_teams'] is True and len(spawns['maps']) == 1,
            'Require native Domination spawn profile')
    pool = spawns['maps'][0]
    map_name = pool['name']
    provenance = json.loads(spawns['source'])
    require(pool['runtime_indices_verified'] is True and map_name == spawns['active']['map_name']
            and links['schema'] == 'mag_private_retail_domination_aircraft_links_v1'
            and paths['schema'] == 'mag_private_retail_domination_helo_paths_v1'
            and models['schema'] == 'mag_private_retail_domination_helo_models_v1'
            and paths['retail_elf_sha256'] == links['retail_elf_sha256'] == models['retail_elf_sha256']
            and models['game_archive_sha256'] == provenance['game_archive_sha256']
            and models['vehicle_registry_sha256'] == links['vehicle_registry_sha256']
            and paths['aircraft_links_sha256'] == sha(links_raw)
            and links['initial_spawn_bindings_sha256'] == provenance['native_spawn_bindings_sha256'],
            'Unbound Domination Helo source receipts')
    link_sets = [row for row in links['maps'] if row['map'] == map_name]
    route_sets = [row for row in paths['maps'] if row['map'] == map_name]
    require(len(link_sets) == len(route_sets) == 1
            and route_sets[0]['level_clv_sha256'] == provenance['level_clv_sha256'],
            'Domination Helo level differs')
    roles = {0: 'Valor', 1: 'Raven', 2: 'SVER'}
    faction = policy['attacking_faction']
    model = models['variants'][roles[faction]]
    passenger = [group for group in model['seat_groups'] if group['SeatType'] == 'PassengerSeat']
    require(model['max_occupancy'] == 16 and len(passenger) == 1
            and model['seat_groups'][-1] == passenger[0]
            and passenger[0]['Name'] == '' and passenger[0]['occupantIsVisible'] == 'false'
            and int(passenger[0]['SeatCount']) in (14, 15),
            'Unsupported original Helo passenger layout')
    passenger_start = 16 - int(passenger[0]['SeatCount'])
    locations = {point['index']: point for point in pool['locations']}
    authored = {point['index']: point for point in provenance['unsupported_spawn_links']
                if point['spawn_type'] == 4}
    routes = {route['point_token']: route for route in route_sets[0]['paths']}
    require(len(locations) == len(pool['locations']) and len(routes) == len(authored) == 4
            and len(link_sets[0]['helo_points']) == 4, 'Incomplete Domination Helo sources')
    carriers = []
    for index, link in enumerate(link_sets[0]['helo_points']):
        packed = link['packed_family35_id']
        spawn_index = packed & 0xffff
        point = locations[spawn_index]
        origin = authored[spawn_index]
        require(packed == (0x23800000 | spawn_index)
                and link['candidate_family2_index'] == index and link['spawn_type'] == 4
                and point['token'] == link['point_token'] and point['kind'] == 'unsupported'
                and not point['active'] and not origin['active']
                and origin['category'] == 2 and origin['user_class'] == 0
                and len(origin['squad_links']) == 2
                and all(side == 0 and slot == 2 and 0 <= platoon <= 3 and 0 <= squad <= 3
                        for side, platoon, squad, slot in origin['squad_links']),
                'Domination Helo spawn point differs')
        squads = sorted({platoon * 4 + squad for _, platoon, squad, _ in origin['squad_links']})
        require(len(squads) == 2, 'Duplicate source Helo squad membership')
        route = routes[point['token']]
        knots = route['knots']
        parameters = [knot['normalized_parameter'] for knot in knots]
        require(3 <= len(knots) <= 128
                and 0 < route['authored_drop_point_index'] < len(knots) - 1
                and parameters[0] == 0.0 and parameters[-1] == 1.0
                and all(a < b for a, b in zip(parameters, parameters[1:]))
                and all(len(knot[key]) == 3 and all(math.isfinite(v) for v in knot[key])
                        for knot in knots for key in ('position_xyz', 'tangent_xyz')),
                'Invalid original Helo spline order')
        carriers.append(dict(object_index=index, spawn_index=spawn_index, spawn_token=point['token'],
                             vehicle_type_token=link['vehicle_type_token'], faction=faction,
                             squads=squads, health_quarter_points=int(model['max_hitpoints'] * 4),
                             passenger_start=passenger_start,
                             acceleration_seconds=model['acceleration_seconds'],
                             braking_seconds=model['braking_seconds'],
                             start_delay_max_seconds=model['flight_start_delay_max_seconds'],
                             route=dict(inbound_flight_seconds=route['inbound_flight_seconds'],
                                        outbound_flight_seconds=route['outbound_flight_seconds'],
                                        drop_point_index=route['authored_drop_point_index'],
                                        hover_position_delay_seconds=model['hover_delay_seconds'],
                                        knots=[{key: knot[key] for key in
                                                ('position_xyz', 'normalized_parameter', 'tangent_xyz')}
                                               for knot in route['knots']])))
    return encode(dict(schema='mag_domination_helo_fleet_v1', map=map_name,
                       spawn_catalog_sha256=sha(spawns_raw),
                       source_receipt_sha256=[sha(links_raw), sha(paths_raw), sha(models_raw)],
                       carriers=carriers))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('spawn-catalog', 'aircraft-links', 'paths', 'models', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [args.spawn_catalog, args.aircraft_links, args.paths, args.models]
    require(len({path.resolve() for path in [*inputs, args.output]}) == 5,
            'Require distinct Domination Helo inputs and output')
    output = project(*(path.read_bytes() for path in inputs))
    if args.check:
        require(args.output.read_bytes() == output, 'Projected Domination Helo fleet differs')
    else:
        with args.output.open('xb') as stream:
            stream.write(output)


if __name__ == '__main__':
    main()
