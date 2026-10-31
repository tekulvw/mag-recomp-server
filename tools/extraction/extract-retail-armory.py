#!/usr/bin/env python3
"""Project verified retail wire IDs, native ownership bits and Recruit starter kits."""
import argparse
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
SOURCES = ('gameplay/data/retail-items.json', 'gameplay/data/retail-item-bindings.json',
           'gameplay/data/retail-all-factions-bootstrap-loadouts.json',
           'gameplay/data/retail-skill-purchases.json',
           'gameplay/evidence/retail-skill-unlocks-native-20261026.json')


def check(condition, message):
    if not condition:
        raise ValueError(message)


def unique(pairs):
    result = {}
    for key, value in pairs:
        check(key not in result, 'duplicate_json_key:' + key)
        result[key] = value
    return result


def project(sources):
    docs = {name: json.loads(raw, object_pairs_hook=unique) for name, raw in sources.items()}
    hashes = {name: hashlib.sha256(raw).hexdigest() for name, raw in sources.items()}
    catalog, bindings, bootstrap, skills, proof = (docs[name] for name in SOURCES)
    check(catalog['schema_version'] == 2 and catalog['application_id'] == 20231
          and bootstrap['schema_version'] == 1 and bootstrap['application_id'] == 20231
          and skills['schema_version'] == 2 and skills['application_id'] == 20231,
          'retail_armory_source_application_mismatch')
    check(bindings['validated'] is True and bindings['positions_compared'] == 667
          and bindings['mismatch_count'] == 0
          and bindings['catalog_sha256'] == hashes[SOURCES[0]], 'unverified_retail_item_binding')
    for path in (SOURCES[0], SOURCES[1], SOURCES[4]):
        check(skills['source_sha256'][path] == hashes[path], 'source_hash_mismatch:' + path)
    check(proof['application_id'] == 20231 and proof['equipment_lookup_flag'] == 1
          and proof['available_item_bit_mask'] == 1023
          and skills['provenance']['equipment_projection_complete'] is True,
          'native_available_items_rule_unverified')
    by_ref = bindings['binding_configuration']['by_authored_ref']
    by_bit = skills['network_item_id_to_available_item_bit']
    check(set(by_ref) == set(catalog['items']) and len(by_ref) == 667
          and set(by_bit) == {str(wire) for wire in by_ref.values()}, 'incomplete_retail_item_mapping')
    items = []
    for ref, item in catalog['items'].items():
        wire = by_ref[ref]
        bit = by_bit[str(wire)]
        check(type(wire) is int and 0 <= wire < 4096 and type(bit) is int
              and 0 <= bit < 768 and bit == (wire & 1023), 'invalid_native_available_item_bit:' + ref)
        match = re.fullmatch(r'FactionItemList([0-3]):(\d+)', ref)
        check(match is not None and item['authored_ref'] == ref, 'invalid_authored_reference')
        group = int(match[1])
        name, definition = item['name'], item['definition']
        if not name:
            check(definition is None and item['item_type'] is None, 'unexpected_reserved_definition')
            continue
        # An assigned native item may lack extracted mechanics. Keep its identity;
        # only a resolved weapon definition can supply primary magazine capacity.
        entry = catalog['definitions'][definition['group']][definition['name']] if definition else None
        capacity = entry.get('parameters', {}).get('magazine_capacity') if entry and item['item_type'] == 'kWeapon' and definition['group'] == 'weapons' else None
        capacity = capacity if type(capacity) is int and 1 <= capacity <= 255 else None
        items.append({'wire_id': wire, 'name': name, 'authored_ref': ref,
                      'available_bit': bit, 'factions': [0, 1, 2] if group == 0 else [group - 1],
                      'primary_ammo': capacity})
    check(len(items) == 642 and len({item['wire_id'] for item in items}) == len(items), 'assigned_item_count_mismatch')
    by_wire = {item['wire_id']: item for item in items}
    recruit = skills['table']['Skills']['MagCharacterSkill0']['MagCharacterSkill']
    check(recruit['Cost'] == 0 and recruit['Tier'] == 0, 'unexpected_recruit_skill')
    factions = []
    for faction, name in enumerate(('Green', 'Black', 'Red')):
        data = recruit[name]
        check(data['Available'] is True, 'recruit_unavailable')
        rewards = data.get('Equipment', recruit.get('Default', {}).get('Equipment', {}))
        bits = set()
        for reward in rewards.values():
            token = reward['element']
            ids = skills['equipment_token_item_ids'].get(token)
            if ids is None:
                check(token in skills['equipment_tokens_without_item_ids'], 'unresolved_recruit_equipment:' + token)
                continue
            check(len(ids) == 1 and type(ids[0]) is int and 0 <= ids[0] < 768, 'invalid_recruit_equipment_bit')
            bits.add(ids[0])
        defaults = [row for row in bootstrap['factions'] if row['faction'] == faction]
        check(len(defaults) == 1 and len(defaults[0]['loadouts']) == 5, 'missing_faction_defaults')
        rows = []
        for row in defaults[0]['loadouts']:
            check(len(row['items']) == 22, 'invalid_starter_slot_count')
            for wire in row['items']:
                if wire is not None:
                    check(wire in by_wire and faction in by_wire[wire]['factions'], 'unknown_or_foreign_starter_item')
            if row['items'][0] is not None:
                check(row['primary_ammo'] == by_wire[row['items'][0]]['primary_ammo'], 'starter_ammo_mismatch')
            rows.append({'update_id': row['update_id'], 'items': row['items']})
        factions.append({'faction': faction, 'available_item_bits': sorted(bits), 'skill_ids': [0], 'loadouts': rows})
    return {'schema_version': 1, 'application_id': 20231, 'source_sha256': hashes,
            'items': sorted(items, key=lambda row: row['wire_id']), 'factions': factions}


def render(catalog_dir):
    return (json.dumps(project({name: (catalog_dir / Path(name).name).read_bytes() for name in SOURCES}),
                       indent=2, allow_nan=False) + '\n').encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--catalog-dir', type=Path, required=True, help='external input directory containing ' + ', '.join(Path(v).name for v in SOURCES))
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    payload = render(args.catalog_dir)
    outputs = [args.output]

    for path in outputs:
        check(path.resolve() not in {(args.catalog_dir / Path(name).name).resolve() for name in SOURCES}, 'refusing_source_overwrite')
    if args.check:
        for path in outputs:
            check(path.read_bytes() == payload, 'retail_armory_catalog_differs:' + str(path))
    else:
        for path in outputs:
            path.write_bytes(payload)
    print(json.dumps({'verified': True, 'assigned_items': len(json.loads(payload)['items']),
                      'factions': 3, 'loadouts_per_faction': 5, 'slots': 22}))


if __name__ == '__main__':
    main()
