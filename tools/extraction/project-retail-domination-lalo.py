#!/usr/bin/env python3
"""Project original inactive Domination LALO human arrival poses."""
import argparse
import json
from pathlib import Path
import runpy

APC = runpy.run_path(str(Path(__file__).with_name('project-retail-apc-insertion.py')))
require, sha, encode = (APC[key] for key in ('require', 'sha', 'encode'))


def project(spawn_raw, lalo_raw):
    spawns, proof = map(json.loads, (spawn_raw, lalo_raw))
    require(spawns['format'] == 'mag_spawn_catalog'
            and spawns['match_profile']['game_type'] == 2
            and spawns['match_profile']['auto_assign_teams'] is True
            and len(spawns['maps']) == 1, 'Require a native Domination profile')
    pool = spawns['maps'][0]
    provenance = json.loads(spawns['source'])
    require(pool['runtime_indices_verified'] is True
            and pool['name'] == spawns['active']['map_name']
            and proof['schema'] == 'mag_private_retail_domination_lalo_v1'
            and proof['initial_spawn_bindings_sha256'] == provenance['native_spawn_bindings_sha256'],
            'Unbound original Domination LALO placement')
    maps = [row for row in proof['maps'] if row['map'] == pool['name']]
    require(len(maps) == 1, 'Require exactly one original LALO map')
    points = {point['index']: point for point in maps[0]['points']}
    authored = {point['index']: point for point in provenance['unsupported_spawn_links']
                if point['spawn_type'] == 5}
    require(len(points) == len(maps[0]['points']) == len(authored) == 8
            and points.keys() == authored.keys(), 'Incomplete original LALO point inventory')
    locations = {point['index']: point for point in pool['locations']}
    require(len(locations) == len(pool['locations']), 'Duplicate native spawn identity')
    for index, row in points.items():
        location = locations[index]
        authored_point = authored[index]
        require(authored_point['category'] == 2 and authored_point['user_class'] == 0
                and location['token'] == row['token'] and location['kind'] == 'unsupported'
                and location['active'] is False and authored_point['active'] is False
                and len(authored_point['squad_links']) == 1
                and authored_point['squad_links'][0][0] == 0 and authored_point['squad_links'][0][3] == 2
                and row['all_wave_poses_clear'] is True and row['all_wave_poses_above_ground'] is True
                and [location['position'][axis] for axis in 'xyz'] == row['source_position_xyz'],
                'Domination LALO point differs from original pose or squad')
        arrival = row['parachute']
        require(set(arrival) == {'forward', 'spacing_m', 'yaw'}
                and len(arrival['forward']) == 3
                and abs(sum(x*x for x in arrival['forward']) - 1.0) < 1e-6
                and arrival['spacing_m'] == 10.0 and type(arrival['yaw']) is int
                and 0 <= arrival['yaw'] < 1024, 'Unqualified native Domination LALO wave')
        location['kind'] = 'parachute'
        location['parachute'] = arrival
    provenance['domination_lalo_sha256'] = sha(lalo_raw)
    spawns['source'] = json.dumps(provenance, sort_keys=True, separators=(',', ':'), allow_nan=False)
    require(len(spawns['source'].encode()) <= 8192, 'LALO spawn source exceeds bound')
    return encode(spawns)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('spawn-catalog', 'lalo-poses', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    require(len({p.resolve() for p in (args.spawn_catalog, args.lalo_poses, args.output)}) == 3,
            'Require separate LALO input and output paths')
    output = project(args.spawn_catalog.read_bytes(), args.lalo_poses.read_bytes())
    if args.check:
        require(args.output.read_bytes() == output, 'Projected Domination LALO spawn catalog differs')
    else:
        with args.output.open('xb') as stream:
            stream.write(output)


if __name__ == '__main__':
    main()
