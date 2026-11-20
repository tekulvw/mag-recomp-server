#!/usr/bin/env python3
# Adapted project-owned projector; precursor source SHA256: 8636a812b4cccbf6c6f1d5d3394decbfa2d26d5bb1cce4844be86e66363fc45f
"""Extract authored structure durability, repair and armor configuration without executing Lua.

--check reconstructs the catalog in memory and requires exact byte identity.
Native interpretation requires explicit external rules; absent authored properties stay absent.
"""
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import sys

PARSER = Path(__file__).with_name('extract-item-catalog.py')
SOURCES = (
    'vehicles.zpr', 'vehicle_damagemodel.lua', 'vehicle_collision_info.lua',
    'vehicle/green_defensive_emplacement.zpr', 'vehicle/black_defensive_emplacement.zpr',
    'vehicle/red_defensive_emplacement.zpr', 'vehicle/red_defensive_emplacement_no_turret.zpr',
    'vehicle/sirsat_defensive_emplacement.zpr', 'active_items.lua', 'character_skills.zpr',
    'settings/mag_game_settings.lua', 'settings/gameproperties.zpr',
)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    return (json.dumps(value, indent=2)+'\n').encode()


def asset_paths(assets):
    return {name: (assets.parent/name if name.startswith('settings/') else assets/name)
            for name in SOURCES}


def retail_source_bytes(archive_path, parser):
    archive_raw = archive_path.read_bytes()
    digest = sha(archive_raw)
    if digest != parser['RETAIL_ARCHIVE_HASH']:
        raise ValueError('wrong retail game.psarc')
    sys.path.insert(0, str(PARSER.parent.parent / 'collision'))
    from archive import Psarc
    archive = Psarc(archive_raw)
    source_bytes = {}
    for name in SOURCES:
        member = '/run/mag/ps3/' + (name if name.startswith('settings/') else 'scripts/' + name)
        if archive.names.count(member) != 1:
            raise ValueError('missing or ambiguous retail member ' + member)
        source_bytes[name] = archive.extract(archive.names.index(member) + 1)
    return source_bytes, digest


def retail_repair_id(items, items_digest, binding_raw, archive_digest):
    receipt = json.loads(binding_raw)
    bound = receipt.get('binding_configuration', {})
    if (items.get('schema_version') != 2 or items.get('application_id') != 20231
            or items.get('build', {}).get('source_archive_sha256') != archive_digest
            or receipt.get('validated') is not True or receipt.get('mismatch_count') != 0
            or receipt.get('catalog_sha256') != items_digest
            or bound.get('application_id') != 20231
            or bound.get('source_catalog_sha256') != items_digest):
        raise ValueError('unqualified retail repair item binding')
    refs = [value for value, row in items['items'].items()
            if row['definition'] == {'group': 'active_items', 'name': 'repairkit'}]
    item = bound.get('first_match_by_name', {}).get('repairkit')
    ids = {bound.get('by_authored_ref', {}).get(ref) for ref in refs}
    if type(item) is not int or not 0 <= item < 1024 or ids != {item}:
        raise ValueError('retail repair item binding did not resolve')
    return item


def render(assets, items_path, native_rules_path, *, game_archive=None, retail_bindings=None):
    parser = runpy.run_path(str(PARSER))
    archive_digest = None
    if game_archive is None:
        source_bytes = {name: path.read_bytes() for name, path in asset_paths(assets).items()}
    else:
        source_bytes, archive_digest = retail_source_bytes(game_archive, parser)
    resources = {}; literals = {}; locations = []
    for name, raw in source_bytes.items():
        node = parser['Parser'](raw.decode('utf-8-sig'), name).parse()
        literals[name] = {}
        resources[name] = parser['normalize'](node, literals=literals[name])
        for _, child, pointer in parser['walk'](node):
            if isinstance(child, parser['Scalar']):
                locations.append({'source': name, 'pointer': pointer, 'line': child.line, 'literal': child.literal})
    item_bytes = items_path.read_bytes(); items = json.loads(item_bytes)
    binding_raw = None
    if archive_digest is None:
        if items.get('application_id', 20230) != 20230:
            raise ValueError('prototype structure source requires prototype items')
        repair_ids = [int(key) for key, row in items['items'].items()
                      if row['definition'] == {'group': 'active_items', 'name': 'repairkit'}]
        if len(repair_ids) != 1:
            raise ValueError('repairkit must have exactly one authored global item binding')
        repair_id = repair_ids[0]
    else:
        binding_raw = retail_bindings.read_bytes()
        repair_id = retail_repair_id(items, sha(item_bytes), binding_raw, archive_digest)
    repair = items['definitions']['active_items']['repairkit']['parameters']
    globals_ = resources['active_items.lua']['ActiveItemDataTable']
    skills = resources['character_skills.zpr']['MagCharacterSkillTable']['Skills']
    repair_skills = {key: row for key, row in skills.items()
                     if row['MagCharacterSkill'].get('Default', {}).get('Modifier') == 'kCharModRepairAmount'}
    profiles = []
    for key, wrapper in resources['vehicles.zpr']['VehicleTable']['Vehicle_Types'].items():
        row = wrapper['VehicleTypeData']
        if row['Type'] not in ('DefensiveEmplacement', 'DefensiveEmplacementNoTurret'):
            continue
        for faction, faction_key in enumerate(('Faction_Green', 'Faction_Black', 'Faction_Red')):
            for vehicle in row[faction_key]['Vehicle_Data'].values():
                vehicle = vehicle['VehicleData']
                source = 'vehicle/'+vehicle['PropertiesFileName']
                if source not in resources:
                    raise ValueError('unresolved authored emplacement properties '+source)
                props = resources[source]['DefensiveEmplacementProperties']
                profiles.append({'faction': faction, 'vehicle_type': row['Type'], 'vehicle_row': key,
                                 'name': vehicle['Name'], 'model': vehicle['ModelFileName'],
                                 'properties_source': source, 'damage_properties': props.get('DamageProperties')})
    damage = {}
    for group in ('hitscan_ammo', 'projectile_ammo'):
        damage[group] = {name: {'native_token_u32': row['native_token_u32'],
                               'damage': row['parameters']['damage']}
                         for name, row in items['definitions'][group].items() if 'damage' in row['parameters']}
    catalog = {
        'schema_version': 1, 'application_id': 20231 if archive_digest is not None else 20230,
        'source_sha256': {name: sha(raw) for name, raw in source_bytes.items()},
        'item_catalog_sha256': sha(item_bytes),
        'native_rules': json.loads(native_rules_path.read_text()),
        'profiles': profiles,
        'repair': {'item_id': repair_id, 'parameters': repair,
                   'globals': {key: value for key, value in globals_.items() if key.startswith('repairkit_')},
                   'skill_modifiers': repair_skills},
        'ammo_damage': damage,
        'resources': {name: value for name, value in resources.items() if name not in ('active_items.lua', 'character_skills.zpr')},
        'source_literals': literals,
        'limitations': ['A profile without authored DamageProperties remains unresolved; no reflection defaults are invented.',
                        'Demolition objectives and vehicle emplacement bodies have separate runtime objects.',
                        'Hit-location enum meaning, team gates, collision and wire behavior require native/runtime evidence.'],
    }
    if archive_digest is not None:
        catalog['source_archive_sha256'] = archive_digest
        catalog['retail_item_binding_sha256'] = sha(binding_raw)
    rules = catalog['native_rules']
    thresholds = rules['health_condition_threshold_percent']
    if (len(thresholds) != 3 or any(type(v) not in (int, float) or not 0 < v < 100 for v in thresholds)
            or not thresholds[0] > thresholds[1] > thresholds[2]
            or not 0 < rules['dead_below_health'] <= 1023.75
            or not 0 < rules['minimum_rebuilt_health_percent'] <= 100):
        raise ValueError('invalid_external_native_rules')
    catalog['native_rules_sha256'] = sha(native_rules_path.read_bytes())
    return encode(catalog)



def main():
    p = argparse.ArgumentParser(description=__doc__)
    source_group = p.add_mutually_exclusive_group(required=True)
    source_group.add_argument('--assets', type=Path, help='prototype scripts directory with sibling settings')
    source_group.add_argument('--game-archive', type=Path, help='original retail game.psarc')
    p.add_argument('--retail-item-bindings', type=Path)
    p.add_argument('--items', type=Path, required=True)
    p.add_argument('--native-rules', type=Path, required=True, help='externally qualified native_rules object; no durability constants are bundled')
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--check', action='store_true')
    a = p.parse_args()
    sources = {a.items.resolve(), a.native_rules.resolve()}
    if a.game_archive is not None:
        if a.retail_item_bindings is None:
            p.error('--game-archive requires --retail-item-bindings')
        sources.update((a.game_archive.resolve(), a.retail_item_bindings.resolve()))
    else:
        if a.retail_item_bindings is not None:
            p.error('--retail-item-bindings requires --game-archive')
        sources.update(path.resolve() for path in asset_paths(a.assets).values())
    if a.output.resolve() in sources:
        raise ValueError('input_output_collision')
    payload = render(a.assets, a.items, a.native_rules, game_archive=a.game_archive,
                     retail_bindings=a.retail_item_bindings)
    if a.check:
        if a.output.read_bytes() != payload:
            raise ValueError('structure_catalog_differs')
    else:
        with a.output.open('xb') as stream:
            stream.write(payload)
    print(json.dumps({'projected': True, 'check': a.check}))



if __name__ == '__main__':
    main()
