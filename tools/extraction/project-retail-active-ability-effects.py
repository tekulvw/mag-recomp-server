#!/usr/bin/env python3
"""Project ClusterBomb and Sensor settings from an explicitly supplied retail archive."""

import argparse
import hashlib
import json
from pathlib import Path
import runpy
import sys

TOOLS = Path(__file__).resolve().parent
CLUSTER = 'cluster_bomb_properties.zpr'
SENSOR = 'ability_sensor_properties.zpr'
FACTIONS = {
    'FactionTypeBlack': 'black',
    'FactionTypeGreen': 'green',
    'FactionTypeRed': 'red',
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def unsigned(value, field, minimum=1, maximum=(1 << 32) - 1):
    require(type(value) is int and minimum <= value <= maximum,
            f'Invalid command property {field}')
    return value


def scalar(value, field, *, zero=False):
    # Reject values that cannot be squared in the server's f32 detector.
    require(type(value) in (int, float) and 0 <= value <= 1e18
            and (zero or value > 0),
            f'Invalid command property {field}')
    return value


def project_cluster(delay, row):
    whole = lambda field, **kw: unsigned(row[field], field, **kw)
    real = lambda field, **kw: scalar(row[field], field, **kw)
    inner_radius = real('HalfDamageRadius')
    outer_radius = real('FullDamageRadius')
    inner_scale = real('HalfDamageScale', zero=True)
    outer_scale = real('FullDamageScale', zero=True)
    require(inner_radius < outer_radius and inner_scale in (0., 0.5) and outer_scale == 0.,
            'Unsupported command blast falloff')
    return {
        'delay_ms': delay,
        'damage_delay_ms': whole('DamageDelay_ms', minimum=0),
        'interval_ms': whole('TimeBetweenExplosions_ms'),
        'count': whole('NumExplosions', maximum=255),
        'scatter_radius': real('Radius'),
        'damage': real('Damage'),
        'inner_radius': inner_radius,
        'inner_scale': inner_scale,
        'outer_radius': outer_radius,
        'outer_scale': outer_scale,
    }


def project_sensor(delay, row):
    return {
        'delay_ms': delay,
        'duration_ms': unsigned(row['Duration_ms'], 'Duration_ms'),
        'attacker_radius': scalar(row['AttackerRadius'], 'AttackerRadius'),
        'defender_radius': scalar(row['DefenderRadius'], 'DefenderRadius'),
        'speed_threshold': scalar(row['DetectionSpeedThreshold'], 'DetectionSpeedThreshold', zero=True),
    }


def project_tables(cluster, sensor):
    rows = []
    for variant, table, property_name, project in [
        ('cluster', cluster, 'ClusterBombProps', project_cluster),
        ('sensor', sensor, 'AbilitySensorProps', project_sensor),
    ]:
        delay = unsigned(table['Delay_ms'], 'Delay_ms')
        properties = table['PropsList']
        expected = [f'{property_name}{i}' for i in range(6)]
        require(list(properties) == expected, f'Invalid {variant} properties rows or order')
        selected = set()
        for ordinal, entry in enumerate(properties.values()):
            row = entry[property_name]
            faction = FACTIONS.get(row['Faction'])
            require(faction is not None, f'Unknown {variant} row faction')
            # Table access partitions the six authored rows into groups of
            # three for side index 0/1, then searches the faction field.
            side = 'attacker' if ordinal < 3 else 'defender'
            require((side, faction) not in selected, f'Unexpected repeated {variant} side/faction')
            selected.add((side, faction))
            rows.append({'side': side, 'faction': faction, 'rules': {variant: project(delay, row)}})
    return rows


def project(game_archive):
    api = runpy.run_path(str(TOOLS / 'extract-item-catalog.py'))
    sys.path.insert(0, str(TOOLS.parent / 'collision'))
    from archive import Psarc

    require(game_archive.stat().st_size <= 16 * 1024 * 1024, 'Unexpected command rules archive size')
    original = game_archive.read_bytes()
    source_archive = digest(original)
    require(source_archive == api['RETAIL_ARCHIVE_HASH'], 'Wrong original retail game archive')
    archive = Psarc(original)
    sources, tables = {}, {}
    for variant, name, table in [
        ('cluster', CLUSTER, 'ClusterBombPropsTable'),
        ('sensor', SENSOR, 'AbilitySensorPropsTable'),
    ]:
        member = f'/run/mag/ps3/scripts/abilities/{name}'
        data = archive.extract(archive.names.index(member) + 1)
        sources[variant] = {'member': member, 'sha256': digest(data)}
        parsed = api['Parser'](data.decode('utf-8-sig'), member).parse()
        tables[variant] = api['normalize'](parsed)[table]
    return {
        'schema_version': 1,
        'application_id': 20231,
        'source_archive_sha256': source_archive,
        'source_members': sources,
        'rows': project_tables(tables['cluster'], tables['sensor']),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        require(not output.is_relative_to(TOOLS.parents[1])
                and output != args.game_archive.resolve(),
                'Output must be outside the source checkout and inputs')
        encoded = (json.dumps(project(args.game_archive), indent=2, allow_nan=False) + '\n').encode()
        if args.check:
            require(output.read_bytes() == encoded, 'Command effect projection differs')
        else:
            with output.open('xb') as stream:
                stream.write(encoded)
    except (KeyError, OSError, TypeError, ValueError) as error:
        parser.error(str(error))


if __name__ == '__main__':
    main()
