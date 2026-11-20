#!/usr/bin/env python3
"""Project qualified Domination ground lifecycle configuration into a private file."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import runpy
import struct

TOOLS = Path(__file__).resolve().parent


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def unique(rows, key):
    indexed = {key(row): row for row in rows}
    require(len(indexed) == len(rows), 'Duplicate source records')
    return indexed


def project_profile(spawn_raw, ground_raw, elf_sha, archive_sha, rules_source, interval):
    spawn, ground = map(json.loads, (spawn_raw, ground_raw))
    policy = spawn['match_profile']
    pool, = spawn['maps']
    provenance = json.loads(spawn['source'])
    require(spawn['format'] == 'mag_spawn_catalog' and policy['game_type'] == 2
            and pool['runtime_indices_verified'] is True
            and pool['name'] == spawn['active']['map_name'] == provenance['map']
            and policy['spawn_wave_ms'] == interval
            and provenance['game_archive_sha256'] == archive_sha
            and all(provenance['game_rules_source'][key] == rules_source[key]
                    for key in ('sha256', 'rules_json_pointer')),
            'Unbound Domination spawn profile or original wave settings')
    source = ground['source']
    require(ground['format'] == 'mag_ground_vehicles'
            and source['creation_scope'] == 'native_domination_vehicle_source_points'
            and source['spawn_catalog_sha256'] == sha(spawn_raw)
            and source['native_elf_sha256'] == elf_sha
            and source['native_binding_sha256'] == provenance['domination_ground_sources_sha256'],
            'Unbound ground catalog')
    vehicles = unique(ground['vehicles'], lambda row: row['index'])
    sources = unique(ground['vehicle_sources'], lambda row: row['vehicle_index'])
    points = unique(pool['locations'], lambda row: row['index'])
    require(set(vehicles) == set(sources) == set(range(16))
            and sorted(row['spawn_type'] for row in sources.values()) == [3] * 8 + [7] * 8,
            'Unexpected ground roster')
    require(len({row['spawn_point_index'] for row in sources.values()}) == 16,
            'Duplicate ground spawn point')
    attacker = policy['attacking_faction']
    defender = pool['owner_faction']
    require(attacker in range(3) and defender in range(3) and attacker != defender,
            'Unexpected ground factions')
    health = {}
    types = {}
    for index, vehicle in vehicles.items():
        row = sources[index]
        types[index] = kind = row['spawn_type']
        team = 0 if kind == 3 else 1
        point = points[row['spawn_point_index']]
        require(row['source_family'] == 35 and row['authored_active'] is True
                and row['user_class'] == row['team'] == vehicle['team'] == team
                and vehicle['faction'] == (attacker if kind == 3 else defender)
                and point['token'] == row['spawn_point_token']
                and all(vehicle[key] == row[key] for key in (
                    'spawn_point_index', 'spawn_point_token', 'vehicle_type_token')),
                'Ground creation differs from its bound source point')
        key = (vehicle['vehicle_type_token'], vehicle['faction'])
        hp = vehicle['health_quarter_points']
        require(health.setdefault(key, hp) == hp, 'Inconsistent source vehicle max health')
    return (pool['name'], attacker, defender), types, health


def project(elf_path, game_path, body_raw, pairs):
    original = runpy.run_path(str(TOOLS / 'extract-retail-falling-damage.py'))
    native = original['Elf'](elf_path, 'retail')
    elf_sha = sha(native.raw)
    game_raw = game_path.read_bytes()
    archive_sha = sha(game_raw)
    require(archive_sha == original['SOURCES']['archive'], 'Wrong original game archive')
    game = original['Psarc'](game_raw)
    reader = runpy.run_path(str(TOOLS / 'build-retail-spawn-catalog.py'))
    parse = runpy.run_path(str(TOOLS / 'retail_map_helpers.py'))['parse_zpr']
    literal = runpy.run_path(str(TOOLS / 'extract-item-catalog.py'))

    def extract(member):
        require(game.names.count(member) == 1, f'Missing or ambiguous original member {member}')
        return game.extract(game.names.index(member) + 1)

    name = '/run/mag/ps3/settings/gameproperties.zpr'
    raw = extract(name)
    _, rules_source = reader['original_rules'](raw, 'GameType256')
    rules = literal['normalize'](literal['Parser'](raw.decode(), name).parse())
    for part in rules_source['rules_json_pointer'].split('/')[1:]:
        rules = rules[part]
    interval = [rules[f'SpawnIntervalTeam{team}'] for team in range(2)]
    require(all(type(value) is int and 0 < value <= 60000 for value in interval),
            'Unsupported original team wave interval')
    require(len(pairs) == 6, 'Require six paired Domination catalogs')
    profiles = []
    observed = {}
    required_health = {}
    for spawn_path, ground_path in pairs:
        spawn_raw, ground_raw = spawn_path.read_bytes(), ground_path.read_bytes()
        (level, attacker, defender), types, health = project_profile(
            spawn_raw, ground_raw, elf_sha, archive_sha, rules_source, interval)
        attackers = observed.setdefault((level, defender), set())
        require(attacker not in attackers, 'Duplicate Domination profile')
        attackers.add(attacker)
        for key, hp in health.items():
            require(required_health.setdefault(key, hp) == hp, 'Inconsistent profile vehicle HP')
        profiles.append(dict(profile=ground_path.name, ground_sha256=sha(ground_raw),
                             spawn_sha256=sha(spawn_raw), spawn_types=types))
    require(len({level for level, _ in observed}) == len(observed) == 3
            and all(attackers == set(range(3)) - {defender}
                    for (_, defender), attackers in observed.items())
            and len(required_health) == 6, 'Incomplete original profile/faction inventory')

    body = json.loads(body_raw)
    require(body['schema'] == 'mag_private_ground_lifecycle_bindings_v1',
            'Require qualified ground body/lifecycle source')
    body_rows = unique(body['vehicles'], lambda row: (row['vehicle_type_token'], row['faction']))
    vehicles = []
    for key, row in body_rows.items():
        if key not in required_health:
            continue
        props_source = row['properties_source']
        props_raw = extract(props_source['member'])
        require(sha(props_raw) == props_source['sha256'], 'Changed original vehicle properties')
        props = parse(props_raw.decode('utf-8-sig'))['GroundVehicleProperties']['DamageProperties']
        capabilities = row['capabilities']
        blast = capabilities['destruction']
        require('DestroyMaterialPenetration' not in props
                and 4 * float(props['MaxHitpoints']) == row['maximum_health_quarter_points']
                == required_health[key]
                and float(props['DestroyDamageValue']) == blast['damage']
                and float(props['DestroyDamageFullRadius']) == blast['full_radius']
                and float(props['DestroyDamageHalfRadius']) == blast['half_radius']
                and 0 < blast['half_radius'] < blast['full_radius'] and blast['damage'] > 0
                and props['DestroyedHulkPhysicsModel']
                and capabilities['hulk_template']
                and capabilities['hulk_template'] == row['hulk_template']
                and len(row['hulk_resource_sha256']) == 64
                and all(char in '0123456789abcdef' for char in row['hulk_resource_sha256'])
                and capabilities['delivered_inert_seconds'] is None,
                'Qualified ground body differs from original vehicle properties')
        vehicles.append(row)
    require(len(vehicles) == len(required_health), 'Missing required ground body bindings')
    # Scalar cell qualified separately for both retail Ground property factories.
    penetration = struct.unpack('>f', native.read(0x1b3dc3c, 4)[0])[0]
    require(math.isfinite(penetration) and penetration > 0., 'Invalid native penetration value')
    return dict(schema='mag_domination_ground_lifecycle_v1', native_elf_sha256=elf_sha,
                game_rules_source=rules_source, base_lifecycle_sha256=sha(body_raw),
                material_penetration=penetration, team_interval_ms=interval, vehicles=vehicles,
                profiles=sorted(profiles, key=lambda row: row['profile']))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ('elf', 'game-archive', 'body-lifecycle', 'output'):
        parser.add_argument('--' + flag, type=Path, required=True)
    parser.add_argument('--profile', type=Path, nargs=2, action='append', required=True,
                        metavar=('SPAWNS', 'GROUND'))
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        inputs = [args.elf, args.game_archive, args.body_lifecycle]
        inputs.extend(path for pair in args.profile for path in pair)
        require(not output.is_relative_to(TOOLS.parents[1])
                and output not in {path.resolve() for path in inputs},
                'Output must be outside the source checkout and inputs')
        result = project(args.elf, args.game_archive, args.body_lifecycle.read_bytes(), args.profile)
        data = (json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + '\n').encode()
        if args.check:
            require(output.read_bytes() == data, 'Ground lifecycle output differs')
        else:
            with output.open('xb') as stream:
                stream.write(data)
        print(json.dumps({'output_sha256': sha(data)}))
    except (KeyError, TypeError, ValueError, OSError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
