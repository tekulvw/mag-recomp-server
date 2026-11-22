#!/usr/bin/env python3
"""Project original rifle underbarrel launcher equipment and authored ammo."""
import argparse
from pathlib import Path
import runpy

HERE = Path(__file__).resolve().parent
BASE = runpy.run_path(str(HERE / 'project-retail-infantry-rockets.py'))
SOURCE, ITEMS, ROCKETS = (BASE[name] for name in ('SOURCE', 'ITEMS', 'ROCKETS'))
require, sha, encode, parse, number = (BASE[name] for name in ('require', 'sha', 'encode', 'parse', 'number'))
DAMAGE_CHANNELS = ('character', 'characterAPierce', 'characterARedux',
                   'vehicle', 'vehicleAPierce', 'vehicleARedux',
                   'objective', 'objectiveAPierce', 'objectiveARedux', 'mine')
AMMO_STRATEGIES = {
    'propulsion_strategy': 'kWepPropulsionStrategyThrown',
    'detonation_strategy': 'kWepDetonationStrategyImpact',
    'explosion_strategy': 'kWepExplosionStrategyDebris',
    'collisionType': 'kWepProjCollisionShapeCapsule',
}
AUTHORED_SCALARS = (
    'launch_velocity', 'm_fArmedTime', 'm_fLifetimeMax', 'detonation_timer', 'radius',
    'mass', 'friction', 'restitution', 'linearDamping', 'linearDampingOnGround',
    'angularDamping', 'angularDampingOnGround', 'fBounceDampingDefault',
    'fBounceDampingCharacter', 'm_fBounceToExplode', 'm_fImpactDamage',
    'full_damage_radius', 'half_damage_radius', 'MaterialPenetration',
)


def project_weapon(host, attachment, weapon):
    require(host['category'] == 'kWepCategoryAssaultRifle'
            and attachment['attachment_type'] == 'kAttachTypeLauncher'
            and attachment['character_attachment_type'] == 'kCharAttachTypePrimary'
            and attachment['id'] in {row['WeaponAttachment']['name']
                                     for row in host['attachment_list'].values()}
            and weapon['id'] == attachment['weapon_name'] != host['id']
            and weapon['category'] == 'kWepCategoryGrenadeLauncher'
            and weapon['ammo_name'] != host['ammo_name'],
            'expected compatible launcher attachment with separate WeaponData')
    size, count = weapon['magazine_capacity'], weapon['initial_magazine_count']
    require(weapon['infinite_ammo'] is False and type(size) is int and type(count) is int
            and size > 0 and count > 0 and size * count <= 255,
            'expected finite launcher ammunition')
    mode = weapon['fire_mode']
    return {'magazine_capacity': size, 'initial_magazine_count': count,
            'refire_seconds': number(mode['refire_time'], 'refire_time', 0.001, 60),
            'reload_seconds': number(mode['reload_time'], 'reload_time', 0.001, 120)}


def project_damage(damage):
    require(set(damage) == set(DAMAGE_CHANNELS), 'unexpected launcher damage channels')
    return {channel: number(damage[channel], channel, 0, 100_000) for channel in DAMAGE_CHANNELS}


def project_ammo(ammo, token):
    require(all(ammo[field] == expected for field, expected in AMMO_STRATEGIES.items()),
            'unsupported original launcher strategy')
    spine = ammo['capsuleHeight']
    require(isinstance(spine, list) and len(spine) == 3, 'invalid capsule vector')
    fields = {key: number(ammo[key], key, 0, 100_000) for key in AUTHORED_SCALARS}
    require(fields['launch_velocity'] > 0 and fields['radius'] > 0 and fields['mass'] > 0
            and fields['m_fLifetimeMax'] > fields['m_fArmedTime']
            and fields['full_damage_radius'] >= fields['half_damage_radius'] > 0,
            'invalid authored projectile bounds')
    require(type(ammo['destroy_objective']) is bool, 'invalid objective option')
    return {'name': ammo['id'], 'token': token, 'authored': {
        **AMMO_STRATEGIES, **fields,
        'capsuleHeight': [number(value, 'capsuleHeight', -10, 10) for value in spine],
        'damage': project_damage(ammo['damage']),
        'destroy_objective': ammo['destroy_objective'],
    }}


def native_witness(elf):
    require(sha(elf.raw) == SOURCE['RETAIL_ELF_SHA256'], 'wrong original retail ELF')
    for site, value in {
        0x49C4CC: 0x93C10074, 0x1B3FAD8: 0x016296E0,
        0x62EC34: 0x38800011, 0x62EC48: 0x38C00011,
        0x7678AC: 0x38800011, 0x7678D0: 0x7F36CB78,
        0x7676F4: 0x9AC10160, 0x453FCC: 0x38800210,
    }.items():
        require(elf.word(site) == value, f'original launcher producer differs at {site:#x}')
    for site, name in ((0x16296E0, b'kAttachTypeLauncher'), (0x1637EF8, b'kItemSlotPrimaryUnderbarrel')):
        require(elf.read(site, len(name) + 1)[0] == name + b'\0', 'original launcher role differs')
    spans = {
        'launcher_role': (0x49C488, 0x49C4D4),
        'bind_slot_17': (0x62EC18, 0x62EC50),
        'select_slot_17': (0x76787C, 0x7678D8),
        'request_producer': (0x7676D8, 0x7677BC),
        'request_constructor': (0x453FCC, 0x45401C),
    }
    return {name: {'start': start, 'end_exclusive': end,
                   'sha256': sha(elf.read(start, end - start)[0])}
            for name, (start, end) in spans.items()}


def upgraded_host(namespace, original, grant, weapons):
    require(grant['Available'] is True, 'original tier-two rifle unlock unavailable')
    names = {entry['element'] for entry in grant['Equipment'].values()}
    matches = [(ordinal, row['id']) for ordinal, row in original.items()
               if row.get('item_type') == 'kWeapon' and row.get('id') in names
               and weapons.get(row['id'], {}).get('category') == 'kWepCategoryAssaultRifle']
    require(len(matches) == 1, 'expected one original tier-two rifle grant per faction')
    ordinal, name = matches[0]
    return namespace + ':' + ordinal.removeprefix('ItemListData'), name


def inventory_bit(inventory, purchases, faction, item, ref, name):
    row = inventory[item]
    bit = purchases['network_item_id_to_available_item_bit'][str(item)]
    require(row['name'] == name and row['authored_ref'] == ref
            and row['factions'] == [faction] and row['available_bit'] == bit
            and bit == (item & 1023) and bit < 768
            and purchases['equipment_token_item_ids'][name] == [bit],
            'original equipment unlock binding differs')
    return bit


def project(catalog_raw, bindings_raw, bootstrap_raw, purchases_raw, armory_raw, member, native,
            *, include_tier2_hosts=False):
    require(sha(bootstrap_raw) == SOURCE['COMBAT']['BOOTSTRAP_SHA256'], 'unreviewed retail bootstrap')
    catalog, ids, members = SOURCE['require_item_binding'](catalog_raw, bindings_raw, member)
    bootstrap, purchases, armory = map(parse, (bootstrap_raw, purchases_raw, armory_raw))
    BASE['validate_bootstrap'](bootstrap, sha(catalog_raw), parse(bindings_raw)['snapshot_sha256'])
    pins = {'items': sha(catalog_raw), 'item-bindings': sha(bindings_raw),
            'all-factions-bootstrap-loadouts': sha(bootstrap_raw), 'skill-purchases': sha(purchases_raw)}
    require(all(armory['source_sha256'][f'gameplay/data/retail-{name}.json'] == value
                for name, value in pins.items()), 'retail Armory source differs')
    stock = {row['faction']: row['loadouts'][0] for row in bootstrap['factions']}
    require(len(bootstrap['factions']) == 3 and set(stock) == {0, 1, 2}, 'expected three stock factions')
    source_lists = BASE['authored_table'](member, 'item_list.zpr', 'ItemList')['factionItemLists']
    skills_file, skill_key = 'character_skills.zpr', 'MagCharacterSkill42'
    skills = BASE['authored_table'](member, skills_file, 'MagCharacterSkillTable')
    skill = skills['Skills'][skill_key]['MagCharacterSkill']
    require(purchases['schema_version'] == 2 and purchases['application_id'] == 20231
            and skill == purchases['table']['Skills'][skill_key]['MagCharacterSkill'],
            'original launcher skill differs')
    host_skill_key = 'MagCharacterSkill1'
    if include_tier2_hosts:
        host_skill = skills['Skills'][host_skill_key]['MagCharacterSkill']
        require(host_skill == purchases['table']['Skills'][host_skill_key]['MagCharacterSkill'],
                'original tier-two rifle skill differs')
    members[BASE['SCRIPT_ROOT'] + skills_file] = sha(member(BASE['SCRIPT_ROOT'] + skills_file))
    tables = {
        group: BASE['indexed_definitions'](BASE['authored_table'](member, file, kind + 'Table'), kind)
        for file, (group, kind) in ITEMS['GROUPS'].items()
        if group in ('weapons', 'attachments', 'projectile_ammo', 'hitscan_ammo')
    }
    def definition(group, name):
        original = tables[group][name]
        require(original == catalog['definitions'][group][name]['parameters'],
                f'original definition differs for {group}/{name}')
        return original

    refs = {wire: ref for ref, wire in ids.items()}
    inventory = {row['wire_id']: row for row in armory['items']}
    variants, witnesses, ammo_names = [], [], set()
    roles = [(False, faction, color) for faction, color in enumerate(('Green', 'Black', 'Red'))]
    if include_tier2_hosts:
        roles += [(True, faction, color) for _, faction, color in roles]
    for upgraded, faction, color in roles:
        namespace = f'FactionItemList{faction + 1}'
        original = source_lists[namespace]['items']
        if upgraded:
            host_ref, host_name = upgraded_host(namespace, original, host_skill[color], tables['weapons'])
            host_id = ids[host_ref]
            require(host_id != variants[faction]['host_item_id'], 'tier-two rifle must differ from stock')
            host_bit = inventory_bit(inventory, purchases, faction, host_id, host_ref, host_name)
        else:
            host_id = stock[faction]['items'][0]
            host_ref = refs[host_id]
            require(host_ref.startswith(namespace + ':') and stock[faction]['items'][17] is None,
                    'unexpected stock host binding')
            host_row = original['ItemListData' + host_ref.split(':')[1]]
            require(host_row['item_type'] == 'kWeapon', 'wrong stock item role')
            host_name = host_row['id']
        host = definition('weapons', host_name)
        require(host['ammo_name'] in tables['hitscan_ammo'], 'rifle must retain hitscan')
        candidates = [entry for entry in host['attachment_list'].values()
                      if tables['attachments'][entry['WeaponAttachment']['name']]['attachment_type']
                      == 'kAttachTypeLauncher']
        require(len(candidates) == 1, 'expected one compatible launcher')
        name = candidates[0]['WeaponAttachment']['name']
        attachment = definition('attachments', name)
        weapon = definition('weapons', attachment['weapon_name'])
        matches = [ordinal for ordinal, row in original.items()
                   if row == {'id': name, 'item_type': 'kAttachmentItem'}]
        require(len(matches) == 1, 'missing or ambiguous original launcher inventory row')
        ref = namespace + ':' + matches[0].removeprefix('ItemListData')
        item = ids[ref]
        bit = inventory_bit(inventory, purchases, faction, item, ref, name)
        require(skill[color]['Available'] is True
                and name in {entry['element'] for entry in skill[color]['Equipment'].values()},
                'original launcher unlock binding differs')
        if upgraded:
            require(item != variants[faction]['attachment_item_id'], 'tier-two launcher must differ from stock')
        variants.append({'faction': faction, 'host_item_id': host_id, 'attachment_item_id': item,
                         **project_weapon(host, attachment, weapon)})
        witness = {'host_ref': host_ref, 'attachment_ref': ref, 'available_bit': bit,
                   'host_definition_sha256': sha(encode(host)),
                   'attachment_definition_sha256': sha(encode(attachment)),
                   'launcher_definition_sha256': sha(encode(weapon))}
        if upgraded:
            witness.update(host_skill=host_skill_key, host_available_bit=host_bit)
        witnesses.append(witness)
        ammo_names.add(weapon['ammo_name'])

    require(len(ammo_names) == 1, 'expected all launchers to share original ammo')
    name = next(iter(ammo_names))
    ammo = definition('projectile_ammo', name)
    tokens, aliases = ROCKETS['project_tokens'](member(ROCKETS['TOKEN_OVERRIDE']), [name])
    require(tokens[name] == catalog['definitions']['projectile_ammo'][name]['native_token_u32'],
            'original launcher ammo token differs')
    version = 2 if include_tier2_hosts else 1
    rules = {
        'schema': f'mag_retail_underbarrel_launchers_v{version}', 'application_id': 20231, 'attachment_slot': 17,
        'source': {'retail_elf_sha256': SOURCE['RETAIL_ELF_SHA256'],
                   'retail_archive_sha256': SOURCE['GAME_ARCHIVE_SHA256'],
                   'input_sha256': {**pins, 'armory': sha(armory_raw)},
                   'native_proof_sha256': sha(encode(native))},
        'variants': variants, 'ammo': project_ammo(ammo, tokens[name]),
    }
    proof = {'format': f'mag_retail_underbarrel_launcher_proof_v{version}', 'rules_sha256': sha(encode(rules)),
             'native_spans': native, 'source_members_sha256': members, 'token_override': aliases,
             'skill': skill_key, 'variants': witnesses, 'ammo_definition_sha256': sha(encode(ammo))}
    return rules, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    input_names = ('elf', 'game-archive', 'item-catalog', 'item-bindings',
                   'bootstrap-loadouts', 'skill-purchases', 'armory')
    for name in (*input_names, 'output', 'proof-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--include-tier2-hosts', action='store_true')
    args = parser.parse_args()
    inputs = [getattr(args, name.replace('-', '_')) for name in input_names]
    try:
        SOURCE['output_paths'](inputs, [args.output, args.proof_output])
        elf = SOURCE['NATIVE']['Elf'](args.elf, 'retail')
        raw = args.game_archive.read_bytes()
        require(sha(raw) == SOURCE['GAME_ARCHIVE_SHA256'], 'wrong original retail game archive')
        game = SOURCE['NATIVE']['Psarc'](raw)
        rules, proof = project(*(path.read_bytes() for path in inputs[2:]),
                               lambda name: SOURCE['archive_member'](game, name), native_witness(elf),
                               include_tier2_hosts=args.include_tier2_hosts)
        SOURCE['write_outputs'](inputs, [(args.output, encode(rules)), (args.proof_output, encode(proof))],
                                check=args.check)
    except (KeyError, ValueError, TypeError, OSError) as exc:
        parser.error(str(exc))
    if args.include_tier2_hosts:
        print('Verified six original stock and tier-two rifle underbarrel launchers')
    else:
        print('Verified three original stock rifle underbarrel launchers')


if __name__ == '__main__':
    main()
