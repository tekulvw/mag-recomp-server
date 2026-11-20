#!/usr/bin/env python3
"""Read supported original Helo durability, armor and blast settings into private rules.

Native interpretation is qualified separately. No game resources are bundled.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import runpy
import struct

TOOLS = Path(__file__).resolve().parent
NAMES = ('Valor', 'Raven', 'SVER')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def number(value):
    require(not isinstance(value, bool), 'Boolean vehicle damage scalar')
    value = float(value)
    require(math.isfinite(value) and value >= 0., 'Invalid vehicle damage scalar')
    return value


def project_profiles(resources, maximum_health, penetration):
    require(type(maximum_health) in (int, float) and type(penetration) in (int, float)
            and math.isfinite(maximum_health) and maximum_health > 0.
            and math.isfinite(penetration) and penetration > 0.,
            'Invalid native Helo defaults')
    profiles = {}
    for faction, name in enumerate(NAMES):
        props = resources[name]['HeloVehicleProperties']
        damage = props['DamageProperties']
        require('MaxHitpoints' not in damage and 'DestroyMaterialPenetration' not in damage
                and damage.get('DestroyedHulk') == ''
                and not damage.get('DestroyedHulkPhysicsModel'),
                'Unqualified Helo damage override or hulk')
        armor = number(damage['ArmorType'])
        require(armor.is_integer() and armor <= 255, 'Invalid Helo armor type')
        inner = number(damage['DestroyDamageHalfRadius'])
        outer = number(damage['DestroyDamageFullRadius'])
        require(outer >= inner, 'Inverted Helo blast radii')
        fractions = [number(props[key]) for key in
                     ('InBoundMinDamageFraction', 'OutBoundMinDamageFraction', 'RetreatDamageFraction')]
        require(all(value <= 1. for value in fractions), 'Invalid Helo damage fraction')
        profiles[faction] = dict(max_health=maximum_health, armor_type=int(armor),
            critical_loss_percent_per_second=number(damage['DamageCriticalHealthLossRate']),
            inbound_min_fraction=fractions[0], outbound_min_fraction=fractions[1],
            retreat_fraction=fractions[2], destruction=dict(
                damage=number(damage['DestroyDamageValue']), inner_radius=inner,
                outer_radius=outer, material_penetration=penetration,
                effect_seconds=number(damage['DestroyFXTime'])))
    return profiles


def project_grenade_penetration(resource):
    rows = resource['ProjectileAmmoDataTable']['ProjectileAmmoDataList']
    require(isinstance(rows, dict) and 1 <= len(rows) <= 128
            and all(key.startswith('ProjectileAmmoData') and isinstance(row, dict)
                    and isinstance(row.get('ProjectileAmmoData'), dict)
                    and 'MaterialPenetration' in row['ProjectileAmmoData'] for key, row in rows.items()),
            'Incomplete original projectile ammo rows')
    values = {number(row['ProjectileAmmoData']['MaterialPenetration']) for row in rows.values()}
    require(len(values) == 1 and 0. < next(iter(values)) <= 3.4028234663852886e38,
            'Nonuniform original grenade material penetration')
    return struct.unpack('>f', struct.pack('>f', next(iter(values))))[0]


def project(elf_path, game_path, models_raw):
    original = runpy.run_path(str(TOOLS / 'extract-retail-falling-damage.py'))
    native = original['Elf'](elf_path, 'retail')
    game_raw = game_path.read_bytes()
    require(sha(game_raw) == original['SOURCES']['archive'], 'Wrong original game archive')
    require(len(models_raw) <= 4 * 1024 * 1024, 'Helo models input exceeds bound')
    models = json.loads(models_raw)
    require(models['schema'] == 'mag_private_retail_domination_helo_models_v1'
            and models['game_archive_sha256'] == sha(game_raw)
            and models['retail_elf_sha256'] == sha(native.raw), 'Unbound Helo model sources')
    require(set(models['variants']) == set(NAMES), 'Incomplete Helo model sources')
    game = original['Psarc'](game_raw)
    parse = runpy.run_path(str(TOOLS / 'retail_map_helpers.py'))['parse_zpr']
    resources = {}
    for name in NAMES:
        source = models['variants'][name]
        member = source['properties_member']
        require(game.names.count(member) == 1, 'Missing or ambiguous original Helo properties')
        raw = game.extract(game.names.index(member) + 1)
        require(sha(raw) == source['properties_sha256'], 'Changed original Helo properties')
        resources[name] = parse(raw.decode('utf-8-sig'))
    # Supported-build scalar cells: inherited maximum HP and explosion material
    # penetration. The reader verifies the complete original ELF digest first.
    read_float = lambda address: struct.unpack('>f', native.read(address, 4)[0])[0]
    profiles = project_profiles(resources, read_float(0x1b3db34), read_float(0x1b3dc3c))
    member = '/run/mag/ps3/scripts/projectileammo.lua'
    require(game.names.count(member) == 1, 'Missing original projectile ammo resource')
    grenade_penetration = project_grenade_penetration(
        parse(game.extract(game.names.index(member) + 1).decode('utf-8-sig')))
    return dict(schema='mag_domination_helo_damage_v1', retail_elf_sha256=sha(native.raw),
                models_sha256=sha(models_raw), profiles=profiles,
                grenade_material_penetration=grenade_penetration)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ('elf', 'game-archive', 'models', 'output'):
        parser.add_argument('--' + flag, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        require(not output.is_relative_to(TOOLS.parents[1]) and output not in
                {path.resolve() for path in (args.elf, args.game_archive, args.models)},
                'Output must be outside the source checkout and inputs')
        data = (json.dumps(project(args.elf, args.game_archive, args.models.read_bytes()),
                           indent=2, sort_keys=True, allow_nan=False) + '\n').encode()
        if args.check:
            require(output.read_bytes() == data, 'Helo damage rules differ')
        else:
            with output.open('xb') as stream:
                stream.write(data)
        print(json.dumps({'output_sha256': sha(data)}))
    except (KeyError, TypeError, ValueError, OSError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
