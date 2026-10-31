#!/usr/bin/env python3
"""Extract retail authored server rules; never execute Lua or infer wire IDs.

Unknown selected syntax fails closed. Executable objective scripts are inventoried
as pending behavior analysis, not converted to configuration or silently skipped.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import runpy
import sys

ROOT = Path(__file__).resolve().parents[2]
PREFIX = '/run/mag/ps3/'
REQUIRED = {
    'scripts/character.zpr', 'scripts/character_class.zpr', 'scripts/character_gameplay.zpr',
    'scripts/character_heads.zpr', 'scripts/character_leadership_abilities.zpr',
    'scripts/character_model_data.zpr', 'scripts/materialproperties.zpr',
    'scripts/character_stats.zpr', 'scripts/character_awards.zpr', 'scripts/weapon_stats.zpr',
    'scripts/armory.zpr', 'scripts/entitled_gear.zpr', 'scripts/game_state_properties.zpr',
    'scripts/mag_squad_matchmaking_properties.zpr', 'scripts/level_boundary.zpr',
    'scripts/hud_properties.zpr', 'scripts/launcher_properties.zpr',
    'scripts/shadowwar_contract_bonuses.zpr', 'scripts/shadowwar_contract_bonus_descriptions.zpr',
    'scripts/vehicles.zpr', 'scripts/training_missions.zpr', 'scripts/training_descriptions.zpr',
    'scripts/streamed_attachments.zpr', 'scripts/streamed_character_class.zpr',
    'scripts/streamed_gear.zpr', 'scripts/streamed_weapons.zpr',
    'settings/gameproperties.zpr', 'settings/mag_game_settings.lua', 'settings/magrootsettings.lua',
}
FAMILIES = ('scripts/abilities/', 'scripts/vehicle/', 'scripts/game_objective_descriptions/')


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    return (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()


def token_overrides(raw):
    # This shipped file is quoted string pairs with // line comments, not Lua.
    text = raw.decode('utf-8-sig')
    result, seen = [], {}
    pattern = re.compile(r'\s*"([^"\r\n]*)"\s*,\s*"([^"\r\n]*)"\s*,?\s*')
    for line_number, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.lstrip().startswith('//'):
            continue
        match = pattern.fullmatch(line)
        if not match:
            raise ValueError(f'Invalid token override at line {line_number}')
        row = {'source': match[1], 'replacement': match[2], 'line': line_number}
        if match[1] in seen:
            row['repeats_source_from_line'] = seen[match[1]]
        seen.setdefault(match[1], line_number)
        result.append(row)
    return result


def extract(archive_path, items_path):
    api = runpy.run_path(str(ROOT / 'tools/extraction/extract-item-catalog.py'))
    sys.path.insert(0, str(ROOT / 'tools/collision'))
    from archive import Psarc
    raw = archive_path.read_bytes()
    if sha(raw) != api['RETAIL_ARCHIVE_HASH']:
        raise ValueError('Wrong retail game.psarc')
    arc = Psarc(raw)
    available = {name.removeprefix(PREFIX): i for i, name in enumerate(arc.names, 1)
                 if name.startswith(PREFIX)}
    missing = REQUIRED - available.keys()
    if missing:
        raise ValueError('Missing selected retail members: ' + repr(sorted(missing)))
    resources, inventory = {}, []
    item_catalog = json.loads(items_path.read_text())
    if item_catalog['application_id'] != 20231 or item_catalog['schema_version'] != 2:
        raise ValueError('Expected namespace-preserving retail item schema2')
    if item_catalog['build']['source_archive_sha256'] != sha(raw):
        raise ValueError('Retail item catalog belongs to a different archive')
    existing = {'scripts/' + name: 'gameplay/data/retail-items.json'
                for name in item_catalog['resources']}
    existing.update({'scripts/character_skills.zpr': 'gameplay/data/retail-skills.json',
                     'scripts/experience.zpr': 'gameplay/data/retail-progression.json',
                     'scripts/vehicle_collision_info.lua': 'gameplay/data/retail-structures.json',
                     'scripts/vehicle_damagemodel.lua': 'gameplay/data/retail-structures.json'})
    for member, index in sorted(available.items()):
        if not member.startswith(('scripts/', 'settings/')) or not member.endswith(('.zpr', '.lua')):
            continue
        content = arc.extract(index)
        row = {'member': member, 'sha256': sha(content), 'bytes': len(content)}
        selected = member in REQUIRED or member.startswith(FAMILIES)
        if selected:
            literals = {}
            parameters = api['normalize'](api['Parser'](content.decode('utf-8-sig'), member).parse(), literals=literals)
            resources[member] = {**row, 'parameters': parameters, 'source_literals': literals}
            row.update(status='authored_extracted_unbound', output='gameplay/data/retail-rules.json')
        elif member == 'scripts/tokenoverride.zpr':
            resources[member] = {**row, 'pairs': token_overrides(content)}
            row.update(status='authored_extracted_unbound', output='gameplay/data/retail-rules.json')
        elif member in existing:
            row.update(status='extracted_by_existing_catalog', output=existing[member])
        elif member.startswith(('scripts/objective_control/', 'scripts/levels/')):
            row.update(status='executable_behavior_analysis_pending', reason='Lua functions are not literal data; native/script control flow must be traced separately')
        elif member in ('scripts/trophies.zpr', 'scripts/net_result_messages.zpr'):
            row.update(status='authored_parser_gap', reason='Arithmetic expressions or non-UTF8 text require a separate evidenced parser/encoding contract')
        else:
            row.update(status='outside_selected_server_tables', reason='Rendering/UI/audio/input or unclassified supporting resource; inventory is not proof of irrelevance')
        inventory.append(row)
    # Name references are deliberately qualified authored identities, never wire IDs.
    by_name = {}
    for identity, item in item_catalog['items'].items():
        if item['name']:
            by_name.setdefault(item['name'], []).append(identity)
    classes, unresolved = [], []
    class_rows = resources['scripts/character_class.zpr']['parameters']['CharacterClassTable']['Classes']
    for identity, wrapper in class_rows.items():
        info = wrapper['CharacterClassInfo']
        kits = info.get('InventoryList', [])
        references = []
        for kit_index, kit in enumerate(kits):
            for slot, name in kit.items():
                if not isinstance(name, str) or not name:
                    continue
                refs = by_name.get(name, [])
                entry = {'kit_index': kit_index, 'slot': slot, 'equipment_name': name, 'authored_item_refs': refs, 'wire_item_id': None}
                references.append(entry)
                if not refs:
                    unresolved.append({'class_ref': identity, **entry})
        classes.append({'class_ref': identity, 'faction': info.get('Faction'), 'class_type': info.get('Class'),
                        'inventory_template_count': len(kits), 'equipment_references': references})
    vehicle_refs = []
    registry = resources['scripts/vehicles.zpr']['parameters']['VehicleTable']['Vehicle_Types']
    for identity, wrapper in registry.items():
        row = wrapper['VehicleTypeData']
        for faction in ('Faction_Green', 'Faction_Black', 'Faction_Red'):
            for variant, data in row.get(faction, {}).get('Vehicle_Data', {}).items():
                filename = data['VehicleData'].get('PropertiesFileName')
                target = 'scripts/vehicle/' + filename if filename else None
                vehicle_refs.append({'vehicle_ref': identity, 'faction_ref': faction, 'variant_ref': variant,
                                     'properties_member': target, 'resolved': target in resources})
    limitations = [
        'Authored configuration only; no account grants, runtime consumers or executable Lua behavior enabled.',
        'Starter templates are not proof of native InitializeLoadouts transformation, runtime wire IDs or item eligibility.',
        'Table order, duplicate assignments, symbolic identifiers and exact scalar literals are retained by the shared parser.',
        'Objective-control behavior, map object graphs, network pool IDs, terrain/collision transforms and native defaults need separate evidence.',
        'Unselected archive members remain visible in coverage; outside_selected_server_tables does not assert they are unnecessary.',
    ]
    rules = {'schema_version': 1, 'application_id': 20231, 'source_archive_sha256': sha(raw),
             'item_catalog_sha256': sha(items_path.read_bytes()),
             'resources': resources, 'starter_templates': classes, 'unresolved_starter_references': unresolved,
             'vehicle_property_references': vehicle_refs, 'limitations': limitations}
    coverage = {'schema_version': 1, 'application_id': 20231, 'source_archive_sha256': sha(raw),
                'scope': 'Every scripts/settings .lua/.zpr member in this pinned global game archive; map archives are a separate extraction boundary',
                'members': inventory, 'counts': dict(sorted(Counter(r['status'] for r in inventory).items())),
                'required_members': sorted(REQUIRED), 'selected_prefixes': list(FAMILIES),
                'limitations': limitations}
    return rules, coverage


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--coverage-output', type=Path, required=True)
    parser.add_argument('--items', type=Path, required=True)
    parser.add_argument('--check', action='store_true', help='Regenerate in memory; write nothing and fail on any mismatch')
    args = parser.parse_args()
    targets = [args.output.resolve(), args.coverage_output.resolve()]
    if len(set(targets)) != 2 or any(p.resolve() in targets for p in (args.game_archive, args.items)):
        raise ValueError('Outputs must be distinct and cannot replace the original archive')
    rules, coverage = extract(args.game_archive, args.items)
    for path, value in ((args.output, rules), (args.coverage_output, coverage)):
        data = encode(value)
        if args.check:
            if path.read_bytes() != data:
                raise ValueError('Catalog differs from original regeneration: ' + str(path))
        else:
            path.write_bytes(data)
    print(json.dumps({'resource_count': len(rules['resources']), 'starter_class_count': len(rules['starter_templates']),
                      'unresolved_starter_references': len(rules['unresolved_starter_references']),
                      'unresolved_vehicle_properties': sum(not r['resolved'] for r in rules['vehicle_property_references']),
                      'coverage_counts': coverage['counts']}))


if __name__ == '__main__':
    main()
