#!/usr/bin/env python3
"""Join authored Domination FSP/squad membership to qualified initial slot catalogs."""
import argparse
import hashlib
import json
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def token(value):
    require(isinstance(value, str) and len(value) == 10 and value.startswith('0x'),
            'Invalid authored token')
    return int(value[2:], 16)


def unique(rows, key):
    result = {row[key]: row for row in rows}
    require(len(result) == len(rows), f'Duplicate {key}')
    return result


def project(metadata_raw, spawn_raw, objective_raw):
    require(len(metadata_raw) <= 32 * 1024**2 and len(spawn_raw) <= 1_048_576
            and len(objective_raw) <= 524_288, 'Input exceeds bound')
    metadata, spawn, objectives = map(json.loads, (metadata_raw, spawn_raw, objective_raw))
    require(metadata['format'] == 'mag_retail_domination_map_metadata'
            and metadata['schema_version'] == 1 and metadata['mode_enum'] == 'GameType256',
            'Require Domination metadata')
    require(spawn['match_profile']['game_type'] == objectives['game_type'] == 2
            and len(spawn['maps']) == 1, 'Require Domination catalogs')
    pool = spawn['maps'][0]
    require(pool['name'] == objectives['map_name']
            and objectives['source']['authored_catalog_sha256'] == sha(metadata_raw),
            'Objective metadata source mismatch')
    require(pool['runtime_indices_verified'] is True, 'Require qualified spawn bindings')
    maps = {
        row['map']['map']: row for row in metadata['maps']}
    require(len(maps) == len(metadata['maps']) and pool['name'] in maps, 'Map source mismatch')
    authored = maps[pool['name']]
    originals = unique(authored['spawns'], 'name_token')
    points_by_token = unique(pool['locations'], 'token')
    require(set(points_by_token) == {token(name) for name in originals}, 'Spawn binding coverage mismatch')
    require(len({p['index'] for p in points_by_token.values()}) == len(points_by_token)
            and all(type(p['index']) is int and 0 <= p['index'] < 128 for p in points_by_token.values()),
            'Invalid spawn indices')
    points = []
    for name, original in originals.items():
        require(original['team'] in (0, 1) and original['user_class'] in (0, 1),
                'Unsupported spawn team or user class')
        value = token(name)
        points.append(dict(token=value, index=points_by_token[value]['index'],
                           team=original['team'], user_class=original['user_class']))
    definitions = unique(authored['objectives']['definitions'], 'definition_token')
    bindings = unique(objectives['objects'], 'token')
    links = []
    for instance in authored['objectives']['instances']:
        fields = definitions[instance['definition_token']]['fields']
        spawn_token = token(fields.get('ForwardSpawnPoint', {'token': '0x00000000'})['token'])
        if not spawn_token:
            continue
        require(instance['name_token'] in bindings and spawn_token in points_by_token,
                'Unbound objective FSP link')
        raw = bytes.fromhex(fields['FSPType']['raw_hex'])
        require(len(raw) == 13 and raw[:12] == bytes.fromhex('deadbeef0000000100000000'),
                'Unsupported FSP enum encoding')
        fsp_type = raw[12]
        require(fsp_type in (0, 1), 'Unsupported FSP type')
        links.append(dict(objective_token=token(instance['name_token']), spawn_token=spawn_token,
                          fsp_type=fsp_type))
    declared = {(token(link['objective_token']), token(link['spawn_token']))
                for link in authored['forward_spawns']}
    require(len(declared) == len(authored['forward_spawns']) == len(links)
            and declared == {(link['objective_token'], link['spawn_token']) for link in links},
            'Authored forward link coverage mismatch')
    require(len(authored['squads']) == 32, 'Require all32 source squads')
    squads = []
    for squad in authored['squads']:
        team, platoon, slot = (squad[key] for key in ('team', 'platoon', 'squad'))
        require(team in (0, 1) and platoon in range(4) and slot in range(4), 'Invalid squad address')
        members = squad['spawns']
        require(0 < len(members) <= 32 and all(name in originals for name in members),
                'Invalid full squad membership')
        require(all(originals[name]['team'] == team for name in members), 'Squad member team mismatch')
        fallback = next((token(name) for name in members if originals[name]['category'] == 0), None)
        squads.append(dict(index=16 * team + 4 * platoon + slot,
                           points=list(map(token, members)), category_zero=fallback))
    require({squad['index'] for squad in squads} == set(range(32)), 'Incomplete32 squad addresses')
    return dict(format='mag_domination_fsp_bindings_v1', metadata_sha256=sha(metadata_raw),
                spawn_catalog_sha256=sha(spawn_raw), map_name=pool['name'],
                points=sorted(points, key=lambda point: point['index']), links=links,
                squads=sorted(squads, key=lambda squad: squad['index']))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('metadata', 'spawn-catalog', 'objective-catalog', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        require(not output.is_relative_to(Path(__file__).resolve().parents[2]) and output not in {
            path.resolve() for path in (args.metadata, args.spawn_catalog, args.objective_catalog)
        }, 'Output must be outside the source checkout and inputs')
        value = project(args.metadata.read_bytes(), args.spawn_catalog.read_bytes(),
                        args.objective_catalog.read_bytes())
        encoded = (json.dumps(value, indent=2) + '\n').encode()
        if args.check:
            require(output.read_bytes() == encoded, 'FSP output differs')
        else:
            with output.open('xb') as stream:
                stream.write(encoded)
        print(json.dumps({'map': value['map_name'], 'points': len(value['points']),
                          'squads': len(value['squads']), 'links': len(value['links']),
                          'sha256': sha(encoded)}))
    except (KeyError, ValueError, TypeError, OSError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')



if __name__ == '__main__':
    main()
