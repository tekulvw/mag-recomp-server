#!/usr/bin/env python3
"""Project Recruit-granted flash grenade rules from owner-supplied retail files."""
import argparse
from pathlib import Path
import runpy
import struct


HERE = Path(__file__).resolve().parent
BASE = runpy.run_path(str(HERE / 'project-retail-infantry-rockets.py'))
SOURCE, ITEMS, ROCKETS = (BASE[name] for name in ('SOURCE', 'ITEMS', 'ROCKETS'))
require, sha, encode, parse = (SOURCE[name] for name in ('require', 'sha', 'encode', 'parse'))
APPLICATION_ID = 20231
COLORS = ('Green', 'Black', 'Red')
RECRUIT = 'MagCharacterSkill0'
SKILL_FILE = 'character_skills.zpr'
RECEIPT_PINS = {
    'gameplay': '4b38a12417f6a4226dc280e1d7d650b947c13eea39fb8719895613cc397f963e',
    'facing': 'e8e35ee56a627c225fce785e6df59150e035d92fcf6f2dbe7ebbf9a735192e22',
    'material': '16e1472a8657d0868f81569baf1622785cb6535eaf811adeea09906c0ac5866c',
}
DAMAGE_CHANNELS = {
    'character', 'characterAPierce', 'characterARedux', 'vehicle', 'vehicleAPierce',
    'vehicleARedux', 'objective', 'objectiveAPierce', 'objectiveARedux', 'mine',
}


def qualified_native(gameplay_raw, facing_raw, material_raw, elf):
    receipts = (gameplay_raw, facing_raw, material_raw)
    require(dict(zip(RECEIPT_PINS, map(sha, receipts), strict=True)) == RECEIPT_PINS,
            'unreviewed original flash qualification receipt')
    gameplay, facing, material = map(parse, receipts)
    elf_sha = sha(elf.raw)
    require(elf_sha == SOURCE['RETAIL_ELF_SHA256']
            == gameplay['retail_elf_sha256'] == facing['retail_elf_sha256'] == material['retail_elf_sha256']
            and facing['baseline_flash_sha256'] == RECEIPT_PINS['gameplay']
            and material['initial_flash_qualification_sha256'] == RECEIPT_PINS['gameplay'],
            'original flash receipts have different source bindings')

    def word(address, expected):
        result = elf.word(address)
        require(result == expected, f'original flash native scalar differs at {address:#x}')
        return result

    # Bind authored-scale defaults to their original constructor stores and
    # floating-point fallback. The zero values are omitted from the authored rows.
    zero = word(0x498FBC, 0x39000000) & 0xffff
    word(0x499014, 0x911F0070)
    word(0x49901C, 0x911F0078)
    word(0x108C090, 0xC0A27150)
    inner_scale, = struct.unpack('>f', elf.read(0x1B4CC54, 4)[0])
    require(inner_scale == .5, 'original flash radial scale differs')
    angle_word = word(0x1B4CFCC, int(facing['words']['0x1b4cfcc'], 16))
    return {
        'retail_elf_sha256': elf_sha, 'qualification_sha256': RECEIPT_PINS,
        'item_catalog_sha256': gameplay['item_catalog_sha256'],
        'qualified_items': gameplay['qualified_items'],
        'blind_values': {
            'facing_angle_rad': struct.unpack('>f', struct.pack('>I', angle_word))[0],
            'inner_scale': inner_scale, 'outer_scale': zero,
            'yaw_max': word(0x4373B8, int(facing['words']['0x4373b8'], 16)) & 0xffff,
            'ray_filter_mask': int(material['row_24_mask'], 16),
            'message_id': word(0x453D10, int(gameplay['words']['0x453d10'], 16)) & 0xffff,
            'duration_integer_bits': word(0x453D50, int(gameplay['words']['0x453d50'], 16)) & 0xffff,
            'duration_fraction_bits': word(0x453D40, int(gameplay['words']['0x453d40'], 16)) & 0xffff,
        },
    }


def select_recruit(items, ids, original, recruit, availability, armory, qualified):
    by_wire = {wire: ref for ref, wire in ids.items()}
    inventory = {row['wire_id']: row for row in armory['items']}
    defaults = {row['faction']: row['available_item_bits'] for row in armory['factions']}
    require(len(by_wire) == len(ids) and len(inventory) == len(armory['items'])
            and len(armory['factions']) == 3 and set(defaults) == {0, 1, 2},
            'ambiguous flash item binding or Armory')
    require(len(qualified) == 3 and recruit['Tier'] == 0 and recruit['Cost'] == 0,
            'expected three original flash rows and the no-cost Recruit grant')
    selected = []
    for faction, (color, qualified_row) in enumerate(zip(COLORS, qualified, strict=True)):
        name, wire = qualified_row['weapon'], qualified_row['item']
        require(type(wire) is int and 0 < wire < 4096 and wire >> 10 == faction + 1 and wire in by_wire,
                'flash item does not match its original faction binding')
        grant = recruit[color]
        require(grant['Available'] is True
                and sum(row['element'] == name for row in grant['Equipment'].values()) == 1,
                'flash must be granted by the available original Recruit row')
        ref = by_wire[wire]
        namespace, ordinal = ref.split(':', maxsplit=1)
        weapon = items[ref]
        require(namespace == f'FactionItemList{faction + 1}'
                and ordinal.isascii() and ordinal.isdecimal()
                and weapon['authored_ref'] == ref and weapon['name'] == name
                and weapon['item_type'] == 'kWeapon'
                and weapon['definition'] == {'group': 'weapons', 'name': name}
                and original[namespace]['items'][f'ItemListData{ordinal}']
                == {'id': name, 'item_type': 'kWeapon'},
                'flash weapon differs from the original faction ItemList')
        bit = availability['network_item_id_to_available_item_bit'][str(wire)]
        bound = inventory[wire]
        require(type(bit) is int and bit == (wire & 1023) and bit < 768
                and bit in defaults[faction]
                and availability['equipment_token_item_ids'][name] == [bit]
                and bound['available_bit'] == bit and bound['authored_ref'] == ref
                and bound['name'] == name and bound['factions'] == [faction],
                'flash item is not available by default in the matching faction Armory')
        selected.append({'faction': faction, 'item_id': wire, 'item_ref': ref,
                         'name': name, 'ammo_name': qualified_row['ammo'], 'available_bit': bit})
    return selected


def scalar(value, field, minimum, maximum):
    require(type(value) in (int, float) and minimum <= value <= maximum, f'invalid flash {field}')
    return value


def duration_ms(value, field):
    ms = scalar(value, field, .001, 300) * 1000
    require(ms % 1 == 0, f'flash {field} is not a whole millisecond')
    return int(ms)


def project_parameters(weapon, ammo, color):
    require(weapon['category'] == 'kWepCategoryGrenade'
            and weapon['faction'] == f'FactionType{color}'
            and weapon['menu_selection_slot'] == 'kSelectionMenuSlotEquipment'
            and weapon['character_attachment_type'] == 'kCharAttachTypeDefault'
            and weapon['fire_param_id'] == 'kFireParamSecondary'
            and weapon['ammo_name'] == ammo['id'],
            'unsupported original flash grenade gear or ammo binding')
    modes = weapon['fire_mode']['fire_modes']
    require(list(modes) == ['FireModeTypeClass0']
            and modes['FireModeTypeClass0']['FireModeTypeClass']['fire_mode_value'] == 'kGrenade',
            'unsupported flash grenade launch role')
    size, count = weapon['magazine_capacity'], weapon['initial_magazine_count']
    require(type(size) is int and size == 1 and type(count) is int and count == 3
            and weapon['infinite_ammo'] is False and weapon['bSupportLightAmmoPack'] is False,
            'unsupported flash inventory')
    for field, expected in {
        'propulsion_strategy': 'kWepPropulsionStrategyThrown',
        'detonation_strategy': 'kWepDetonationStrategyTimer',
        'explosion_strategy': 'kWepExplosionStrategyDebris',
    }.items():
        require(ammo[field] == expected, f'unsupported flash {field}')
    require(ammo['doHearingLoss'] is True and ammo['destroy_objective'] is False
            and not {'half_damage_radius_scale', 'full_damage_radius_scale'}.intersection(ammo),
            'unsupported flash hearing, objective or radial scale override')
    require(set(ammo['damage']) == DAMAGE_CHANNELS and all(
                type(value) in (int, float) and value == 0 for value in ammo['damage'].values()
            ), 'flash must remain non-damaging')
    fuse = duration_ms(ammo['detonation_timer'], 'fuse')
    lifetime = duration_ms(ammo['m_fLifetimeMax'], 'lifetime')
    inner = scalar(ammo['half_damage_radius'], 'inner radius', .001, 300)
    outer = scalar(ammo['full_damage_radius'], 'outer radius', .001, 300)
    penetration = scalar(ammo['MaterialPenetration'], 'penetration', .001, 100)
    strong = scalar(ammo['m_fDuration'], 'strong duration', 1 / 1024, 31 + 1023 / 1024)
    weak = scalar(ammo['m_fInitialDelay'], 'weak duration', 1 / 1024, 31 + 1023 / 1024)
    require(fuse < lifetime and inner < outer, 'invalid flash fuse/lifetime or radii ordering')
    require(weak < strong, 'invalid flash durations ordering')
    return {
        'initial_count': count, 'fuse_ms': fuse, 'projectile_lifetime_ms': lifetime,
        'inner_radius_m': inner, 'outer_radius_m': outer, 'material_penetration': penetration,
    }, {'strong_duration_s': strong, 'weak_duration_s': weak}


def project(catalog_raw, binding_raw, bootstrap_raw, skills_raw, armory_raw, read_game, native):
    require(sha(bootstrap_raw) == SOURCE['COMBAT']['BOOTSTRAP_SHA256'],
            'unreviewed original retail bootstrap')
    catalog, ids, members = SOURCE['require_item_binding'](catalog_raw, binding_raw, read_game)
    bootstrap, skills, armory = map(parse, (bootstrap_raw, skills_raw, armory_raw))
    BASE['validate_bootstrap'](bootstrap, sha(catalog_raw), parse(binding_raw)['snapshot_sha256'])
    pins = {
        'items': sha(catalog_raw), 'item-bindings': sha(binding_raw),
        'all-factions-bootstrap-loadouts': sha(bootstrap_raw), 'skill-purchases': sha(skills_raw),
    }
    require(native['item_catalog_sha256'] == pins['items'], 'flash item catalog differs from native proof')
    require(armory['schema_version'] == 1 and armory['application_id'] == APPLICATION_ID
            and all(armory['source_sha256'][f'gameplay/data/retail-{name}.json'] == digest
                    for name, digest in pins.items()), 'retail flash Armory source differs')

    skill_member = BASE['SCRIPT_ROOT'] + SKILL_FILE
    members[skill_member] = sha(read_game(skill_member))
    recruit = BASE['authored_table'](
        read_game, SKILL_FILE, 'MagCharacterSkillTable'
    )['Skills'][RECRUIT]['MagCharacterSkill']
    require(skills['schema_version'] == 2 and skills['application_id'] == APPLICATION_ID
            and skills['source_sha256']['game.psarc'] == SOURCE['GAME_ARCHIVE_SHA256']
            and skills['source_sha256'][SKILL_FILE] == members[skill_member]
            and all(skills['source_sha256'][f'gameplay/data/retail-{name}.json'] == pins[name]
                    for name in ('items', 'item-bindings'))
            and skills['table']['Skills'][RECRUIT]['MagCharacterSkill'] == recruit,
            'original flash Recruit source differs')
    original = BASE['authored_table'](read_game, 'item_list.zpr', 'ItemList')['factionItemLists']
    selected = select_recruit(catalog['items'], ids, original, recruit, skills, armory,
                              native['qualified_items'])
    tables = {
        group: BASE['indexed_definitions'](
            BASE['authored_table'](read_game, file, kind + 'Table'), kind
        )
        for file, (group, kind) in ITEMS['GROUPS'].items()
        if group in ('weapons', 'projectile_ammo')
    }

    def definition(group, name):
        original_row = tables[group][name]
        require(original_row == catalog['definitions'][group][name]['parameters'],
                f'original flash definition differs for {group}/{name}')
        return original_row

    ammo_names = [row['ammo_name'] for row in selected]
    token_member = ROCKETS['TOKEN_OVERRIDE']
    token_data = read_game(token_member)
    tokens, aliases = ROCKETS['project_tokens'](token_data, ammo_names)
    members[token_member] = sha(token_data)
    require(len(set(tokens.values())) == 3, 'flash variants require three distinct original ammo tokens')

    variants, witnesses = [], []
    common_durations = None
    for row in selected:
        ammo_name = row['ammo_name']
        weapon = definition('weapons', row['name'])
        ammo = definition('projectile_ammo', ammo_name)
        token = tokens[ammo_name]
        require(token != 0 and token == catalog['definitions']['projectile_ammo'][ammo_name]['native_token_u32'],
                'flash native ammo token differs')
        variant, durations = project_parameters(weapon, ammo, COLORS[row['faction']])
        require(common_durations is None or common_durations == durations,
                'stock flash variants disagree on shared blind duration')
        common_durations = durations
        variants.append({'faction': row['faction'], 'item_id': row['item_id'], 'ammo_token': token, **variant})
        witnesses.append({**row, 'weapon_definition_sha256': sha(encode(weapon)),
                          'ammo_definition_sha256': sha(encode(ammo))})
    rules = {
        'schema': 'mag_retail_flash_grenades_v1', 'application_id': APPLICATION_ID,
        'source': {
            'retail_elf_sha256': native['retail_elf_sha256'],
            'retail_archive_sha256': SOURCE['GAME_ARCHIVE_SHA256'],
            'item_catalog_sha256': pins['items'], 'item_bindings_sha256': pins['item-bindings'],
            'bootstrap_sha256': pins['all-factions-bootstrap-loadouts'],
            'skill_catalog_sha256': pins['skill-purchases'], 'native_proof_sha256': sha(encode(native)),
        },
        'blind': {**native['blind_values'], **common_durations},
        'variants': variants,
    }
    proof = {
        'format': 'mag_retail_flash_grenade_proof_v1', 'rules_sha256': sha(encode(rules)),
        'native': native, 'source_members_sha256': members, 'token_override': aliases,
        'armory_sha256': sha(armory_raw), 'recruit_grant': RECRUIT, 'variants': witnesses,
    }
    return rules, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    names = ('elf', 'game-archive', 'item-catalog', 'item-bindings', 'bootstrap-loadouts',
             'skill-purchases', 'armory', 'gameplay-qualification', 'facing-qualification',
             'material-qualification')
    for name in (*names, 'output', 'proof-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [getattr(args, name.replace('-', '_')) for name in names]
    try:
        SOURCE['output_paths'](inputs, [args.output, args.proof_output])
        elf = SOURCE['NATIVE']['Elf'](args.elf, 'retail')
        archive = args.game_archive.read_bytes()
        require(sha(archive) == SOURCE['GAME_ARCHIVE_SHA256'], 'wrong original retail game archive')
        game = SOURCE['NATIVE']['Psarc'](archive)
        native = qualified_native(*(path.read_bytes() for path in inputs[7:]), elf)
        rules, proof = project(*(path.read_bytes() for path in inputs[2:7]),
                               lambda name: SOURCE['archive_member'](game, name), native)
        SOURCE['write_outputs'](inputs, [(args.output, encode(rules)), (args.proof_output, encode(proof))],
                                check=args.check)
    except (KeyError, ValueError, TypeError, OSError) as exc:
        parser.error(str(exc))
    print('Verified three original retail Recruit-granted flash grenade variants')


if __name__ == '__main__':
    main()
