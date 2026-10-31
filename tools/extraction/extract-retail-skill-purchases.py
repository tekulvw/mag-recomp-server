#!/usr/bin/env python3
# Adapted project-owned projector; precursor source SHA256: 68b24ec9adb0a707f5b6cad03fcb1c2441f55e42fe38d47f1a9d3f2715f25176
"""Project retail skill purchases with original-native equipment bit semantics.

AwardSkill requests lookup flag1: the first matching encoded item ID &1023 is
the AvailableItems bit. Missing native tokens are skipped while awarding the
skill. Retain full authored costs, tiers, availability, and modifier metadata.
"""
import argparse
import hashlib
import json
import runpy
from pathlib import Path

PINS = {
    'gameplay/evidence/retail-skill-unlocks-native-20261026.json': 'd60eee0d912eb398897cefa65a630cc914319b7166fac1be2ed26ebce8d906c2',
    'gameplay/data/retail-skills.json': 'fbefcac2b9630f60f36843ecf0ed27f9b6a5144d8b2074f8535a11707769b6ef',
    'gameplay/data/retail-items.json': 'bad8cab139db06380d6a7e8f700150dda5db43099da6d8fd6fcd0342881fc803',
    'gameplay/data/retail-item-bindings.json': '232cbc052e6eb3f0ad84f6c2554336415c934b1ba5f41b758627fc999dd9788b',
    'gameplay/data/retail-itemlist-native-order.json': '5e887016d84107ecf329c77ad4d6fefee728450ab791ac06249c8b0d7a53e15f',
    'gameplay/evidence/dev-itemlist-globalindex-49242c-20261024.json': '075490b3520107ada987b6ba8741ef1dcc3656c1d01ac0ec1afa01fd83aa52d2',
}
ARCHIVE_SHA = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'
SKILL_MEMBER_SHA = 'c976edaaf851eb89783bf6f8746636e4ac2089e5ff64b375c79f597af39e84fd'


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
    """Accept exact retained sources; project original lookup/skip semantics."""
    for name, expected in PINS.items():
        check(hashlib.sha256(sources[name]).hexdigest() == expected,
              'source_hash_mismatch:' + name)
    docs = {name: json.loads(raw, object_pairs_hook=unique) for name, raw in sources.items()}
    skills = docs['gameplay/data/retail-skills.json']
    items = docs['gameplay/data/retail-items.json']
    report = docs['gameplay/data/retail-item-bindings.json']
    snapshot = docs['gameplay/data/retail-itemlist-native-order.json']
    check(skills['schema_version'] == 2 and skills['application_id'] == 20231,
          'retail_skill_schema_required')
    check(skills['source_sha256'] == {
        'game.psarc': ARCHIVE_SHA, 'character_skills.zpr': SKILL_MEMBER_SHA,
        'gameplay/data/retail-items.json': PINS['gameplay/data/retail-items.json']},
        'retail_skill_provenance_mismatch')
    check(items['application_id'] == 20231 and report['validated'] is True
          and report['positions_compared'] == 667 and report['mismatch_count'] == 0
          and report['snapshot_sha256'] == PINS['gameplay/data/retail-itemlist-native-order.json']
          and report['catalog_sha256'] == PINS['gameplay/data/retail-items.json']
          and snapshot['counts'] == [16, 220, 207, 224], 'retail_binding_mismatch')

    native = docs['gameplay/evidence/retail-skill-unlocks-native-20261026.json']
    check(native['equipment_lookup_flag'] == 1 and native['available_item_bit_mask'] == 1023,
          'native_skill_bit_rule_mismatch')
    binding = report['binding_configuration']
    by_name = binding['first_match_by_name']
    aliases = {name: [wire & 1023] for name, wire in by_name.items()}
    network_to_bit = {str(wire): wire & 1023 for wire in binding['by_authored_ref'].values()}
    check(len(network_to_bit) == 667 and all(0 <= bit < 768 for bit in network_to_bit.values()),
          'available_item_bit_out_of_bounds')
    categories = {row['MagCharacterSkillTier']['Category'] for row in skills['table']['Tiers'].values()}
    for key, wrapper in skills['table']['Skills'].items():
        row = wrapper['MagCharacterSkill']
        check(key.startswith('MagCharacterSkill') and 0 <= int(key[17:]) <= 127, 'skill_id_out_of_range')
        if row:
            check(type(row['Tier']) is int and 0 <= row['Tier'] <= 5
                  and type(row['Cost']) is int and 0 <= row['Cost'] <= 255
                  and row['Category'] in categories, 'skill_parser_bounds_mismatch:' + key)
    rewards = {}
    for key, wrapper in skills['table']['Skills'].items():
        row = wrapper['MagCharacterSkill']
        for faction in ('Default', 'Green', 'Black', 'Red'):
            for element in row.get(faction, {}).get('Equipment', {}).values():
                name = element['element']
                rewards.setdefault(name, []).append({'skill': key, 'faction': faction})
    no_award = []
    missing_evidence = {}
    token_fn = runpy.run_path(str(Path(__file__).with_name('extract-item-catalog.py')))['native_token']
    native_tokens = {int(row['tokens_hex'][i:i+8], 16)
                     for row in snapshot['lists'] for i in range(0, len(row['tokens_hex']), 8)}
    for name in sorted(rewards):
        refs = skills['equipment_token_item_refs'].get(name, [])
        check(all(ref in items['items'] and items['items'][ref]['name'] == name for ref in refs),
              'reward_authored_reference_mismatch:' + name)
        if refs:
            check(name in aliases and aliases[name], 'reward_native_binding_missing:' + name)
        else:
            token = token_fn(name)[1]
            check(name not in aliases and token not in native_tokens,
                  'missing_name_has_native_token_match:' + name)
            no_award.append(name)
            missing_evidence[name] = {'token_hex': f'{token:08x}', 'uses': rewards[name],
                                      'reason': 'token absent from all667 native positions; native skips item'}
    return {
        'schema_version': 2,
        'application_id': 20231,
        'status': 'retail authored purchases with original-native equipment bit lookup and missing-token skip',
        'source_sha256': {**skills['source_sha256'], **PINS},
        'provenance': {
            'equipment_id_semantics': 'AvailableItems[24] bit index below768; first encoded native item ID &1023',
            'equipment_projection_complete': True,
            'native_binding_scope': '667 owner-attested faction-local item positions',
            'native_global_ordinal_evidence_pending': False,
            'native_rule_evidence': 'gameplay/evidence/retail-skill-unlocks-native-20261026.json',
            'item_source_counts': snapshot['counts'],
            'missing_native_equipment_tokens': missing_evidence,
        },
        'equipment_token_item_ids': aliases,
        'network_item_id_to_available_item_bit': network_to_bit,
        'equipment_tokens_without_item_ids': no_award,
        'unresolved_equipment_rewards': {},
        'table': skills['table'],
    }




def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, required=True,
                        help='JSON mapping each required logical source name to an explicit qualified file path')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--require-complete-equipment', action='store_true',
                        help='require no remaining unproved equipment rewards')
    args = parser.parse_args()
    paths = json.loads(args.inputs.read_bytes(), object_pairs_hook=unique)
    check(set(paths) == set(PINS), 'incomplete_input_manifest')
    paths = {name: Path(path) for name, path in paths.items()}
    payload = (json.dumps(project({name: path.read_bytes() for name, path in paths.items()}), indent=2, allow_nan=False) + '\n').encode()
    result = json.loads(payload)
    if args.require_complete_equipment and result['unresolved_equipment_rewards']:
        raise SystemExit('Retail AvailableItems equipment binding evidence is unresolved')
    outputs = [args.output]
    check(args.output.resolve() != args.inputs.resolve(), 'input_output_collision')
    for path in outputs:
        check(path.resolve() not in {source.resolve() for source in paths.values()},
              'refusing_source_overwrite')
    if args.check:
        for path in outputs:
            check(path.read_bytes() == payload, 'retail_skill_purchase_catalog_differs:' + str(path))
    else:
        check(all(not path.exists() for path in outputs), 'refusing_existing_output')
        for path in outputs:
            with path.open('xb') as output:
                output.write(payload)
    print(json.dumps({'verified': True, 'skill_rows': len(result['table']['Skills']),
                      'equipment_projection_complete': result['provenance']['equipment_projection_complete'],
                      'unresolved_reward_tokens': len(result['unresolved_equipment_rewards'])}))


if __name__ == '__main__':
    main()
