#!/usr/bin/env python3
"""Project the three original retail hospital secondary rocket rules.

The default mode binds them to Domination's hospital turret rules.
``--mode acquisition`` binds the same three emplacement rockets to the
Acquisition field-hospital turret rules from
``project-retail-hospital-turrets.py --mode acquisition``.
"""
import argparse
import math
from pathlib import Path
import re
import runpy
import struct


HERE = Path(__file__).resolve().parent
HOSPITAL = runpy.run_path(str(HERE / 'project-retail-hospital-turrets.py'))
NATIVE = HOSPITAL['NATIVE']
sha, encode, parse, require, name_token = (
    HOSPITAL[name] for name in ('sha', 'encode', 'parse', 'require', 'name_token')
)
SCRIPT_ROOT = HOSPITAL['SCRIPT_ROOT']
TOKEN_OVERRIDE = SCRIPT_ROOT + 'tokenoverride.zpr'
FIREPOINT = 'firepoint_02'

# Each mode's rocket rules bind only that mode's hospital turret rules.
MODES = {
    'domination': {
        'turrets': 'mag_domination_hospital_turrets_v1',
        'rockets': 'mag_domination_hospital_rockets_v1',
        'proof': 'mag_retail_domination_hospital_rocket_proof_v1',
    },
    'acquisition': {
        'turrets': 'mag_acquisition_hospital_turrets_v1',
        'rockets': 'mag_acquisition_hospital_rockets_v1',
        'proof': 'mag_retail_acquisition_hospital_rocket_proof_v1',
    },
}

FLIGHT_STRATEGIES = {
    'propulsion_strategy': ('propulsion_strategy', 'kWepPropulsionStrategyRocketStraight'),
    'detonation_strategy': ('detonation_strategy', 'kWepDetonationStrategyImpact'),
    'explosion_strategy': ('explosion_strategy', 'kWepExplosionStrategyDebris'),
    'collision_shape': ('collisionType', 'kWepProjCollisionShapeSphere'),
}


def project_tokens(raw, names):
    pairs = re.compile(r'"([^"]*)"\s*,\s*"([^"]*)"\s*,?\s*(?://.*)?')
    counts = {}
    for line in raw.decode('utf-8-sig').splitlines():
        line = line.strip()
        if not line or line.startswith('//'):
            continue
        match = pairs.fullmatch(line)
        require(match is not None, 'malformed original token override entry')
        key = name_token(match[1])
        counts[key] = counts.get(key, 0) + 1

    selected = {name: name_token(name) for name in names}
    keys = set(selected.values())
    require(len(keys) == len(selected) == len(names) and 0 not in keys,
            'duplicate or invalid rocket source name')
    require(keys.isdisjoint(counts), 'original default token override aliases a rocket resource')
    proof = {
        'member': TOKEN_OVERRIDE,
        'sha256': sha(raw),
        'authored_pairs': sum(counts.values()),
        'canonical_keys': len(counts),
        'duplicate_key_counts': {f'0x{key:08x}': count
                                 for key, count in sorted(counts.items()) if count > 1},
        'unmapped_original_names': selected,
    }
    return selected, proof


def project_muzzle(model, firepoint):
    nodes = model['nodes']
    transforms = model['reference_transforms']
    chain, references = {}, {}
    for index, (slot, name) in enumerate(zip(
        ('root', 'yaw', 'pitch', 'firepoint'),
        ('gunner_seat', 'turret_yaw', 'turret_pitch', firepoint),
    )):
        name_key = name_token(name)
        matching = [node for node in nodes if node.name_token == name_key]
        require(len(matching) == 1, f'missing or ambiguous original hospital rocket {slot} node')
        node = matching[0]
        parent = 0xffff if index == 0 else index - 1
        require((node.node_id, node.parent_node_id, node.transform_index) == (index, parent, index)
                and index < len(transforms), 'original hospital rocket reference chain differs')
        local = transforms[index]
        position, rotation = local.translation_xyz, local.quaternion_xyzw
        require(len(position) == 3 and len(rotation) == 4
                and all(math.isfinite(value) for value in (*position, *rotation))
                and abs(sum(value * value for value in rotation) - 1.) < 1e-5
                and local.uniform_scale == 1., 'unqualified original hospital rocket transform')
        chain[slot] = {'position_xyz': position, 'rotation_xyzw': rotation}
        references[slot] = {
            'name': name, 'name_token': name_key, 'node_id': node.node_id,
            'parent_node_id': parent, 'transform_index': node.transform_index,
        }
    require(chain['root'] == {'position_xyz': (0., 0., 0.), 'rotation_xyzw': (0., 0., 0., 1.)}
            and chain['firepoint']['position_xyz'] != (0., 0., 0.)
            and chain['firepoint']['rotation_xyzw'] == (0., 0., 0., 1.),
            'unqualified original hospital rocket root or firepoint pose')
    return chain, references


def number(value, field, minimum, maximum):
    require(type(value) in (int, float) and minimum <= value <= maximum,
            f'unqualified original hospital rocket {field}')
    return float(value)


def project_weapon(weapon):
    fire_mode = weapon['fire_mode']
    infinite = weapon['infinite_ammo']
    size = weapon['magazine_capacity']
    initial_magazines = weapon['initial_magazine_count']
    require(type(infinite) is bool
            and type(size) is int and 1 <= size <= 255
            and type(initial_magazines) is int and 1 <= initial_magazines <= 255,
            'unqualified original hospital rocket magazine')
    return {
        'refire_seconds': number(fire_mode['refire_time'], 'refire time', 0.001, 60.),
        'reload_seconds': number(fire_mode['reload_time'], 'reload time', 0.001, 120.),
        'infinite_ammo': infinite,
        'magazine_size': size,
        'initial_magazine_count': initial_magazines,
    }


def project_ballistics(weapon, ammo, age_grace):
    modes = weapon['fire_mode']['fire_modes']
    require(weapon['id'] == weapon['ammo_name'] == ammo['id']
            and weapon['category'] == 'kWepCategoryTurretRocket'
            and weapon['fire_param_id'] == 'kFireParamSecondary'
            and weapon['initial_fire_mode'] == 'kSemiAuto'
            and list(modes) == ['FireModeTypeClass0']
            and modes['FireModeTypeClass0']['FireModeTypeClass']['fire_mode_value'] == 'kSemiAuto'
            and weapon['use_turret_mesh'] is True and weapon['model_name'] == ''
            and weapon['alternate_firepoint'] == FIREPOINT, 'unqualified original secondary rocket')
    flight = {}
    for field, (authored_field, strategy) in FLIGHT_STRATEGIES.items():
        require(ammo[authored_field] == strategy, f'unqualified original hospital rocket {field}')
        flight[field] = ammo[authored_field]
    flight.update({
        'radius_m': number(ammo['radius'], 'collision radius', 0.001, 1.),
        'speed_mps': number(ammo['launch_velocity'], 'launch speed', 1., 500.),
        'lifetime_seconds': number(ammo['m_fLifetimeMax'], 'lifetime', 0.01, 60.),
        'age_grace_seconds': number(age_grace, 'generic age grace', 0., 10.),
        'armed_time_seconds': number(ammo['m_fArmedTime'], 'armed timer', -1., -1.),
        'detonation_timer_seconds': number(ammo['detonation_timer'], 'impact timer', 0., 0.),
    })
    damage = ammo['damage']
    for field in ('vehicleAPierce', 'vehicleARedux', 'objectiveAPierce', 'objectiveARedux'):
        number(damage[field], field, 0., 0.)
    blast = {
        'half_radius_m': number(ammo['half_damage_radius'], 'half-damage radius', 0.001, 100.),
        'outer_radius_m': number(ammo['full_damage_radius'], 'outer radius', 0.001, 100.),
        'material_penetration': number(ammo['MaterialPenetration'], 'material penetration', 0., 20.),
        **{f'{channel}_hp': number(damage[channel], f'{channel} channel', 0.001, 10000.)
           for channel in ('character', 'vehicle', 'objective')},
    }
    require(blast['half_radius_m'] < blast['outer_radius_m'],
            'original hospital rocket radius ordering differs')
    return flight, blast


def native_witness(elf):
    def literal(address, content):
        require(elf.read(address, len(content))[0] == content,
                'original hospital rocket native literal differs')

    require(elf.word(0x1b3231c) == 0x15fa998,
            'original default token override loader path differs')
    literal(0x15fa998, b'/scripts/tokenoverride.zpr\0')
    literal(elf.word(elf.word(0x1b3f530)), b'alternate_firepoint\0')
    literal(elf.word(elf.word(0x1b3f678)), b'use_turret_mesh\0')
    for address, word in {
        0x49194c: 0x38c0044c, 0x49295c: 0x38c00519, 0x10389d0: 0x3863044c,
        0x7b29e8: 0xa009133c, 0x7b2a18: 0x38000017, 0x7acec0: 0x5529801e,
    }.items():
        require(elf.word(address) == word, 'original hospital rocket native field differs')
    families = list(struct.unpack('>4I', elf.read(0x15af0b8, 16)[0]))
    require(families == [16, 17, 18, 19], 'original projectile family slots differ')
    spans = {
        'token_override_loader': (0x234148, 0x234a20),
        'alternate_firepoint_reflection': (0x491948, 0x491968),
        'use_turret_mesh_reflection': (0x492958, 0x492974),
        'alternate_feature_selection': (0x10389bc, 0x1038a38),
        'equipped_external_geometry': (0x6e3c18, 0x6e3c58),
        'geometry_and_node_cache': (0x764f00, 0x76500c),
        'secondary_launch_origin': (0x6e63c8, 0x6e647c),
        'barrel_transform_consumer': (0x6e5bb8, 0x6e5d90),
        'straight_propulsion': (0x484248, 0x4842b0),
        'impact_strategy': (0x480f48, 0x480fe0),
        'initial_projectile_filter': (0x7b29dc, 0x7b2a3c),
        'sphere_filter_pack': (0x7ace44, 0x7acf60),
        'projectile_age_and_cleanup': (0x7a43c0, 0x7a44bc),
        'debris_explosion': (0x7a4cf8, 0x7a5184),
        'debris_character_damage': (0x108b910, 0x108badc),
        'debris_body_and_cover': (0x108b368, 0x108b848),
        'radius_damage_scale': (0x7a23d4, 0x7a24e0),
    }
    return {
        'retail_elf_sha256': sha(elf.raw),
        'projectile_slot_families': families,
        'raw_collision_layer': 23,
        'associated_group_shift': 16,
        'age_grace_seconds': struct.unpack('>f', elf.read(0x1b4cafc, 4)[0])[0],
        'spans': {name: {'start': hex(start), 'end_exclusive': hex(end),
                         'sha256': sha(elf.read(start, end - start)[0])}
                  for name, (start, end) in spans.items()},
    }


def project(hospital_raw, hospital_rules, hospital_proof, catalog_raw,
            read_game, read_main, decode_model, native, mode='domination'):
    schemas = MODES[mode]
    require(hospital_rules['schema'] == schemas['turrets'],
            f'{mode} rockets need {mode} hospital turret rules')
    require(hospital_raw == encode(hospital_rules),
            'supplied hospital turret rules differ from the six reproduced source profiles')
    catalog = parse(catalog_raw)
    base = hospital_rules['source']
    models = HOSPITAL['indexed'](hospital_proof['pose']['models'], 'faction')
    equipment = HOSPITAL['indexed'](hospital_proof['wire']['equipment_by_faction'], 'faction')
    ammo_names = [row['secondary']['ammo_name'] for row in equipment.values()]
    tokens, override = project_tokens(read_game(TOKEN_OVERRIDE), [*ammo_names, FIREPOINT])
    require(override['authored_pairs'] == 141 and override['canonical_keys'] == 135,
            'original default token override table differs')
    variants, sources = [], []
    for row in hospital_rules['variants']:
        faction = row['faction']
        second = equipment[faction]['secondary']
        require(second['wire_id'] == row['secondary_item_id'], 'hospital secondary item differs')
        definitions = catalog['definitions']
        ammo_name = second['ammo_name']
        weapon = definitions['weapons'][second['name']]['parameters']
        ammo = definitions['projectile_ammo'][ammo_name]['parameters']
        flight, blast = project_ballistics(weapon, ammo, native['age_grace_seconds'])
        model = models[faction]
        body = read_main(model['body_member'])
        require(sha(body) == model['body_member_sha256'], 'original rocket hospital model changed')
        geometry = decode_model(body, model['model_token'])
        muzzle, references = project_muzzle(geometry, weapon['alternate_firepoint'])
        variants.append({
            'faction': faction, 'secondary_item_id': second['wire_id'],
            'ammo_name': ammo_name, 'ammo_token': tokens[ammo_name],
            'muzzle': muzzle, 'weapon': project_weapon(weapon), 'flight': flight, 'blast': blast,
        })
        for reference in references.values():
            reference['node_evidence'] = geometry['node_evidence'][reference['node_id']]
            reference['transform_evidence'] = geometry['transform_evidence'][reference['transform_index']]
        sources.append({
            'faction': faction, 'weapon': second, 'model': model,
            'reference_nodes': references,
        })
    proof = {
        'format': schemas['proof'],
        'hospital_turrets_sha256': sha(hospital_raw),
        'hospital_turret_proof_sha256': sha(encode(hospital_proof)),
        'retail_archive_sha256': base['retail_archive_sha256'],
        'retail_main_sha256': HOSPITAL['MAIN_ARCHIVE_SHA256'],
        'item_catalog_sha256': base['item_catalog_sha256'],
        'source_members_sha256': hospital_proof['wire']['item_members_sha256'],
        'native': native, 'token_override': override, 'variants': sources,
    }
    rules = {
        'schema': schemas['rockets'],
        'source': {
            'retail_elf_sha256': base['retail_elf_sha256'],
            'retail_archive_sha256': base['retail_archive_sha256'],
            'retail_main_sha256': HOSPITAL['MAIN_ARCHIVE_SHA256'],
            'item_catalog_sha256': base['item_catalog_sha256'],
            'hospital_turrets_sha256': sha(hospital_raw),
            'launch_proof_sha256': sha(encode(proof)),
            'token_override_sha256': override['sha256'],
            'vehicle_type_token': base['vehicle_type_token'],
        },
        'variants': variants,
    }
    return rules, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=tuple(MODES), default='domination')
    for name in ('elf', 'game-archive', 'main-archive', 'hospital-turrets',
                 'item-catalog', 'item-bindings', 'structures', 'output', 'proof-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--hospital-sources', type=Path, help='Domination hospital placer inputs')
    parser.add_argument('--profile', metavar=('SPAWNS', 'HOSPITALS'), type=Path, nargs=2,
                        required=True, action='append', help='repeat once per profile')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    domination = args.mode == 'domination'
    if domination and args.hospital_sources is None:
        parser.error('domination mode requires --hospital-sources')
    if not domination and args.hospital_sources is not None:
        parser.error('acquisition mode reads the hospital variants from the original archives')
    inputs = [args.elf, args.game_archive, args.main_archive,
              *([args.hospital_sources] if domination else []),
              args.hospital_turrets, args.item_catalog, args.item_bindings, args.structures,
              *(path for pair in args.profile for path in pair)]
    HOSPITAL['output_paths'](inputs, [args.output, args.proof_output])
    require(len(args.profile) == 6, f'expected six {args.mode.capitalize()} profile pairs')
    elf = NATIVE['Elf'](args.elf, 'retail')
    game_raw, main_raw = args.game_archive.read_bytes(), args.main_archive.read_bytes()
    require(sha(game_raw) == HOSPITAL['GAME_ARCHIVE_SHA256']
            and sha(main_raw) == HOSPITAL['MAIN_ARCHIVE_SHA256'],
            'wrong original retail game or main archive')
    game, main_archive = map(NATIVE['Psarc'], (game_raw, main_raw))

    def read_game(name):
        return HOSPITAL['archive_member'](game, name)

    def read_main(name):
        return HOSPITAL['archive_member'](main_archive, name)

    catalog = args.item_catalog.read_bytes()
    decode = HOSPITAL['read_model']
    common = (catalog, args.item_bindings.read_bytes(), args.structures.read_bytes(),
              [(a.read_bytes(), b.read_bytes()) for a, b in args.profile])
    if domination:
        hospital_rules, hospital_proof = HOSPITAL['project'](
            args.hospital_sources.read_bytes(), *common,
            read_game, read_main, decode, HOSPITAL['native_witness'](elf),
        )
    else:
        hospital_rules, hospital_proof = HOSPITAL['project_acquisition'](
            *common, read_game, read_main, decode,
            HOSPITAL['body_locator'](main_archive), HOSPITAL['native_witness'](elf),
        )
    rules, proof = project(
        args.hospital_turrets.read_bytes(), hospital_rules, hospital_proof, catalog,
        read_game, read_main, decode, native_witness(elf), args.mode,
    )
    HOSPITAL['write_outputs'](inputs, [(args.output, encode(rules)), (args.proof_output, encode(proof))],
                              check=args.check)
    print('Verified three original hospital rocket variants and six bound '
          f'{args.mode.capitalize()} profiles')


if __name__ == '__main__':
    main()
