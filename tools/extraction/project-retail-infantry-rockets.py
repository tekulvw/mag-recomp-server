#!/usr/bin/env python3
"""Project the three original retail infantry launchers from stock Heavy kits."""
import argparse
import math
from pathlib import Path
import runpy
import struct


HERE = Path(__file__).resolve().parent
SOURCE = runpy.run_path(str(HERE / 'project-retail-hospital-turrets.py'))
ROCKETS = runpy.run_path(str(HERE / 'project-retail-hospital-rockets.py'))
ITEMS = runpy.run_path(str(HERE / 'extract-item-catalog.py'))
require, sha, encode, parse = (SOURCE[name] for name in ('require', 'sha', 'encode', 'parse'))
SCRIPT_ROOT = SOURCE['SCRIPT_ROOT']
APPLICATION_ID = 20231
STOCK_KIT = 2
LAUNCHER_GEAR = 3
SIGHT_SLOT = 21


def validate_bootstrap(bootstrap, catalog_sha, snapshot_sha):
    provenance = bootstrap['provenance']
    require(bootstrap['schema_version'] == 1 and bootstrap['application_id'] == APPLICATION_ID
            and provenance['original_retail_elf_sha256'] == SOURCE['RETAIL_ELF_SHA256']
            and provenance['catalog_sha256'] == catalog_sha
            and provenance['snapshot_sha256'] == snapshot_sha,
            'different original retail bootstrap binding')


def select_stock(bootstrap, items, ids, original):
    by_faction = {row['faction']: row for row in bootstrap['factions']}
    require(len(bootstrap['factions']) == 3 and set(by_faction) == {0, 1, 2},
            'expected one stock launcher loadout per faction')
    by_wire = {wire: ref for ref, wire in ids.items()}
    require(len(by_wire) == len(ids), 'ambiguous stock item binding')
    result = []
    for faction, faction_row in sorted(by_faction.items()):
        require(type(faction) is int and len(faction_row['loadouts']) >= STOCK_KIT + 1,
                'invalid stock faction loadouts')
        kit = faction_row['loadouts'][STOCK_KIT]['items']
        require(isinstance(kit, list) and len(kit) == 22, 'invalid stock Heavy kit')
        group = f'FactionItemList{faction + 1}'
        row = {'faction': faction}
        for slot, kind, definition_group, id_key, name_key, ref_key in (
            (LAUNCHER_GEAR, 'kWeapon', 'weapons', 'item_id', 'weapon_name', 'weapon_ref'),
            (SIGHT_SLOT, 'kAttachmentItem', 'attachments', 'stock_sight_item_id', 'sight_name',
             'sight_ref'),
        ):
            wire = kit[slot]
            require(type(wire) is int and 0 < wire < 4096 and wire in by_wire,
                    f'unbound original stock Heavy slot {slot}')
            ref = by_wire[wire]
            item = items[ref]
            namespace, ordinal = ref.split(':', maxsplit=1)
            require(namespace == group and ordinal.isascii() and ordinal.isdecimal()
                    and item['authored_ref'] == ref and item['item_type'] == kind,
                    f'wrong original faction or item type for Heavy slot {slot}')
            name = item['name']
            authored = original[group]['items'].get(f'ItemListData{ordinal}')
            require(item['definition'] == {'group': definition_group, 'name': name}
                    and authored == {'id': name, 'item_type': kind},
                    f'original ItemList differs for Heavy slot {slot}')
            row.update({id_key: wire, name_key: name, ref_key: ref})
        result.append(row)
    return result


def number(value, field, minimum, maximum):
    require(type(value) in (int, float) and minimum <= value <= maximum,
            f'invalid infantry launcher {field}')
    return float(value)


def project_ballistics(weapon, ammo, age_grace):
    modes = weapon['fire_mode']['fire_modes']
    require(weapon['category'] == 'kWepCategoryRocketLauncher'
            and weapon['fire_param_id'] == 'kFireParamSecondary'
            and weapon['initial_fire_mode'] == 'kSemiAuto'
            and list(modes) == ['FireModeTypeClass0']
            and modes['FireModeTypeClass0']['FireModeTypeClass']['fire_mode_value'] == 'kSemiAuto'
            and weapon['use_turret_mesh'] is False and weapon['alternate_firepoint'] == '',
            'unsupported stock infantry launcher')
    size, initial_magazines = weapon['magazine_capacity'], weapon['initial_magazine_count']
    require(weapon['infinite_ammo'] is False and weapon['bSupportLightAmmoPack'] is False
            and type(size) is int and size == 1
            and type(initial_magazines) is int and 1 <= initial_magazines <= 255,
            'unsupported finite infantry launcher inventory')
    weapon_fields = {
        'magazine_size': size,
        'initial_magazine_count': initial_magazines,
        'refire_seconds': number(weapon['fire_mode']['refire_time'], 'refire time', .001, 60.),
        'reload_seconds': number(weapon['fire_mode']['reload_time'], 'reload time', .001, 120.),
    }

    for field, expected in {
        'propulsion_strategy': 'kWepPropulsionStrategyRocketDumb',
        'collisionType': 'kWepProjCollisionShapeCapsule',
        'detonation_strategy': 'kWepDetonationStrategyImpact',
        'explosion_strategy': 'kWepExplosionStrategyDebris',
    }.items():
        require(ammo[field] == expected, f'unsupported infantry launcher {field}')
    require(weapon['ammo_name'] == ammo['id'], 'infantry launcher ammo binding differs')
    number(ammo['detonation_timer'], 'impact timer', 0., 0.)
    turn_range = ammo['rocketDumbDirectionChangeRange']
    require(isinstance(turn_range, list) and len(turn_range) == 2,
            'invalid infantry launcher turn range')
    for value in turn_range:
        number(value, 'turn range', 0., 0.)

    spine = ammo['capsuleHeight']
    require(isinstance(spine, list) and len(spine) == 3, 'invalid infantry launcher capsule spine')
    spine = [number(value, 'capsule spine component', -2., 2.) for value in spine]
    require(0. < math.hypot(*spine) <= 2., 'invalid infantry launcher capsule length')
    lifetime = number(ammo['m_fLifetimeMax'], 'lifetime', .01, 60.)
    armed = number(ammo['m_fArmedTime'], 'arming time', .001, lifetime)
    require(armed < lifetime, 'infantry launcher must arm before its lifetime')
    cutoff = number(ammo['rocketAutoExplodeDistance'], 'auto-explode distance', .01, 1000.)
    cruise = number(ammo['rocketDumbCruiseStraightRange'], 'straight cruise', 0., cutoff)
    require(cruise < cutoff, 'infantry launcher must leave straight cruise before its range bound')
    flight = {
        'radius_m': number(ammo['radius'], 'collision radius', .001, 1.),
        'capsule_spine_local_xyz': spine,
        'speed_mps': number(ammo['launch_velocity'], 'launch speed', 1., 500.),
        'armed_time_seconds': armed,
        'auto_explode_distance_m': cutoff,
        'lifetime_seconds': lifetime,
        'age_grace_seconds': number(age_grace, 'generic age grace', 0., 10.),
    }

    damage = ammo['damage']
    for field in ('vehicleAPierce', 'vehicleARedux'):
        number(damage[field], field, 0., 0.)
    blast = {
        'half_radius_m': number(ammo['half_damage_radius'], 'half-damage radius', .001, 100.),
        'outer_radius_m': number(ammo['full_damage_radius'], 'outer radius', .001, 100.),
        'material_penetration': number(ammo['MaterialPenetration'], 'material penetration', 0., 20.),
        'character_hp': number(damage['character'], 'Character damage', .001, 10000.),
        'vehicle_hp': number(damage['vehicle'], 'vehicle damage', .001, 10000.),
    }
    require(blast['half_radius_m'] < blast['outer_radius_m'],
            'infantry launcher blast radius ordering differs')
    return weapon_fields, flight, blast


def native_witness(elf):
    words = {
        0x767330: 0x780a6ee2,  # Kit index.
        0x7676f4: 0x9ac10160,  # Request gear byte.
        0x767748: 0xb1610172,  # Independent projectile slot.
        0x7dc7a4: 0x889c0040, 0x7dcb0c: 0x812b0020,  # Equipped availability and use.
        0x7b343c: 0x13ffe00a,  # Extra velocity.
        0x7b2a18: 0x38000017,  # Contact layer.
        0x7ace18: 0x419e08a4,  # Capsule selection.
        0x4812dc: 0xc02b0158, 0x483f54: 0xc3ea015c,  # Range and cruise.
        0x7a2b7c: 0xc0090018, 0x7a2b8c: 0xc009001c,  # Impact timers.
        0x1084a08: 0x3863ffbc, 0x1084a14: 0x7c4300ce,  # Debris origin is projectile root.
    }
    for address, value in words.items():
        require(elf.word(address) == value, f'original launcher instruction differs at {address:#x}')
    for slot, target in {
        0x1a6aee4: 0x5df7fc,
        0x1a745b4: 0x883d5c, 0x1a745c4: 0x8872a4, 0x1a745c8: 0x8863b0,
        0x1a5efe0: 0x483ee0, 0x1a5f000: 0x4812d4,
        0x1a73c44: 0x108cb50, 0x1a73c78: 0x1084a08,
    }.items():
        require(elf.word(elf.word(slot)) == target,
                f'original launcher native dispatch differs at {slot:#x}')
    families = list(struct.unpack('>4I', elf.read(0x15af0b8, 16)[0]))
    require(families == [16, 17, 18, 19], 'original projectile family slots differ')
    toc = elf.word(0x1b008ac)
    half = struct.unpack('>f', elf.read(elf.word(toc + 0x74d4), 4)[0])[0]
    age_grace = struct.unpack('>f', elf.read(toc + 0x6ff8, 4)[0])[0]
    require(half == .5 and age_grace == 2. and elf.word(toc + 0x6ffc) == 0,
            'original launcher shape or timer constants differ')
    spans = {
        'client_launch_gear': (0x7672f8, 0x7673c4),
        'client_launch_packet': (0x7676b0, 0x7677bc),
        'personal_equipped_ammo': (0x7dc784, 0x7dc814),
        'server_launch_and_deduct': (0x7dca80, 0x7dcb28),
        'finite_magazine_initialization': (0x7fa63c, 0x7fa68c),
        'projectile_creation_velocity_and_family': (0x7b331c, 0x7b34ac),
        'projectile_contact_group': (0x7b29dc, 0x7b2a30),
        'capsule_spine_constructor': (0x7ad6bc, 0x7ad79c),
        'impact_arming_and_callback': (0x7a2b64, 0x7a2be4),
        'impact_strategy': (0x480f48, 0x480fe0),
        'rocket_dumb_update': (0x483ee0, 0x483fc4),
        'rocket_dumb_get_auto_explosion_distance': (0x4812d4, 0x4812e4),
        'rocket_base_range_explosion': (0x483c90, 0x483e18),
        'projectile_debris_root_position': (0x1084a08, 0x1084a1c),
        'projectile_debris_sphere_origin': (0x7a4d54, 0x7a4e94),
        'age_explosion': (0x7a7d34, 0x7a7d80),
    }
    return {
        'retail_elf_sha256': sha(elf.raw),
        'gear_slot': LAUNCHER_GEAR,
        'projectile_slot_families': families,
        'raw_contact_layer': 23,
        'age_grace_seconds': age_grace,
        'capsule_spine_endpoint_factor': half,
        'blast_origin': 'projectile_root_position',
        'spans': {name: {'start': hex(start), 'end_exclusive': hex(end),
                         'sha256': sha(elf.read(start, end - start)[0])}
                  for name, (start, end) in spans.items()},
    }


def authored_table(read_game, file, table):
    raw = read_game(SCRIPT_ROOT + file)
    parsed = ITEMS['Parser'](raw.decode('utf-8-sig'), file).parse()
    return ITEMS['normalize'](parsed)[table]


def indexed_definitions(source, kind):
    rows = source[kind + 'List']
    values = rows.values() if isinstance(rows, dict) else rows
    result = {}
    for wrapped in values:
        row = wrapped[kind]
        name = row['id']
        require(name not in result, f'ambiguous original launcher {kind} name')
        result[name] = row
    return result


def project(catalog_raw, binding_raw, bootstrap_raw, read_game, native):
    require(sha(bootstrap_raw) == SOURCE['COMBAT']['BOOTSTRAP_SHA256'],
            'unreviewed original retail bootstrap')
    catalog, ids, members = SOURCE['require_item_binding'](catalog_raw, binding_raw, read_game)
    bootstrap = parse(bootstrap_raw)
    validate_bootstrap(bootstrap, sha(catalog_raw), parse(binding_raw)['snapshot_sha256'])
    original_items = authored_table(read_game, 'item_list.zpr', 'ItemList')['factionItemLists']
    stock = select_stock(bootstrap, catalog['items'], ids, original_items)
    definitions = {
        group: indexed_definitions(authored_table(read_game, file, kind + 'Table'), kind)
        for file, (group, kind) in ITEMS['GROUPS'].items()
        if group in ('weapons', 'projectile_ammo', 'attachments')
    }
    attachment_member = SCRIPT_ROOT + 'attachments.lua'
    members[attachment_member] = sha(read_game(attachment_member))
    require(members[attachment_member] == catalog['sources']['attachments.lua']['sha256'],
            'original attachment catalog source differs')

    def qualified_definition(group, name):
        original = definitions[group][name]
        require(original == catalog['definitions'][group][name]['parameters'],
                f'original launcher definition differs for {group}/{name}')
        return original

    ammo_names = [
        qualified_definition('weapons', variant['weapon_name'])['ammo_name']
        for variant in stock
    ]
    tokens, aliases = ROCKETS['project_tokens'](read_game(ROCKETS['TOKEN_OVERRIDE']), ammo_names)
    require(aliases['authored_pairs'] == 141 and aliases['canonical_keys'] == 135,
            'original default token override inventory differs')
    variants, witnesses = [], []
    for selected, ammo_name in zip(stock, ammo_names):
        weapon = qualified_definition('weapons', selected['weapon_name'])
        ammo = qualified_definition('projectile_ammo', ammo_name)
        sight = selected['sight_name']
        qualified_definition('attachments', sight)
        attachments = weapon['attachment_list']
        require(weapon['required_sight_attachment'] == sight
                and [row['WeaponAttachment']['name'] for row in attachments.values()] == [sight],
                'stock launcher sight conflicts with the original weapon definition')
        inventory, flight, blast = project_ballistics(weapon, ammo, native['age_grace_seconds'])
        variants.append({
            'faction': selected['faction'],
            'item_id': selected['item_id'],
            'stock_sight_item_id': selected['stock_sight_item_id'],
            'ammo_token': tokens[ammo_name],
            'weapon': inventory, 'flight': flight, 'blast': blast,
        })
        witnesses.append({
            **selected,
            'ammo_name': ammo_name,
            'weapon_definition_sha256': sha(encode(weapon)),
            'ammo_definition_sha256': sha(encode(ammo)),
        })
    rules = {
        'schema': 'mag_retail_infantry_rockets_v1',
        'application_id': APPLICATION_ID,
        'source': {
            'retail_elf_sha256': native['retail_elf_sha256'],
            'retail_archive_sha256': SOURCE['GAME_ARCHIVE_SHA256'],
            'item_catalog_sha256': sha(catalog_raw),
            'item_bindings_sha256': sha(binding_raw),
            'bootstrap_sha256': sha(bootstrap_raw),
            'native_proof_sha256': sha(encode(native)),
        },
        'variants': variants,
    }
    proof = {
        'format': 'mag_retail_infantry_rocket_proof_v1',
        'rules_sha256': sha(encode(rules)),
        'native': native,
        'source_members_sha256': members,
        'token_override': aliases,
        'variants': witnesses,
    }
    return rules, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('elf', 'game-archive', 'item-catalog', 'item-bindings', 'bootstrap-loadouts', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--proof-output', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [args.elf, args.game_archive, args.item_catalog, args.item_bindings, args.bootstrap_loadouts]
    output_paths = [args.output, *([args.proof_output] if args.proof_output else [])]
    try:
        SOURCE['output_paths'](inputs, output_paths)
        elf = SOURCE['NATIVE']['Elf'](args.elf, 'retail')
        game_raw = args.game_archive.read_bytes()
        require(sha(game_raw) == SOURCE['GAME_ARCHIVE_SHA256'], 'wrong original retail game archive')
        game = SOURCE['NATIVE']['Psarc'](game_raw)

        def read_game(name):
            return SOURCE['archive_member'](game, name)

        rules, proof = project(args.item_catalog.read_bytes(), args.item_bindings.read_bytes(),
                               args.bootstrap_loadouts.read_bytes(), read_game, native_witness(elf))
        outputs = [(args.output, encode(rules))]
        if args.proof_output:
            outputs.append((args.proof_output, encode(proof)))
        SOURCE['write_outputs'](inputs, outputs, check=args.check)
    except (KeyError, ValueError, TypeError, OSError) as exc:
        parser.error(str(exc))
    print('Verified three original retail stock infantry launcher variants')


if __name__ == '__main__':
    main()
