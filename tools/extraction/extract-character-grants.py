#!/usr/bin/env python3
"""Export bounded local starter grants from authored initial skills and default equipment."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import runpy

REPO = Path(__file__).resolve().parents[2]
FACTIONS = {0: 'Green', 1: 'Black', 2: 'Red'}


def render(SKILLS, ITEMS, LOADOUTS):
    source_bytes = {path: path.read_bytes() for path in [SKILLS, ITEMS, LOADOUTS]}
    parser = runpy.run_path(str(REPO / 'tools/extraction/extract-item-catalog.py'))
    authored = parser['normalize'](parser['Parser'](source_bytes[SKILLS].decode(), 'character_skills.zpr').parse())
    catalog = json.loads(source_bytes[ITEMS]); defaults = json.loads(source_bytes[LOADOUTS])
    if catalog['application_id'] != 20230 or defaults['application_id'] != 20230:
        raise ValueError('prototype starter grants require application20230')
    names = {}
    for ordinal, item in catalog['items'].items():
        if item['name'] and item['definition']:
            names.setdefault(item['name'], []).append(int(ordinal))
    factions = []
    for faction in defaults['factions']:
        faction_id = faction['faction']; color = FACTIONS[faction_id]
        item_ids = set(); skill_ids = []; rewards = []
        for key, row in authored['MagCharacterSkillTable']['Skills'].items():
            skill = row['MagCharacterSkill']; availability = skill.get(color, {})
            if skill.get('Tier') != 0 or availability.get('Available') is not True:
                continue
            match = re.fullmatch(r'MagCharacterSkill(\d+)', key)
            if match is None or int(match[1]) >= 160:
                raise ValueError('authored starter skill ordinal exceeds native five-word mask')
            skill_id = int(match[1]); skill_ids.append(skill_id); equipment = []
            for value in availability.get('Equipment', {}).values():
                name = value['element']; bindings = names.get(name, [])
                if len(bindings) != 1 or not 0 <= bindings[0] < 768:
                    raise ValueError(f'ambiguous/unresolved/out-of-range starter equipment {name!r}')
                item_ids.add(bindings[0]); equipment.append({'name': name, 'item_id': bindings[0]})
            rewards.append({'skill_id': skill_id, 'equipment': equipment})
        skill_equipment_ids = set(item_ids)
        for loadout in faction['loadouts']:
            for item_id in loadout['items']:
                if item_id < 0: continue
                if not 0 <= item_id < 768 or not catalog['items'][str(item_id)]['definition']:
                    raise ValueError('default equipment is not an assigned native availability ordinal')
                item_ids.add(item_id)
        factions.append({'faction': faction_id, 'available_item_ids': sorted(item_ids), 'skill_ids': sorted(skill_ids),
                         'authored_skill_rewards': rewards, 'local_default_equipment_ids': sorted(item_ids - skill_equipment_ids)})
    result = {'schema_version': 1, 'application_id': 20230,
              'policy': {'skills': 'Authored Tier0 skills available to the faction, matching native CreateCharacterRecord.',
                         'available_items': 'Those skill equipment rewards plus existing authored default-kit equipment.',
                         'default_equipment': 'Local starter policy; these extra default IDs are not additional skill rewards.',
                         'excluded': 'Other skills, other armory equipment and broader free cosmetics are not granted.'},
              'factions': factions,
              'source_sha256': {path.name: hashlib.sha256(contents).hexdigest() for path, contents in source_bytes.items()},
              'original_equipment_source_sha256': defaults['source_sha256']}
    return (json.dumps(result, indent=2) + '\n').encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skills', type=Path, required=True)
    parser.add_argument('--items', type=Path, required=True)
    parser.add_argument('--loadouts', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [args.skills, args.items, args.loadouts]
    if args.output.resolve() in {v.resolve() for v in inputs}:
        parser.error('output must not replace source inputs')
    payload = render(*inputs)
    if args.check:
        if args.output.read_bytes() != payload:
            raise ValueError('catalog differs from source regeneration')
    else:
        with args.output.open('xb') as stream:
            stream.write(payload)

if __name__ == '__main__':
    main()
