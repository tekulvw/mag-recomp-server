#!/usr/bin/env python3
"""Project purchased gas grenade rules from owner-supplied retail files."""
import argparse
from pathlib import Path
import runpy


HERE = Path(__file__).resolve().parent
BASE = runpy.run_path(str(HERE / 'project-retail-infantry-rockets.py'))
SOURCE, ITEMS, ROCKETS = (BASE[name] for name in ('SOURCE', 'ITEMS', 'ROCKETS'))
require, sha, encode, parse = (SOURCE[name] for name in ('require', 'sha', 'encode', 'parse'))
APPLICATION_ID = 20231
COLORS = ('Green', 'Black', 'Red')
SKILL = 'MagCharacterSkill50'
SKILL_FILE = 'character_skills.zpr'
NETWORK_SHA = '467a788982b95fd6b1ce38a4308e6db60064e53efbb2ca475485a7d7043c4b40'
VOLUME_SHA = '7c61caca65fba47e73f9e05aab149c6b77c30e56f9e34904f6400677f13871ff'
ZERO_DAMAGE_CHANNELS = {
    'characterAPierce', 'characterARedux', 'vehicle', 'vehicleAPierce', 'vehicleARedux',
    'objective', 'objectiveAPierce', 'objectiveARedux', 'mine',
}


def qualified_native(network_raw, volume_raw, elf_sha):
    require(sha(network_raw) == NETWORK_SHA and sha(volume_raw) == VOLUME_SHA,
            'unreviewed original gas qualification receipt')
    network, volume = map(parse, (network_raw, volume_raw))
    require(elf_sha == SOURCE['RETAIL_ELF_SHA256']
            == network['original_retail_elf_sha256'] == volume['retail_elf_sha256'],
            'wrong original gas ELF')
    return {
        'retail_elf_sha256': elf_sha,
        'network_qualification_sha256': NETWORK_SHA,
        'volume_qualification_sha256': VOLUME_SHA,
        'volume_values': volume['source_values'],
        'visual_sources': network['gas_visual_sources'],
        'visual_members_sha256': network['source_members_sha256'],
        'visual_item_catalog_sha256': network['item_catalog_sha256'],
        'visual_item_bindings_sha256': network['bindings_sha256'],
        'visual_archive_sha256': network['retail_archive_sha256'],
    }


def select_purchased(items, ids, original, skill, purchases, armory):
    inventory = {row['wire_id']: row for row in armory['items']}
    ownership = {row['faction']: row['available_item_bits'] for row in armory['factions']}
    require(len(inventory) == len(armory['items'])
            and len(armory['factions']) == 3 and set(ownership) == {0, 1, 2},
            'invalid gas Armory inventory')
    selected = []
    for faction, color in enumerate(COLORS):
        grant = skill[color]
        require(grant['Available'] is True and len(grant['Equipment']) == 1,
                'original gas skill must grant one available faction item')
        name = next(iter(grant['Equipment'].values()))['element']
        namespace = f'FactionItemList{faction + 1}'
        rows = original[namespace]['items']
        matches = [key for key, row in rows.items() if row.get('id') == name]
        require(len(matches) == 1 and rows[matches[0]] == {'id': name, 'item_type': 'kWeapon'},
                'expected one original gas weapon in the faction ItemList')
        ordinal = matches[0].removeprefix('ItemListData')
        require(matches[0] != ordinal and ordinal.isascii() and ordinal.isdecimal(),
                'invalid original gas ItemList row key')
        ref = f'{namespace}:{ordinal}'
        weapon = items[ref]
        item = ids[ref]
        require(weapon['authored_ref'] == ref and weapon['name'] == name
                and weapon['item_type'] == 'kWeapon'
                and weapon['definition'] == {'group': 'weapons', 'name': name}
                and type(item) is int and 0 < item < 4096 and item >> 10 == faction + 1,
                'gas item does not match the original faction binding')
        bound = inventory[item]
        bit = purchases['network_item_id_to_available_item_bit'][str(item)]
        require(type(bit) is int and bit == (item & 1023) and bit < 768
                and bit not in ownership[faction]
                and purchases['equipment_token_item_ids'][name] == [bit]
                and bound['available_bit'] == bit and bound['authored_ref'] == ref
                and bound['name'] == name and bound['factions'] == [faction],
                'gas item does not match the original purchased availability bit')
        selected.append({'faction': faction, 'item_id': item, 'item_ref': ref, 'name': name,
                         'available_bit': bit})
    return selected


def scalar(value, field, minimum, maximum):
    require(type(value) in (int, float) and minimum <= value <= maximum,
            f'invalid gas {field}')
    return value


def duration_ms(value, field):
    milliseconds = scalar(value, field, .001, 300) * 1000
    require(milliseconds % 1 == 0, f'gas {field} is not a whole millisecond')
    return int(milliseconds)


def project_parameters(weapon, ammo, faction_color, native_values):
    require(weapon['category'] == 'kWepCategoryGrenade'
            and weapon['faction'] == f'FactionType{faction_color}'
            and weapon['menu_selection_slot'] == 'kSelectionMenuSlotEquipment'
            and weapon['character_attachment_type'] == 'kCharAttachTypeDefault'
            and weapon['fire_param_id'] == 'kFireParamSecondary'
            and weapon['ammo_name'] == ammo['id'],
            'unsupported original gas grenade gear or ammo binding')
    modes = weapon['fire_mode']['fire_modes']
    require(list(modes) == ['FireModeTypeClass0']
            and modes['FireModeTypeClass0']['FireModeTypeClass']['fire_mode_value'] == 'kGrenade',
            'unsupported gas grenade launch role')
    size, count = weapon['magazine_capacity'], weapon['initial_magazine_count']
    require(weapon['infinite_ammo'] is False and weapon['bSupportLightAmmoPack'] is False
            and type(size) is int and size == 1 and type(count) is int and 0 < count <= 255
            and count == native_values['capacity'], 'unsupported gas inventory')
    for field, expected in {
        'propulsion_strategy': 'kWepPropulsionStrategyThrown',
        'detonation_strategy': 'kWepDetonationStrategyTimer',
        'explosion_strategy': 'kWepExplosionStrategyVolume',
        'm_eAmmoPersistance': 'kWepPersistanceGrenadeGasPoison',
    }.items():
        require(ammo[field] == expected, f'unsupported gas {field}')
    require(ammo['destroy_objective'] is False and ammo['doHearingLoss'] is False
            and not {'half_damage_radius_scale', 'full_damage_radius_scale'}.intersection(ammo),
            'unsupported gas objective, hearing-loss or scale override')
    damage = ammo['damage']
    require(set(damage) == ZERO_DAMAGE_CHANNELS | {'character'}
            and all(type(damage[field]) in (int, float) and damage[field] == 0
                    for field in ZERO_DAMAGE_CHANNELS), 'unsupported gas damage channel')
    fuse = duration_ms(ammo['detonation_timer'], 'detonation_timer')
    lifetime = duration_ms(ammo['m_fLifetimeMax'], 'm_fLifetimeMax')
    limit = duration_ms(ammo['m_fDuration'], 'm_fDuration')
    warmup = duration_ms(ammo['m_fInitialDelay'], 'm_fInitialDelay')
    interval = duration_ms(native_values['interval_s'], 'native interval')
    inner = scalar(ammo['half_damage_radius'], 'half_damage_radius', .001, 300)
    outer = scalar(ammo['full_damage_radius'], 'full_damage_radius', .001, 300)
    amount = scalar(damage['character'], 'Character pulse', .001, 100_000)
    require(fuse < lifetime and warmup < limit and interval <= limit and inner < outer,
            'invalid gas timing or radius ordering')
    require(warmup == duration_ms(native_values['delay_s'], 'native warmup')
            and limit == duration_ms(native_values['limit_s'], 'native duration')
            and inner == native_values['inner_radius_m'] and outer == native_values['outer_radius_m']
            and amount == native_values['character_amount_per_pulse'],
            'authored gas parameters disagree with the native qualification')
    return {
        'initial_count': count, 'fuse_ms': fuse, 'projectile_lifetime_ms': lifetime,
        'volume_duration_ms': limit, 'warmup_ms': warmup, 'interval_ms': interval,
        'inner_radius_m': inner, 'inner_scale': scalar(native_values['inner_scale'], 'inner scale', 0, 1),
        'outer_radius_m': outer, 'outer_scale': scalar(native_values['outer_scale'], 'outer scale', 0, 1),
        'character_hp_per_pulse': amount,
    }


def project(catalog_raw, binding_raw, bootstrap_raw, skill_raw, armory_raw, read_game, native):
    require(sha(bootstrap_raw) == SOURCE['COMBAT']['BOOTSTRAP_SHA256'],
            'unreviewed original retail bootstrap')
    catalog, ids, members = SOURCE['require_item_binding'](catalog_raw, binding_raw, read_game)
    bootstrap, purchases, armory = map(parse, (bootstrap_raw, skill_raw, armory_raw))
    BASE['validate_bootstrap'](bootstrap, sha(catalog_raw), parse(binding_raw)['snapshot_sha256'])
    pins = {
        'items': sha(catalog_raw), 'item-bindings': sha(binding_raw),
        'all-factions-bootstrap-loadouts': sha(bootstrap_raw), 'skill-purchases': sha(skill_raw),
    }
    require(armory['schema_version'] == 1 and armory['application_id'] == APPLICATION_ID
            and all(armory['source_sha256'][f'gameplay/data/retail-{name}.json'] == digest
                    for name, digest in pins.items()), 'retail gas Armory source differs')
    skill_member = BASE['SCRIPT_ROOT'] + SKILL_FILE
    members[skill_member] = sha(read_game(skill_member))
    original_skills = BASE['authored_table'](read_game, SKILL_FILE, 'MagCharacterSkillTable')['Skills']
    original_skill = original_skills[SKILL]['MagCharacterSkill']
    require(purchases['schema_version'] == 2 and purchases['application_id'] == APPLICATION_ID
            and purchases['source_sha256']['game.psarc'] == SOURCE['GAME_ARCHIVE_SHA256']
            and purchases['source_sha256'][SKILL_FILE] == members[skill_member]
            and all(purchases['source_sha256'][f'gameplay/data/retail-{name}.json'] == pins[name]
                    for name in ('items', 'item-bindings'))
            and purchases['table']['Skills'][SKILL]['MagCharacterSkill'] == original_skill,
            'original gas skill differs')
    source_lists = BASE['authored_table'](read_game, 'item_list.zpr', 'ItemList')['factionItemLists']
    selected = select_purchased(catalog['items'], ids, source_lists, original_skill, purchases, armory)
    starter = {item for row in bootstrap['factions'] for kit in row['loadouts'] for item in kit['items']}
    require(all(row['item_id'] not in starter for row in selected),
            'purchased gas appears in a stock kit')
    require(native['visual_archive_sha256'] == SOURCE['GAME_ARCHIVE_SHA256']
            and native['visual_item_catalog_sha256'] == pins['items']
            and native['visual_item_bindings_sha256'] == pins['item-bindings']
            and all(sha(read_game(member)) == digest
                    for member, digest in native['visual_members_sha256'].items()),
            'original gas visual sources differ')
    tables = {
        group: BASE['indexed_definitions'](
            BASE['authored_table'](read_game, file, kind + 'Table'), kind
        )
        for file, (group, kind) in ITEMS['GROUPS'].items()
        if group in ('weapons', 'projectile_ammo')
    }

    def definition(group, name):
        original = tables[group][name]
        require(original == catalog['definitions'][group][name]['parameters'],
                f'original gas definition differs for {group}/{name}')
        return original

    ammo_names = [definition('weapons', row['name'])['ammo_name'] for row in selected]
    tokens, aliases = ROCKETS['project_tokens'](read_game(ROCKETS['TOKEN_OVERRIDE']), ammo_names)
    require(len(set(tokens.values())) == len(selected) == len(native['visual_sources']),
            'original gas variants require three distinct ammo tokens')
    variants, witnesses = [], []
    for row, ammo_name, visual in zip(selected, ammo_names, native['visual_sources'], strict=True):
        weapon = definition('weapons', row['name'])
        ammo = definition('projectile_ammo', ammo_name)
        token = tokens[ammo_name]
        weapon_sha, ammo_sha = sha(encode(weapon)), sha(encode(ammo))
        require(token != 0 and token == catalog['definitions']['projectile_ammo'][ammo_name]['native_token_u32']
                and visual == {
                    'faction': row['faction'], 'item_id': row['item_id'], 'item_ref': row['item_ref'],
                    'item': row['name'], 'ammo': ammo_name, 'ammo_token': hex(token),
                    'dry_effect': ammo['debris_effect'], 'weapon_sha256': weapon_sha, 'ammo_sha256': ammo_sha,
                },
                'original gas presentation or ammo token differs')
        variants.append({
            'faction': row['faction'], 'item_id': row['item_id'], 'ammo_token': token,
            **project_parameters(weapon, ammo, COLORS[row['faction']], native['volume_values']),
        })
        witnesses.append({**row, 'ammo_name': ammo_name,
                          'weapon_definition_sha256': weapon_sha, 'ammo_definition_sha256': ammo_sha})
    rules = {
        'schema': 'mag_retail_gas_grenades_v1', 'application_id': APPLICATION_ID,
        'source': {
            'retail_elf_sha256': native['retail_elf_sha256'],
            'retail_archive_sha256': SOURCE['GAME_ARCHIVE_SHA256'],
            'item_catalog_sha256': pins['items'], 'item_bindings_sha256': pins['item-bindings'],
            'bootstrap_sha256': pins['all-factions-bootstrap-loadouts'],
            'skill_catalog_sha256': pins['skill-purchases'], 'native_proof_sha256': sha(encode(native)),
        },
        'variants': variants,
    }
    proof = {
        'format': 'mag_retail_gas_grenade_proof_v1', 'rules_sha256': sha(encode(rules)),
        'native': native, 'source_members_sha256': members, 'token_override': aliases,
        'armory_sha256': sha(armory_raw), 'skill': SKILL, 'variants': witnesses,
    }
    return rules, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    names = ('elf', 'game-archive', 'item-catalog', 'item-bindings', 'bootstrap-loadouts',
             'skill-purchases', 'armory', 'network-qualification', 'volume-qualification')
    for name in (*names, 'output', 'proof-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [getattr(args, name.replace('-', '_')) for name in names]
    try:
        SOURCE['output_paths'](inputs, [args.output, args.proof_output])
        elf = SOURCE['NATIVE']['Elf'](args.elf, 'retail')
        raw = args.game_archive.read_bytes()
        require(sha(raw) == SOURCE['GAME_ARCHIVE_SHA256'], 'wrong original retail game archive')
        game = SOURCE['NATIVE']['Psarc'](raw)
        native = qualified_native(args.network_qualification.read_bytes(),
                                  args.volume_qualification.read_bytes(), sha(elf.raw))
        rules, proof = project(*(path.read_bytes() for path in inputs[2:7]),
                               lambda name: SOURCE['archive_member'](game, name), native)
        SOURCE['write_outputs'](inputs, [(args.output, encode(rules)), (args.proof_output, encode(proof))],
                                check=args.check)
    except (KeyError, ValueError, TypeError, OSError) as exc:
        parser.error(str(exc))
    print('Verified three original retail purchased gas grenade variants')


if __name__ == '__main__':
    main()
