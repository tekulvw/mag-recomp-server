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


def render(assets, items_path, native_rules_path):
    parser = runpy.run_path(str(PARSER))
    source_paths = {name: (assets.parent/name if name.startswith('settings/') else assets/name) for name in SOURCES}
    source_bytes = {name: path.read_bytes() for name, path in source_paths.items()}
    resources = {}; literals = {}; locations = []
    for name, raw in source_bytes.items():
        node = parser['Parser'](raw.decode('utf-8-sig'), name).parse()
        literals[name] = {}
        resources[name] = parser['normalize'](node, literals=literals[name])
        for _, child, pointer in parser['walk'](node):
            if isinstance(child, parser['Scalar']):
                locations.append({'source': name, 'pointer': pointer, 'line': child.line, 'literal': child.literal})
    item_bytes = items_path.read_bytes(); items = json.loads(item_bytes)
    repair_ids = [int(key) for key, row in items['items'].items()
                  if row['definition'] == {'group': 'active_items', 'name': 'repairkit'}]
    if len(repair_ids) != 1:
        raise ValueError('repairkit must have exactly one authored global item binding')
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
        'schema_version': 1, 'application_id': 20230,
        'source_sha256': {name: sha(raw) for name, raw in source_bytes.items()},
        'item_catalog_sha256': sha(item_bytes),
        'native_rules': json.loads(native_rules_path.read_text()),
        'profiles': profiles,
        'repair': {'item_id': repair_ids[0], 'parameters': repair,
                   'globals': {key: value for key, value in globals_.items() if key.startswith('repairkit_')},
                   'skill_modifiers': repair_skills},
        'ammo_damage': damage,
        'resources': {name: value for name, value in resources.items() if name not in ('active_items.lua', 'character_skills.zpr')},
        'source_literals': literals,
        'limitations': ['A profile without authored DamageProperties remains unresolved; no reflection defaults are invented.',
                        'Demolition objectives and vehicle emplacement bodies have separate runtime objects.',
                        'Hit-location enum meaning, team gates, collision and wire behavior require native/runtime evidence.'],
    }
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
    p.add_argument('--assets', type=Path, required=True, help='explicit prototype scripts directory; sibling settings directory required')
    p.add_argument('--items', type=Path, required=True)
    p.add_argument('--native-rules', type=Path, required=True, help='externally qualified native_rules object; no durability constants are bundled')
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--check', action='store_true')
    a = p.parse_args()
    sources = {a.items.resolve(), a.native_rules.resolve()}
    sources.update((a.assets.parent/name if name.startswith('settings/') else a.assets/name).resolve() for name in SOURCES)
    if a.output.resolve() in sources:
        raise ValueError('input_output_collision')
    payload = render(a.assets, a.items, a.native_rules)
    if a.check:
        if a.output.read_bytes() != payload:
            raise ValueError('structure_catalog_differs')
    else:
        with a.output.open('xb') as stream:
            stream.write(payload)
    print(json.dumps({'projected': True, 'check': a.check}))



if __name__ == '__main__':
    main()
