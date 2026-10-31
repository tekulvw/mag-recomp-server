#!/usr/bin/env python3
"""Extract the retail skill and structure catalogs from the original game archive.

Reads the decrypted retail game.psarc directly (sha256 pinned) with the existing
literal Lua/ZPR parser; nothing is executed. Item ordinals come from
gameplay/data/retail-items.json (extract-item-catalog.py --game-archive).
--check regenerates in memory and requires byte-identical output files.
Prototype-recovered native constants are carried as flagged, unverified-for-retail.
"""
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import sys

ROOT = Path(__file__).resolve().parents[2]
PARSER = ROOT/'tools/extraction/extract-item-catalog.py'
SCRIPTS = '/run/mag/ps3/'
STRUCTURE_SOURCES = (
    'scripts/vehicles.zpr', 'scripts/vehicle_damagemodel.lua', 'scripts/vehicle_collision_info.lua',
    'scripts/vehicle/green_defensive_emplacement.zpr', 'scripts/vehicle/black_defensive_emplacement.zpr',
    'scripts/vehicle/red_defensive_emplacement.zpr', 'scripts/vehicle/red_defensive_emplacement_no_turret.zpr',
    'scripts/vehicle/sirsat_defensive_emplacement.zpr', 'scripts/active_items.lua', 'scripts/character_skills.zpr',
    'settings/mag_game_settings.lua', 'settings/gameproperties.zpr',
)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    return (json.dumps(value, indent=2)+'\n').encode()


def open_archive(path, parser):
    sys.path.insert(0, str(ROOT/'tools/collision'))
    from archive import Psarc
    raw = path.read_bytes()
    if sha(raw) != parser['RETAIL_ARCHIVE_HASH']:
        raise SystemExit('Wrong retail game.psarc')
    archive = Psarc(raw)
    return sha(raw), lambda name: archive.extract(archive.names.index(SCRIPTS+name)+1)


def render(archive_path, items_path, skills_output, structures_output, native_rules_path):
    parser = runpy.run_path(str(PARSER))
    archive_sha, member = open_archive(archive_path, parser)
    item_bytes = items_path.read_bytes(); items = json.loads(item_bytes)
    if items['application_id'] != 20231 or items['schema_version'] != 2 or items['build']['source_archive_sha256'] != archive_sha:
        raise SystemExit('retail item catalog does not come from this archive; run extract-item-catalog.py --game-archive first')
    skills_raw = member('scripts/character_skills.zpr')
    table = parser['normalize'](parser['Parser'](skills_raw.decode(), 'character_skills.zpr').parse())['MagCharacterSkillTable']
    bindings = {}
    for identity, item in items['items'].items():
        if item.get('name') and item.get('definition'):
            bindings.setdefault(item['name'], []).append(identity)
    skills = {'schema_version': 2, 'application_id': 20231,
              'status': 'authored research configuration; purchases and modifier application not implemented',
              'source_sha256': {'game.psarc': archive_sha, 'character_skills.zpr': sha(skills_raw),
                                'gameplay/data/retail-items.json': sha(item_bytes)},
              'equipment_token_item_refs': bindings, 'table': table}

    source_bytes = {name: member(name) for name in STRUCTURE_SOURCES}
    resources, literals = {}, {}
    for name, raw in source_bytes.items():
        literals[name] = {}
        resources[name] = parser['normalize'](parser['Parser'](raw.decode('utf-8-sig'), name).parse(), literals=literals[name])
    repair_refs = [key for key, row in items['items'].items()
                  if row['definition'] == {'group': 'active_items', 'name': 'repairkit'}]
    if len(repair_refs) > 1:
        raise ValueError('repairkit has several authored namespace bindings')
    globals_ = resources['scripts/active_items.lua']['ActiveItemDataTable']
    skill_rows = resources['scripts/character_skills.zpr']['MagCharacterSkillTable']['Skills']
    repair_skills = {key: row for key, row in skill_rows.items()
                     if row['MagCharacterSkill'].get('Default', {}).get('Modifier') == 'kCharModRepairAmount'}
    profiles = []
    for key, wrapper in resources['scripts/vehicles.zpr']['VehicleTable']['Vehicle_Types'].items():
        row = wrapper['VehicleTypeData']
        if row['Type'] not in ('DefensiveEmplacement', 'DefensiveEmplacementNoTurret'):
            continue
        for faction, faction_key in enumerate(('Faction_Green', 'Faction_Black', 'Faction_Red')):
            for vehicle in row[faction_key]['Vehicle_Data'].values():
                vehicle = vehicle['VehicleData']
                source = 'scripts/vehicle/'+vehicle['PropertiesFileName']
                if source not in resources:
                    raise ValueError('unresolved authored emplacement properties '+source)
                profiles.append({'faction': faction, 'vehicle_type': row['Type'], 'vehicle_row': key,
                                 'name': vehicle['Name'], 'model': vehicle['ModelFileName'],
                                 'properties_source': source,
                                 'damage_properties': resources[source]['DefensiveEmplacementProperties'].get('DamageProperties')})
    damage = {group: {name: {'native_token_u32': row['native_token_u32'], 'damage': row['parameters']['damage']}
                      for name, row in items['definitions'][group].items() if 'damage' in row['parameters']}
              for group in ('hitscan_ammo', 'projectile_ammo')}
    structures = {
        'schema_version': 2, 'application_id': 20231,
        'source_sha256': {'game.psarc': archive_sha, **{name: sha(raw) for name, raw in source_bytes.items()}},
        'item_catalog_sha256': sha(item_bytes),
        'native_rules': json.loads(native_rules_path.read_text()),
        'profiles': profiles,
        'repair': {'item_id': None, 'authored_item_refs': repair_refs,
                   'wire_binding': 'unverified; faction-list local ordinal is not a global wire ID',
                   'parameters': items['definitions']['active_items']['repairkit']['parameters'],
                   'globals': {key: value for key, value in globals_.items() if key.startswith('repairkit_')},
                   'skill_modifiers': repair_skills},
        'ammo_damage': damage,
        'resources': {name: value for name, value in resources.items()
                      if name not in ('scripts/active_items.lua', 'scripts/character_skills.zpr')},
        'source_literals': literals,
        'limitations': ['A profile without authored DamageProperties remains unresolved; no reflection defaults are invented.',
                        'Retail bunker/emplacement placement lives in level data and is not part of this catalog.',
                        'Native rule constants are prototype-derived and unverified for retail.'],
    }
    return {skills_output: encode(skills), structures_output: encode(structures)}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--game-archive', type=Path, required=True, help='decrypted retail game.psarc (sha256 b49c9435...)')
    p.add_argument('--items', type=Path, required=True)
    p.add_argument('--skills-output', type=Path, required=True)
    p.add_argument('--structures-output', type=Path, required=True)
    p.add_argument('--native-rules', type=Path, required=True, help='externally verified structure native_rules object; constants are not in authored resources')
    p.add_argument('--check', action='store_true')
    a = p.parse_args()
    sources = {v.resolve() for v in (a.game_archive, a.items, a.native_rules)}
    outputs = (a.skills_output, a.structures_output)
    if len({v.resolve() for v in outputs}) != 2 or any(v.resolve() in sources for v in outputs):
        p.error('outputs must be distinct and separate from source inputs')
    outputs = render(a.game_archive, a.items, a.skills_output, a.structures_output, a.native_rules)
    if a.check:
        bad = [str(path) for path, raw in outputs.items() if not path.is_file() or path.read_bytes() != raw]
        if bad:
            raise SystemExit('Retail catalogs differ: '+', '.join(bad))
        print('Retail skill and structure catalogs reproduce exactly')
    else:
        for path, raw in outputs.items():
            path.write_bytes(raw); print(path)


if __name__ == '__main__':
    main()
