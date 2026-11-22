#!/usr/bin/env python3
"""Project source-bound Skill 25 launcher rules from owner-supplied retail files."""
import argparse
import math
from pathlib import Path
import runpy

HERE = Path(__file__).resolve().parent
BASE = runpy.run_path(str(HERE / 'project-retail-infantry-rockets.py'))
SOURCE, ROCKETS, ITEMS = (BASE[key] for key in ('SOURCE', 'ROCKETS', 'ITEMS'))
require, sha, encode, parse = (BASE[key] for key in ('require', 'sha', 'encode', 'parse'))
number = BASE['number']
APPLICATION_ID = BASE['APPLICATION_ID']
SKILL_FILE = 'character_skills.zpr'
SKILL = 'MagCharacterSkill25'
COLORS = ('Green', 'Black', 'Red')
RECEIPT_PINS = {
    'motion': '4d594e8c61334e4845df0d3341e976eaaa466259842d851c01f9d0c1a0be052d',
    'ordered_motion': '9b4a70ecec0fc21a2f72f093aff719b3700d8c3714c3735e245d8c871b6123b5',
    'network': 'eb41505bb4b655b135030bbc9e496e109e19d89b577ac22b49a9eb6263e68612',
}


def qualified_native(motion_raw, ordered_raw, network_raw, elf):
    receipts = (motion_raw, ordered_raw, network_raw)
    require(dict(zip(RECEIPT_PINS, map(sha, receipts), strict=True)) == RECEIPT_PINS,
            'unreviewed original guided launcher qualification receipt')
    motion, ordered, network = map(parse, receipts)
    elf_sha = sha(elf.raw)
    require(elf_sha == SOURCE['RETAIL_ELF_SHA256']
            == motion['elf_sha256'] == ordered['retail_elf_sha256'] == network['retail_elf_sha256']
            and ordered['motion_v2_sha256'] == RECEIPT_PINS['motion'],
            'original guided launcher receipts have different source bindings')
    for address, expected in {
        0x7AC5C8: 0x409DFE90,  # Strict active lifetime comparison.
        0x7B40A0: 0x81690024,  # Set the launch target on the selected strategy.
        0x7B42FC: 0x808B0034,  # Select the alternate propulsion template.
        0x481B10: 0xC02900B8,  # Global guided range.
        0x7A92AC: 0xC01E009C,  # Actual vehicle aim offset.
        0x466F2C: 0x38C00098,  # Distinct authored placement field.
        0x1B3DB70: 0x3F4CCCCD, 0x1B3DD4C: 0xC0000000, 0x1B3DB48: 0x3F800000,
    }.items():
        require(elf.word(address) == expected, f'original guided launcher instruction differs at {address:#x}')
    return {
        'retail_elf_sha256': elf_sha,
        'qualification_sha256': RECEIPT_PINS,
        'item_catalog_sha256': motion['items_sha256'],
        'age_grace_seconds': BASE['native_witness'](elf)['age_grace_seconds'],
        'guided_max_range_m': motion['guided_auto_explode_distance_m'],
        'anchor_y_m': motion['aim_anchor_constructor_seed_y_m'],
        'qualified_variants': [
            {name: row[name] for name in ('faction', 'wire_item', 'authored_item', 'item', 'ammo', 'ammo_token')}
            for row in motion['variants']
        ],
    }


def select_skill_pair(skill, color):
    require(skill['Tier'] == 2 and skill['Cost'] == 5 and skill['Category'] == 'kSkillCategoryHeavy',
            'unsupported retail guided-launcher skill')
    grant = skill[color]
    require(grant['Available'] is True and list(grant['Equipment']) == ['Element0', 'Element1'],
            'guided launcher grant must contain the weapon and its required sight')
    return tuple(grant['Equipment'][key]['element'] for key in ('Element0', 'Element1'))


def select_variants(items, ids, original, skill, availability, armory, qualified):
    inventory = {row['wire_id']: row for row in armory['items']}
    default_bits = {row['faction']: row['available_item_bits'] for row in armory['factions']}
    require(len(inventory) == len(armory['items']) and len(default_bits) == len(armory['factions']) == 3
            and list(default_bits) == list(range(3)) and len(qualified) == 3,
            'invalid original guided launcher Armory')
    result = []
    for faction, color, proved in zip(range(3), COLORS, qualified, strict=True):
        weapon_name, sight_name = select_skill_pair(skill, color)
        require(proved['faction'] == faction and proved['item'] == weapon_name,
                'guided launcher skill differs from the qualified equipment')
        group = f'FactionItemList{faction + 1}'
        selected = dict(faction=faction, weapon_name=weapon_name, sight_name=sight_name,
                        ammo_name=proved['ammo'], ammo_token=proved['ammo_token'])
        for name, item_type, definition_group, field in (
            (weapon_name, 'kWeapon', 'weapons', 'item_id'),
            (sight_name, 'kAttachmentItem', 'attachments', 'stock_sight_item_id'),
        ):
            rows = [(ref, row) for ref, row in items.items()
                    if ref.startswith(group + ':') and row['name'] == name]
            require(len(rows) == 1, 'missing or ambiguous guided launcher ItemList member')
            ref, row = rows[0]
            ordinal = ref.split(':', maxsplit=1)[1]
            require(row['authored_ref'] == ref
                    and row['definition'] == {'group': definition_group, 'name': name}
                    and original[group]['items'].get(f'ItemListData{ordinal}') == {'id': name, 'item_type': item_type},
                    'guided launcher ItemList member differs from original')
            wire = ids[ref]
            owned = inventory[wire]
            require(owned['name'] == name and owned['authored_ref'] == ref and owned['factions'] == [faction]
                    and owned['available_bit'] in availability['equipment_token_item_ids'][name]
                    and owned['available_bit'] not in default_bits[faction],
                    'guided launcher gear differs from its purchased Armory binding')
            if field == 'item_id':
                require(wire == proved['wire_item'] and ref == proved['authored_item'] and owned['primary_ammo'] == 1,
                        'guided launcher wire item differs from native proof')
            selected[field] = wire
        result.append(selected)
    return result


def project_steering(ammo):
    def pair(field, minimum, maximum):
        values = ammo[field]
        require(isinstance(values, list) and len(values) == 2, f'invalid guided launcher {field}')
        first, last = (number(value, field, minimum, maximum) for value in values)
        require(first <= last, f'reversed guided launcher {field}')
        return first, last

    guided_min, guided_max = pair('rocketGuidedDecisionTimeRange', .001, 1.)
    dumb_min, dumb_max = pair('rocketDumbDecisionTimeRange', .001, 1.)
    turn_min, turn_max = pair('rocketDumbDirectionChangeRange', 0., 1.)
    maximum = number(ammo['rocketAutoExplodeDistance'], 'dumb range', .01, 1000.)
    cruise = number(ammo['rocketDumbCruiseStraightRange'], 'dumb cruise range', 0., maximum)
    require(cruise < maximum, 'guided launcher cruise range must be below terminal range')
    return dict(guided_decision_min_seconds=guided_min, guided_decision_max_seconds=guided_max,
                guided_max_direction_change=number(ammo['rocketGuidedMaxDirectionChange'],
                                                   'guided direction change', .001, .99),
                dumb_decision_min_seconds=dumb_min, dumb_decision_max_seconds=dumb_max,
                dumb_direction_change_min=turn_min, dumb_direction_change_max=turn_max,
                dumb_cruise_range_m=cruise, dumb_max_range_m=maximum)


def project_ballistics(weapon, ammo, age_grace):
    require(weapon['is_lock_on_rocket_launcher'] is True
            and ammo['propulsion_strategy'] == 'kWepPropulsionStrategyRocketGuided',
            'unsupported original guided launcher strategy')
    steering = project_steering(ammo)
    # The stock validator handles the common physical and damage representation.
    # Only its stock propulsion identity and zero-turn qualification differ.
    common = dict(ammo, propulsion_strategy='kWepPropulsionStrategyRocketDumb',
                  rocketDumbDirectionChangeRange=[0., 0.])
    inventory, flight, blast = BASE['project_ballistics'](weapon, common, age_grace)
    require(inventory['initial_magazine_count'] == 3, 'guided launcher inventory differs')
    return inventory, flight, blast, steering


def project(catalog_raw, binding_raw, bootstrap_raw, skills_raw, armory_raw, read_game, native):
    require(sha(bootstrap_raw) == SOURCE['COMBAT']['BOOTSTRAP_SHA256'],
            'unreviewed original retail bootstrap')
    catalog, ids, members = SOURCE['require_item_binding'](catalog_raw, binding_raw, read_game)
    bootstrap, skills, armory = map(parse, (bootstrap_raw, skills_raw, armory_raw))
    BASE['validate_bootstrap'](bootstrap, sha(catalog_raw), parse(binding_raw)['snapshot_sha256'])
    pins = {'items': sha(catalog_raw), 'item-bindings': sha(binding_raw),
            'all-factions-bootstrap-loadouts': sha(bootstrap_raw), 'skill-purchases': sha(skills_raw)}
    require(native['item_catalog_sha256'] == pins['items'], 'guided items differ from native proof')
    require(armory['schema_version'] == 1 and armory['application_id'] == APPLICATION_ID
            and all(armory['source_sha256'][f'gameplay/data/retail-{name}.json'] == digest
                    for name, digest in pins.items()), 'original guided launcher Armory source differs')
    skill_member = BASE['SCRIPT_ROOT'] + SKILL_FILE
    members[skill_member] = sha(read_game(skill_member))
    skill = BASE['authored_table'](read_game, SKILL_FILE, 'MagCharacterSkillTable')['Skills'][SKILL]['MagCharacterSkill']
    require(skills['schema_version'] == 2 and skills['application_id'] == APPLICATION_ID
            and skills['source_sha256']['game.psarc'] == SOURCE['GAME_ARCHIVE_SHA256']
            and skills['source_sha256'][SKILL_FILE] == members[skill_member]
            and all(skills['source_sha256'][f'gameplay/data/retail-{name}.json'] == pins[name]
                    for name in ('items', 'item-bindings'))
            and skills['table']['Skills'][SKILL]['MagCharacterSkill'] == skill,
            'original guided launcher Skill 25 source differs')
    original = BASE['authored_table'](read_game, 'item_list.zpr', 'ItemList')['factionItemLists']
    selected = select_variants(catalog['items'], ids, original, skill, skills, armory, native['qualified_variants'])
    tables = {}
    for file, (group, kind) in ITEMS['GROUPS'].items():
        if group not in ('weapons', 'projectile_ammo', 'attachments'):
            continue
        member = BASE['SCRIPT_ROOT'] + file
        members[member] = sha(read_game(member))
        require(members[member] == catalog['sources'][file]['sha256'],
                f'original guided launcher source differs for {file}')
        authored = BASE['authored_table'](read_game, file, kind + 'Table')
        if group == 'weapons':
            require(authored['RocketLockOnRange'] == native['guided_max_range_m'],
                    'original global guided launcher range differs')
        tables[group] = BASE['indexed_definitions'](authored, kind)

    def definition(group, name):
        row = tables[group][name]
        require(row == catalog['definitions'][group][name]['parameters'],
                f'original guided launcher definition differs for {group}/{name}')
        return row

    token_member = ROCKETS['TOKEN_OVERRIDE']
    token_data = read_game(token_member)
    tokens, aliases = ROCKETS['project_tokens'](token_data, [row['ammo_name'] for row in selected])
    members[token_member] = sha(token_data)
    require(len(set(tokens.values())) == 3, 'guided launcher ammunition tokens must be distinct')
    variants, witnesses = [], []
    for row in selected:
        ammo_name, sight = row['ammo_name'], row['sight_name']
        weapon, ammo = definition('weapons', row['weapon_name']), definition('projectile_ammo', ammo_name)
        attached = definition('attachments', sight)
        require(weapon['ammo_name'] == ammo_name and weapon['required_sight_attachment'] == sight
                and [entry['WeaponAttachment']['name'] for entry in weapon['attachment_list'].values()] == [sight],
                'guided launcher weapon does not select the original ammo and sight')
        require(tokens[ammo_name] == row['ammo_token']
                == catalog['definitions']['projectile_ammo'][ammo_name]['native_token_u32'],
                'original guided launcher ammo token differs')
        inventory, flight, blast, steering = project_ballistics(weapon, ammo, native['age_grace_seconds'])
        variants.append({key: row[key] for key in ('faction', 'item_id', 'stock_sight_item_id', 'ammo_token')}
                        | dict(weapon=inventory, flight=flight, blast=blast, steering=steering))
        witnesses.append(row | {'weapon_definition_sha256': sha(encode(weapon)),
                                'ammo_definition_sha256': sha(encode(ammo)),
                                'sight_definition_sha256': sha(encode(attached))})
    anchors = native['anchor_y_m']
    rules = {
        'schema': 'mag_retail_guided_launchers_v1', 'application_id': APPLICATION_ID,
        'source': {
            'retail_elf_sha256': native['retail_elf_sha256'],
            'retail_archive_sha256': SOURCE['GAME_ARCHIVE_SHA256'],
            'item_catalog_sha256': pins['items'], 'item_bindings_sha256': pins['item-bindings'],
            'bootstrap_sha256': pins['all-factions-bootstrap-loadouts'],
            'skill_catalog_sha256': pins['skill-purchases'], 'native_proof_sha256': sha(encode(native)),
        },
        'guidance': {'guided_max_range_m': native['guided_max_range_m'],
                     'anchor_policy': 'retail_constructor_seed',
                     'anchor_y_m': dict(ground=anchors['ground'], helo=anchors['helo'],
                                        hospital=anchors['defensive_emplacement'])},
        'variants': variants,
    }
    proof = {
        'format': 'mag_retail_guided_launcher_proof_v1', 'rules_sha256': sha(encode(rules)),
        'native': native, 'source_members_sha256': members, 'token_override': aliases,
        'armory_sha256': sha(armory_raw), 'skill_grant': SKILL, 'variants': witnesses,
    }
    return rules, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    names = ('elf', 'game-archive', 'item-catalog', 'item-bindings', 'bootstrap-loadouts',
             'skill-purchases', 'armory', 'motion-qualification', 'ordered-motion-qualification',
             'network-qualification')
    for name in (*names, 'output', 'proof-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [getattr(args, name.replace('-', '_')) for name in names]
    try:
        SOURCE['output_paths'](inputs, [args.output, args.proof_output])
        elf = SOURCE['NATIVE']['Elf'](args.elf, 'retail')
        game_raw = args.game_archive.read_bytes()
        require(sha(game_raw) == SOURCE['GAME_ARCHIVE_SHA256'], 'wrong original retail game archive')
        game = SOURCE['NATIVE']['Psarc'](game_raw)

        def read_game(name):
            return SOURCE['archive_member'](game, name)

        native = qualified_native(*(path.read_bytes() for path in inputs[7:]), elf)
        rules, proof = project(*(path.read_bytes() for path in inputs[2:7]), read_game, native)
        SOURCE['write_outputs'](inputs, [(args.output, encode(rules)), (args.proof_output, encode(proof))],
                                check=args.check)
    except (KeyError, ValueError, TypeError, OSError) as exc:
        parser.error(str(exc))
    print('Verified three original Skill 25 guided launcher variants')


if __name__ == '__main__':
    main()
