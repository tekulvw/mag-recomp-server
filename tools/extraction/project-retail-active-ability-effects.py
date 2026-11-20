#!/usr/bin/env python3
"""Project command effect settings from an explicitly supplied retail archive."""

import argparse
import hashlib
import json
from pathlib import Path
import runpy
import sys

TOOLS = Path(__file__).resolve().parent
CLUSTER = 'cluster_bomb_properties.zpr'
SENSOR = 'ability_sensor_properties.zpr'
PRECISION = 'ability_precision_strike_properties.zpr'
GUIDED = 'ability_guided_artillery_properties.zpr'
GAS = 'ability_gas_bombardment_properties.zpr'
STRAFE = 'ability_strafing_run_properties.zpr'
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


def project_precision(delay, _row):
    return {'delay_ms': delay}


def project_guided(delay, row):
    return {
        'delay_ms': delay,
        'damage_delay_ms': unsigned(row['DamageDelay_ms'], 'DamageDelay_ms', minimum=0),
        'effective_radius': scalar(row['EffectiveRadius'], 'EffectiveRadius'),
    }


def project_falloff(row, prefix, *, coincident=False):
    outer_radius = scalar(row[prefix + 'FullDamageRadius'], prefix + 'FullDamageRadius')
    inner_radius = scalar(row[prefix + 'HalfDamageRadius'], prefix + 'HalfDamageRadius')
    outer_scale = scalar(row[prefix + 'FullDamageScale'], prefix + 'FullDamageScale', zero=True)
    inner_scale = scalar(row[prefix + 'HalfDamageScale'], prefix + 'HalfDamageScale', zero=True)
    require((inner_radius < outer_radius or coincident and inner_radius == outer_radius)
            and outer_scale <= 1 and inner_scale <= 1, 'Invalid command blast falloff')
    return {
        'outer_radius': outer_radius,
        'outer_scale': outer_scale,
        'inner_radius': inner_radius,
        'inner_scale': inner_scale,
    }


def project_gas(delay, row):
    # The inherited DamageDelay_ms is absent in these retail rows. Original
    # AbilityProps construction stores zero at its field offset (+0x28).
    return {
        'delay_ms': delay,
        'damage_delay_ms': unsigned(row.get('DamageDelay_ms', 0), 'DamageDelay_ms', minimum=0),
        'duration_ms': unsigned(row['Duration_ms'], 'Duration_ms'),
        'interval_ms': unsigned(row['TimeBetweenBomblets_ms'], 'TimeBetweenBomblets_ms'),
        'num_bomblets': unsigned(row['NumBomblets'], 'NumBomblets', maximum=255),
        'bomblet_damage': scalar(row['BombletDamage'], 'BombletDamage'),
        'bomblet': project_falloff(row, 'Bomblet'),
        'gas_damage_per_second': scalar(row['GasDamagePerSecond'], 'GasDamagePerSecond'),
        'gas': project_falloff(row, 'Gas', coincident=True),
    }


def project_strafe(delay, row):
    return {
        'delay_ms': delay,
        'damage_delay_ms': unsigned(row['DamageDelay_ms'], 'DamageDelay_ms', minimum=0),
        'duration_ms': unsigned(row['StrafeDuration_ms'], 'StrafeDuration_ms'),
        'interval_ms': unsigned(row['TimeBetweenBomblets_ms'], 'TimeBetweenBomblets_ms'),
        'strafe_length': scalar(row['StrafeLength'], 'StrafeLength'),
        'deviation_radius': scalar(row['DeviationRadius'], 'DeviationRadius', zero=True),
        'bomblet_damage': scalar(row['BombletDamage'], 'BombletDamage'),
        'bomblet': project_falloff(row, 'Bomblet'),
    }


TABLES = (
    ('cluster', CLUSTER, 'ClusterBombProps', 6, project_cluster),
    ('sensor', SENSOR, 'AbilitySensorProps', 6, project_sensor),
    ('precision_strike', PRECISION, 'PrecisionStrikeProps', 3, project_precision),
    ('guided_artillery', GUIDED, 'GuidedArtilleryProps', 6, project_guided),
    ('gas_bombardment', GAS, 'GasBombardmentProps', 6, project_gas),
    ('strafing_run', STRAFE, 'StrafingRunProps', 6, project_strafe),
)


def project_tables(tables):
    rows = []
    for variant, _, property_name, count, project in TABLES:
        table = tables[variant]
        delay = unsigned(table['Delay_ms'], 'Delay_ms')
        properties = table['PropsList']
        expected = [f'{property_name}{i}' for i in range(count)]
        require(list(properties) == expected, f'Invalid {variant} properties rows or order')
        selected = set()
        for ordinal, entry in enumerate(properties.values()):
            row = entry[property_name]
            faction = FACTIONS.get(row['Faction'])
            require(faction is not None, f'Unknown {variant} row faction')
            # Precision shares one faction group on both gameplay sides.
            # Store those rows under the canonical first (attacker) group.
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
    for variant, name, property_name, _, _ in TABLES:
        member = f'/run/mag/ps3/scripts/abilities/{name}'
        data = archive.extract(archive.names.index(member) + 1)
        sources[variant] = {'member': member, 'sha256': digest(data)}
        parsed = api['Parser'](data.decode('utf-8-sig'), member).parse()
        tables[variant] = api['normalize'](parsed)[property_name + 'Table']
    return {
        'schema_version': 2,
        'application_id': 20231,
        'source_archive_sha256': source_archive,
        'source_members': sources,
        'rows': project_tables(tables),
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
