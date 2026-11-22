#!/usr/bin/env python3
"""Project stock smoke metadata from an owner's original retail files, offline."""
import argparse
import math
from pathlib import Path
import runpy


HERE = Path(__file__).resolve().parent
INFANTRY = runpy.run_path(str(HERE / 'project-retail-infantry-rockets.py'))
SOURCE, ROCKETS, ITEMS = (INFANTRY[name] for name in ('SOURCE', 'ROCKETS', 'ITEMS'))
require, sha, encode, parse = (SOURCE[name] for name in ('require', 'sha', 'encode', 'parse'))
STOCK_KIT = 1
GRENADE_GEAR = 4
DAMAGE_CHANNELS = {'character', 'characterAPierce', 'characterARedux', 'vehicle',
                   'vehicleAPierce', 'vehicleARedux', 'objective', 'objectiveAPierce',
                   'objectiveARedux', 'mine'}


def select_stock(bootstrap, items, ids, original):
    factions = bootstrap['factions']
    by_faction = {row['faction']: row for row in factions}
    require(len(factions) == 3 and set(by_faction) == {0, 1, 2},
            'expected all three stock smoke factions')
    by_wire = {wire: ref for ref, wire in ids.items()}
    require(len(by_wire) == len(ids), 'ambiguous smoke item binding')
    selected = []
    for faction, row in sorted(by_faction.items()):
        require(type(faction) is int and len(row['loadouts']) > STOCK_KIT,
                'missing stock Marksman kit')
        kit = row['loadouts'][STOCK_KIT]['items']
        require(isinstance(kit, list) and len(kit) == 22, 'invalid stock Marksman kit')
        wire = kit[GRENADE_GEAR]
        require(type(wire) is int and 0 < wire < 4096 and wire in by_wire,
                'unbound original smoke item')
        ref = by_wire[wire]
        item = items[ref]
        namespace, ordinal = ref.split(':', maxsplit=1)
        group = f'FactionItemList{faction + 1}'
        name = item['name']
        require(namespace == group and ordinal.isascii() and ordinal.isdecimal()
                and item['authored_ref'] == ref and item['item_type'] == 'kWeapon'
                and item['definition'] == {'group': 'weapons', 'name': name}
                and original[group]['items'].get(f'ItemListData{ordinal}')
                == {'id': name, 'item_type': 'kWeapon'},
                'stock smoke faction or original ItemList differs')
        selected.append({'faction': faction, 'item_id': wire, 'item_ref': ref, 'name': name})
    return selected


def duration_ms(ammo, field):
    value = ammo[field]
    require(type(value) in (int, float) and math.isfinite(value)
            and 0 < value <= 300 and value * 1000 % 1 == 0,
            f'invalid smoke duration {field}')
    return int(value * 1000)


def project_parameters(weapon, ammo):
    require(weapon['category'] == 'kWepCategoryGrenade'
            and weapon['ammo_name'] == ammo['id'], 'unsupported smoke weapon binding')
    size, count = weapon['magazine_capacity'], weapon['initial_magazine_count']
    require(weapon['infinite_ammo'] is False and type(size) is int and size == 1
            and type(count) is int and 0 < count <= 255, 'unsupported smoke inventory')
    for field, expected in {
        'propulsion_strategy': 'kWepPropulsionStrategyThrown',
        'detonation_strategy': 'kWepDetonationStrategyTimer',
        'explosion_strategy': 'kWepExplosionStrategyVolume',
        'm_eAmmoPersistance': 'kWepPersistanceGrenadeSmoke',
    }.items():
        require(ammo[field] == expected, f'unsupported smoke {field}')
    require(ammo['destroy_objective'] is False and ammo['doHearingLoss'] is False
            and set(ammo['damage']) == DAMAGE_CHANNELS
            and all(type(value) in (int, float) and value == 0
                    for value in ammo['damage'].values()), 'smoke must be non-damaging')
    fuse = duration_ms(ammo, 'detonation_timer')
    lifetime = duration_ms(ammo, 'm_fLifetimeMax')
    require(fuse < lifetime, 'smoke fuse must precede projectile lifetime')
    return {'initial_count': size * count, 'fuse_ms': fuse,
            'projectile_lifetime_ms': lifetime,
            'volume_duration_ms': duration_ms(ammo, 'm_fDuration')}


def native_witness(elf):
    # Dispatch and scalar flow only; no family-5 wire schema is inferred here.
    for site, opcode in {
        0x48337C: 0xC01D000C, 0x48339C: 0xD007000C,
        0x482358: 0xC003000C, 0x482364: 0xEC000828, 0x482368: 0xD003000C,
        0x7B2F68: 0x801E000C, 0x7B3014: 0x4BFFFAFD, 0x7B301C: 0x907E000C,
        0x7B56C8: 0x813F000C, 0x7B56DC: 0xC1A9000C, 0x7B56E0: 0xC0090030,
        0x7B56E8: 0x419CFFC8, 0x7B5764: 0x4BFFA379, 0x7B5770: 0x901F000C,
    }.items():
        require(elf.word(site) == opcode, f'original smoke instruction differs at {site:#x}')
    spans = {'timer_clone': (0x483314, 0x4833A4), 'timer_update': (0x48234C, 0x4823C0),
             'volume_explosion': (0x7B2F24, 0x7B3090),
             'volume_strategy_update': (0x7B5678, 0x7B57A4)}
    return {'retail_elf_sha256': sha(elf.raw),
            'spans': {name: {'start': hex(start), 'end_exclusive': hex(end),
                             'sha256': sha(elf.read(start, end - start)[0])}
                      for name, (start, end) in spans.items()}}


def project(catalog_raw, binding_raw, bootstrap_raw, read_game, native):
    require(sha(bootstrap_raw) == SOURCE['COMBAT']['BOOTSTRAP_SHA256'],
            'unreviewed original retail bootstrap')
    catalog, ids, members = SOURCE['require_item_binding'](catalog_raw, binding_raw, read_game)
    bootstrap = parse(bootstrap_raw)
    INFANTRY['validate_bootstrap'](bootstrap, sha(catalog_raw), parse(binding_raw)['snapshot_sha256'])
    original_items = INFANTRY['authored_table'](read_game, 'item_list.zpr', 'ItemList')['factionItemLists']
    selected = select_stock(bootstrap, catalog['items'], ids, original_items)
    tables = {
        group: INFANTRY['indexed_definitions'](INFANTRY['authored_table'](read_game, file, kind + 'Table'), kind)
        for file, (group, kind) in ITEMS['GROUPS'].items()
        if group in ('weapons', 'projectile_ammo')
    }

    def definition(group, name):
        original = tables[group][name]
        require(original == catalog['definitions'][group][name]['parameters'],
                f'original smoke definition differs for {group}/{name}')
        return original

    ammo_names = [definition('weapons', row['name'])['ammo_name'] for row in selected]
    tokens, aliases = ROCKETS['project_tokens'](read_game(ROCKETS['TOKEN_OVERRIDE']), ammo_names)
    variants, witnesses = [], []
    for row, ammo_name in zip(selected, ammo_names, strict=True):
        weapon, ammo = definition('weapons', row['name']), definition('projectile_ammo', ammo_name)
        require(tokens[ammo_name] == catalog['definitions']['projectile_ammo'][ammo_name]['native_token_u32'],
                'original smoke token differs from bound catalog')
        variants.append({'faction': row['faction'], 'item_id': row['item_id'],
                         'ammo_token': tokens[ammo_name], **project_parameters(weapon, ammo)})
        witnesses.append({**row, 'ammo_name': ammo_name,
                          'weapon_definition_sha256': sha(encode(weapon)),
                          'ammo_definition_sha256': sha(encode(ammo))})
    rules = {
        'schema': 'mag_retail_smoke_grenades_v1', 'application_id': 20231,
        'source': {'retail_elf_sha256': native['retail_elf_sha256'],
                   'retail_archive_sha256': SOURCE['GAME_ARCHIVE_SHA256'],
                   'item_catalog_sha256': sha(catalog_raw), 'item_bindings_sha256': sha(binding_raw),
                   'bootstrap_sha256': sha(bootstrap_raw), 'native_proof_sha256': sha(encode(native))},
        'variants': variants,
    }
    proof = {'format': 'mag_retail_smoke_grenade_proof_v1', 'rules_sha256': sha(encode(rules)),
             'native': native, 'source_members_sha256': members,
             'token_override': aliases, 'variants': witnesses}
    return rules, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('elf', 'game-archive', 'item-catalog', 'item-bindings', 'bootstrap-loadouts', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--proof-output', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [args.elf, args.game_archive, args.item_catalog, args.item_bindings, args.bootstrap_loadouts]
    try:
        SOURCE['output_paths'](inputs, [args.output, *([args.proof_output] if args.proof_output else [])])
        elf = SOURCE['NATIVE']['Elf'](args.elf, 'retail')
        raw = args.game_archive.read_bytes()
        require(sha(raw) == SOURCE['GAME_ARCHIVE_SHA256'], 'wrong original retail game archive')
        game = SOURCE['NATIVE']['Psarc'](raw)
        read_game = lambda name: SOURCE['archive_member'](game, name)
        rules, proof = project(args.item_catalog.read_bytes(), args.item_bindings.read_bytes(),
                               args.bootstrap_loadouts.read_bytes(), read_game, native_witness(elf))
        outputs = [(args.output, encode(rules))]
        if args.proof_output:
            outputs.append((args.proof_output, encode(proof)))
        SOURCE['write_outputs'](inputs, outputs, check=args.check)
    except (KeyError, ValueError, TypeError, OSError) as exc:
        parser.error(str(exc))
    print('Verified three original retail stock smoke grenade variants')


if __name__ == '__main__':
    main()
