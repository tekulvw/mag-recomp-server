#!/usr/bin/env python3
"""Project source-qualified retail Domination hospital turret rules offline."""
import argparse
from pathlib import Path
import re
import runpy
import sys
import zlib


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
COMBAT = runpy.run_path(str(HERE / 'project-retail-combat.py'))
MAP = runpy.run_path(str(HERE / 'retail_map_helpers.py'))
NATIVE = runpy.run_path(str(HERE / 'extract-retail-falling-damage.py'))
sha, encode, parse, require = (COMBAT[name] for name in ('sha', 'encode', 'parse', 'check'))
RETAIL_ELF_SHA256 = NATIVE['SOURCES']['retail']
GAME_ARCHIVE_SHA256 = NATIVE['SOURCES']['archive']
MAIN_ARCHIVE_SHA256 = 'a2e8345cd9f5c650ac22544096ba83e8c9e6f0fffd0ef69eb8791fb265f9baad'
BINDING_REPORT_SHA256 = COMBAT['BINDING_REPORT_SHA256']
BINDING_POSITIONS = 667
FACTIONS = ('Valor', 'Raven', 'SVER')
SQUADS = ('Alpha', 'Bravo', 'Charlie', 'Delta')
SCRIPT_ROOT = '/run/mag/ps3/scripts/'
POSE_SCOPE = 'retail_root0_identity_fallback'
PASSENGER_SCOPE = 'retail_unresolved_passenger_node_zero_local_pose'
ALLOCATION_SCOPE = 'successful_production_fresh_level_load'


def name_token(name):
    return zlib.crc32(name.lower().replace('\\', '/').encode('ascii'))


def token(value):
    result = int(value, 16) if isinstance(value, str) and value.startswith('0x') else value
    require(type(result) is int and 0 < result <= 0xffffffff, 'invalid hospital source token')
    return result


def digest(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None


def indexed(rows, field):
    result = {row[field]: row for row in rows}
    require(len(result) == len(rows), f'duplicate hospital source {field}')
    return result


def archive_member(archive, name):
    require(archive.names.count(name) == 1, f'missing or ambiguous original member {name}')
    return archive.extract(archive.names.index(name) + 1)


def read_model(body, model_token):
    sys.path.insert(0, str(ROOT / 'tools/collision'))
    from rigid_hierarchy import parse_rigid_resource
    from zrb_records import parse as parse_zrb

    records = [row for row in parse_zrb(body, retail13_compat=True)['records']
               if row['class_token'] == 'a4f4d23c' and row['name_token'] == f'{model_token:08x}']
    require(len(records) == 1, 'expected a unique original hospital render hierarchy')
    return parse_rigid_resource(body, records[0])


def native_witness(elf):
    # The caller's Elf loader already verifies the complete supported PPC64 image.
    def spans(ranges):
        return {name: {'start': hex(a), 'end_exclusive': hex(b),
                       'sha256': sha(elf.read(a, b - a)[0])}
                for name, (a, b) in ranges.items()}

    fallback = elf.word(0x1b45b04 + 0x4e1c)
    require(elf.read(fallback, 12)[0] == b'gunner_seat\0', 'original seat fallback literal differs')
    return {
        'retail_elf_sha256': sha(elf.raw),
        'pose': {'fallback_name': 'gunner_seat', 'fallback_address': hex(fallback),
                 'spans': spans({
                     'seat_lookup_dispatch': (0x701a98, 0x701ad8),
                     'seat_fallback_construction': (0x701300, 0x70137c),
                     'seat_world_transform': (0x6c9f5c, 0x6ca0bc),
                     'hulk_activate_heading': (0x88d3ac, 0x88d4b4),
                     'hulk_refresh_heading': (0x88d7a0, 0x88d828),
                 })},
        'wire': {'spans': spans({
            'family13_client_registration': (0x71e08, 0x71e38),
            'vehicle_field_registration': (0x445e9c, 0x446478),
            'overheat_field_registration': (0x446a38, 0x446b00),
            'angle_markup_widths': (0x43fc18, 0x43fd88),
            'three_slot_inventory_initialization': (0x87c2a0, 0x87c300),
            'inventory_item_width': (0x4574c4, 0x457604),
            'primary_and_secondary_version_gates': (0x6dbeb8, 0x6dc088),
            'hit_gear_and_mode': (0x76acc0, 0x76ad34),
            'mounted_hit_kit': (0x7c7c14, 0x7c7ef0),
        })},
    }


def require_item_binding(catalog_raw, binding_raw, read_game):
    require(sha(binding_raw) == BINDING_REPORT_SHA256, 'unreviewed retail item binding')
    catalog, report = map(parse, (catalog_raw, binding_raw))
    binding = report['binding_configuration']
    catalog_hash = sha(catalog_raw)
    require(catalog['schema_version'] == 2 and catalog['application_id'] == 20231
            and catalog['build']['source_archive_sha256'] == GAME_ARCHIVE_SHA256
            and report.get('catalog_sha256') == catalog_hash
            and binding.get('source_catalog_sha256') == catalog_hash,
            'different retail item catalog')
    ids = binding['by_authored_ref']
    require(report['validated'] is True and report['positions_compared'] == BINDING_POSITIONS
            and report['mismatch_count'] == 0 and report['mismatches'] == []
            and binding['application_id'] == 20231
            and binding['original_retail_elf_sha256'] == RETAIL_ELF_SHA256
            and len(ids) == BINDING_POSITIONS and ids.keys() == catalog['items'].keys()
            and all(type(wire_id) is int and 0 <= wire_id < 4096 for wire_id in ids.values())
            and len(set(ids.values())) == len(ids), 'invalid retail item binding')
    members = {}
    for filename in ('item_list.zpr', 'weapons.lua', 'hitscanammo.lua', 'projectileammo.lua'):
        member = SCRIPT_ROOT + filename
        members[member] = sha(read_game(member))
        require(members[member] == catalog['sources'][filename]['sha256'],
                f'original item catalog source member differs for {filename}')
    return catalog, ids, members


def weapon_binding(catalog, ids, faction, name, slot):
    namespace = f'FactionItemList{faction + 1}:'
    matches = [ref for ref, row in catalog['items'].items()
               if ref.startswith(namespace) and row['name'] == name]
    require(len(matches) == 1, f'expected one faction item binding for turret slot {slot}')
    ref = matches[0]
    item = catalog['items'][ref]
    require(item['authored_ref'] == ref and item['item_type'] == 'kWeapon'
            and item['definition'] == {'group': 'weapons', 'name': name}
            and 0 < ids[ref] < 4096, f'wrong original turret item binding for slot {slot}')
    definitions = catalog['definitions']
    weapon = definitions['weapons'][name]['parameters']
    ammo_group = ('hitscan_ammo', 'projectile_ammo')[slot]
    category = ('kWepCategoryTurretGun', 'kWepCategoryTurretRocket')[slot]
    ammo_name = weapon['ammo_name']
    require(weapon['category'] == category and ammo_name in definitions[ammo_group],
            f'wrong original turret weapon definition for slot {slot}')
    if slot == 0:
        require(weapon['initial_fire_mode'] == 'kAuto'
                and list(weapon['fire_mode']['fire_modes']) == ['FireModeTypeClass0']
                and weapon['fire_mode']['fire_modes']['FireModeTypeClass0']['FireModeTypeClass']
                ['fire_mode_value'] == 'kAuto', 'unsupported primary turret fire-mode definition')
    return {'slot': slot, 'name': name, 'authored_ref': ref, 'wire_id': ids[ref],
            'ammo_group': ammo_group, 'ammo_name': ammo_name}


def model_pose(body, variant, seat, decode_model):
    model_token = name_token(variant['ModelFileName'])
    model = decode_model(body, model_token)
    nodes = model['nodes']
    authored_name = seat['Name']
    authored_token = name_token(authored_name)
    fallback_token = name_token('gunner_seat')
    require(authored_name and all(node.name_token != authored_token for node in nodes),
            'authored seat already resolves in the original hospital model')
    fallback = [node for node in nodes if node.name_token == fallback_token]
    require(len(fallback) == 1, 'original gunner_seat fallback is missing or ambiguous')
    root = fallback[0]
    require((root.node_id, root.transform_index, root.parent_node_id) == (0, 0, 0xffff),
            'original gunner_seat fallback must use identity root 0')
    local = model['reference_transforms'][root.transform_index]
    require(local.translation_xyz == (0., 0., 0.)
            and local.quaternion_xyzw == (0., 0., 0., 1.) and local.uniform_scale == 1.,
            'original gunner_seat fallback must use identity root 0')
    pivots = [(node.node_id, node.transform_index, node.parent_node_id, node.name_token)
              for node in nodes[1:3]]
    require(pivots == [(1, 1, 0, name_token('turret_yaw')), (2, 2, 1, name_token('turret_pitch'))],
            'original attached turret pivot chain differs')
    return {
        'model_token': model_token, 'model_record_sha256': model['source_sha256'],
        'authored_seat_name': authored_name, 'authored_seat_token': authored_token,
        'fallback_node': {'name': 'gunner_seat', 'name_token': root.name_token,
                          'node_id': root.node_id, 'transform_index': root.transform_index,
                          'parent_node_id': root.parent_node_id, 'translation_xyz': local.translation_xyz,
                          'rotation_xyzw': local.quaternion_xyzw, 'uniform_scale': local.uniform_scale},
        'attached_pivots': pivots,
    }


def project_variants(sources, structures, catalog, ids, read_game, read_main, decode_model):
    require(set(sources['faction_sources']) == set(FACTIONS), 'incomplete original hospital factions')
    vehicle_name = 'DefensiveEmplacement'
    vehicle_type = name_token(vehicle_name)
    registry_raw = read_game(MAP['VEHICLES_MEMBER'])
    registry_hash = sha(registry_raw)
    require(registry_hash == sources['vehicle_registry_sha256'] == structures['source_sha256']['vehicles.zpr'],
            'original hospital vehicle registry differs')
    parse_zpr = MAP['parse_zpr']
    registry = MAP['vehicle_registry'](parse_zpr(registry_raw.decode()))
    definitions = registry[f'0x{vehicle_type:08x}']['variants']
    variants, poses, equipment = [], [], []
    for faction, name in enumerate(FACTIONS):
        source = sources['faction_sources'][name]
        matching = [row for row in definitions if row['faction'] == name and row['ModifierLevel'] == '0']
        require(matching == [source['variant']], f'original hospital registry variant differs for {name}')
        variant = matching[0]
        filename = variant['PropertiesFileName']
        require(variant['Class'] == 'VehicleClass_DefensiveEmplacement'
                and Path(filename).name == filename and filename.endswith('.zpr'),
                f'invalid hospital vehicle properties selector for {name}')
        script = f'vehicle/{filename}'
        member = SCRIPT_ROOT + script
        require(member == source['source_member'], f'original hospital property member differs for {name}')
        properties_raw = read_game(member)
        properties_hash = sha(properties_raw)
        require(properties_hash == source['sha256'] == structures['source_sha256'][script],
                f'original hospital property member differs for {name}')
        profiled = [row for row in structures['profiles'] if row['properties_source'] == script
                    and row['vehicle_type'] == vehicle_name and row['faction'] == faction]
        require(len(profiled) == 1
                and profiled[0]['model'] == variant['ModelFileName'],
                f'original defensive emplacement structure row differs for {name}')
        properties = parse_zpr(properties_raw.decode())['DefensiveEmplacementProperties']
        require(properties['Seats'] == source['seats'], f'original hospital seat properties differ for {name}')
        seat = properties['Seats']['SeatPropertiesList']['SeatProperties0']['SeatProperties']
        require(seat['SeatType'] == 'TurretOperatorSeat' and seat['TurretIndex'] == '0'
                and seat['occupantIsVisible'] == 'false', f'unqualified hospital seat 0 for {name}')
        neutral = source['neutral_body']
        body = read_main(neutral['member'])
        body_hash = sha(body)
        require(body_hash == neutral['member_sha256'], f'original hospital body member differs for {name}')
        pose = model_pose(body, variant, seat, decode_model)
        poses.append(dict(pose, faction=faction, property_member=member, property_sha256=properties_hash,
                          body_member=neutral['member'], body_member_sha256=body_hash))
        turret = properties['Turrets']['TurretProps']['TurretProperties0']['TurretProperties']
        primary = weapon_binding(catalog, ids, faction, turret['Weapon1'], 0)
        secondary = weapon_binding(catalog, ids, faction, turret['Weapon2'], 1)
        require(primary['wire_id'] != secondary['wire_id'], f'duplicate hospital equipment for {name}')
        equipment.append({'faction': faction, 'primary': primary, 'secondary': secondary,
                          'definition_slots': [primary['wire_id'], secondary['wire_id'], None]})
        variants.append({'faction': faction, 'primary_item_id': primary['wire_id'],
                         'secondary_item_id': secondary['wire_id'], 'properties_source': script,
                         'seat_node': 'gunner_seat', 'seat_index': 0, 'turret_index': 0,
                         'seat_pose_scope': POSE_SCOPE, 'occupant_visible': False})
    return variants, poses, equipment, registry_hash, vehicle_type


def project_placements(pool, hospitals, level, variant, vehicle_type):
    placements = indexed(hospitals['placements'], 'object_index')
    source = indexed(level['hospitals'], 'candidate_type13_index_if_all_initial_creations_succeed')
    points = indexed([row for row in pool['locations'] if row['kind'] == 'bunker_passenger'], 'index')
    squads = indexed(pool['squads'], 'index')
    require(set(placements) == set(source) == set(range(16)) and len(points) == 16
            and all(type(index) is int for index in placements)
            and len({row['spawn_index'] for row in placements.values()}) == 16,
            'incomplete indexed hospital placement identity')
    result = []
    for index in range(16):
        row, authored = placements[index], source[index]
        spawn_index = row['spawn_index']
        point = points.get(spawn_index)
        link = authored['squad_link']
        platoon = link['platoon_id']
        require(type(platoon) is int and 0 <= platoon < 4 and link['squad'] in SQUADS,
                'invalid original hospital squad placement identity')
        squad_num = SQUADS.index(link['squad'])
        squad = squads.get(16 + platoon * 4 + squad_num)
        spawn_token = token(authored['spawn_token'])
        require(type(spawn_index) is int and 0 <= spawn_index < 128
                and spawn_index == authored['spawn_index']
                and type(row['spawn_token']) is int and row['spawn_token'] == spawn_token
                and row['properties_source'] == variant['properties_source']
                and token(authored['vehicle_type_token']) == vehicle_type
                and authored['team'] == link['team'] == 1 and link['slot'] == 2
                and point is not None and point['active'] is True and point['token'] == spawn_token
                and squad is not None and squad['team'] == 1 and squad['faction'] == variant['faction']
                and squad['platoon'] == platoon and squad['squad'] == squad_num
                and len(squad['available']) > 2 and squad['available'][2] == spawn_index,
                f'hospital placement identity differs at allocation {index}')
        result.append({'object_index': index, 'spawn_index': spawn_index, 'spawn_token': spawn_token,
                       'faction': variant['faction']})
    return result


def project_profiles(profiles, levels, variants, structure_hash, source, vehicle_type):
    require(len(profiles) == 6, 'expected six Domination profile pairs')
    expected = {(name, attacker) for name, level in levels.items()
                for attacker in range(3) if attacker != level['owner_faction_id']}
    seen, hashes = set(), set()
    rules, evidence = [], []
    for spawn_raw, hospital_raw in profiles:
        spawns, hospitals = map(parse, (spawn_raw, hospital_raw))
        spawn_hash = sha(spawn_raw)
        name = spawns['active']['map_name']
        pool, = spawns['maps']
        policy = spawns['match_profile']
        attacker, owner = policy['attacking_faction'], pool['owner_faction']
        key = (name, attacker)
        require(type(attacker) is int and type(owner) is int
                and key in expected and key not in seen and spawn_hash not in hashes,
                'incomplete or duplicate Domination profile coverage')
        level = levels[name]
        provenance = parse(spawns['source'])
        require(spawns['format'] == 'mag_spawn_catalog' and spawns['schema_version'] == 1
                and policy['game_type'] == 2 and policy['auto_assign_teams'] is True
                and policy['has_bunkers'] is True
                and owner == level['owner_faction_id'] and pool['name'] == name
                and pool['runtime_indices_verified'] is True and provenance['map'] == name
                and provenance['game_archive_sha256'] == GAME_ARCHIVE_SHA256
                and provenance['native_spawn_bindings_sha256'] == source['spawn_binding_sha256']
                and provenance['level_archive_sha256'] == level['level_source']['archive_sha256']
                and provenance['level_clv_sha256'] == level['level_source']['clv_sha256'],
                f'Domination spawn source differs for {name}')
        require(hospitals['format'] == 'mag_domination_hospital_placements_v1'
                and hospitals['map_name'] == name and hospitals['allocation_scope'] == ALLOCATION_SCOPE
                and hospitals['passenger_pose_scope'] == PASSENGER_SCOPE
                and digest(hospitals['passenger_pose_sha256'])
                and hospitals['spawn_catalog_sha256'] == spawn_hash
                and hospitals['structure_catalog_sha256'] == structure_hash
                and digest(hospitals['source_proof_sha256'])
                and hospitals['source_proof_sha256'] == provenance['hospital_allocation_sha256']
                and token(hospitals['vehicle_type_token']) == vehicle_type,
                f'unbound hospital placement catalog for {name}')
        placements = project_placements(pool, hospitals, level, variants[owner], vehicle_type)
        rules.append({'map_name': name, 'spawn_catalog_sha256': spawn_hash, 'placements': placements})
        evidence.append({'map_name': name, 'attacking_faction': attacker, 'defending_faction': owner,
                         'spawn_catalog_sha256': spawn_hash, 'hospital_placements_sha256': sha(hospital_raw),
                         'allocation_sha256': hospitals['source_proof_sha256']})
        seen.add(key)
        hashes.add(spawn_hash)
    require(seen == expected, 'incomplete Domination profile coverage')
    return (sorted(rules, key=lambda row: (row['map_name'], row['spawn_catalog_sha256'])),
            sorted(evidence, key=lambda row: (row['map_name'], row['attacking_faction'])))


def project(sources_raw, catalog_raw, binding_raw, structure_raw, profiles,
            read_game, read_main, decode_model, native):
    source, structures = map(parse, (sources_raw, structure_raw))
    require(source['format'] == 'mag_retail_domination_hospital_placer_inputs' and source['schema_version'] == 1
            and source['retail_elf_sha256'] == native['retail_elf_sha256'] == RETAIL_ELF_SHA256
            and source['retail_game_sha256'] == GAME_ARCHIVE_SHA256
            and source['retail_main_sha256'] == MAIN_ARCHIVE_SHA256 and digest(source['spawn_binding_sha256']),
            'wrong original retail hospital source identity')
    require(structures['schema_version'] == 1 and structures['application_id'] == 20231
            and structures['source_archive_sha256'] == GAME_ARCHIVE_SHA256
            and structures['native_rules']['original_elf_sha256'] == RETAIL_ELF_SHA256
            and structures['item_catalog_sha256'] == sha(catalog_raw)
            and structures['retail_item_binding_sha256'] == sha(binding_raw),
            'wrong original retail structure source')
    levels = indexed(source['maps'], 'map')
    require(len(levels) == 3 and {row['owner_faction_id'] for row in levels.values()} == {0, 1, 2}
            and all(type(row['owner_faction_id']) is int
                    and row['owner_faction'] == FACTIONS[row['owner_faction_id']]
                    and digest(row['level_source']['archive_sha256'])
                    and digest(row['level_source']['clv_sha256']) for row in levels.values()),
            'incomplete original hospital maps')
    catalog, ids, item_members = require_item_binding(catalog_raw, binding_raw, read_game)
    variants, models, equipment, registry_hash, vehicle_type = project_variants(
        source, structures, catalog, ids, read_game, read_main, decode_model)
    structure_hash = sha(structure_raw)
    profiles, profile_proof = project_profiles(profiles, levels, variants, structure_hash, source, vehicle_type)
    pose = {'scope': POSE_SCOPE, 'retail_elf_sha256': RETAIL_ELF_SHA256,
            'retail_main_sha256': MAIN_ARCHIVE_SHA256, 'retail_game_sha256': GAME_ARCHIVE_SHA256,
            'vehicle_registry_sha256': registry_hash, 'models': models, 'native': native['pose']}
    wire = {'retail_elf_sha256': RETAIL_ELF_SHA256, 'retail_game_sha256': GAME_ARCHIVE_SHA256,
            'family': 13, 'allocations': 16, 'vehicle_fields': 25,
            'owners': {'vehicle': 1, 'turret0': 6, 'bits': 16},
            'yaw': {'field': 8, 'sign_bits': 1, 'integer_bits': 3, 'fraction_bits': 7},
            'pitch': {'field': 9, 'sign_bits': 1, 'integer_bits': 1, 'fraction_bits': 8},
            'definition': {'field': 10, 'version_bits': 8, 'id_bits': 12, 'slots': 3,
                           'initial_populated_version': 2},
            'equipment': {'primary_field': 11, 'secondary_field': 12,
                          'version_bits': 8, 'gear_bits': 5, 'pool_bits': 2,
                          'count_bits': 8, 'reloading_bits': 1},
            'overheat': {'field': 21, 'bits': 8},
            'primary': {'gear': 0, 'pool': 0, 'hit_mode': 0},
            'item_members_sha256': item_members, 'item_catalog_sha256': sha(catalog_raw),
            'item_bindings_sha256': sha(binding_raw), 'equipment_by_faction': equipment,
            'native': native['wire']}
    rules = {'schema': 'mag_domination_hospital_turrets_v1',
             'source': {'structure_sha256': structure_hash, 'item_catalog_sha256': sha(catalog_raw),
                        'retail_elf_sha256': RETAIL_ELF_SHA256, 'retail_archive_sha256': GAME_ARCHIVE_SHA256,
                        'seat_pose_sha256': sha(encode(pose)), 'wire_proof_sha256': sha(encode(wire)),
                        'vehicle_type_token': vehicle_type},
             'variants': variants, 'profiles': profiles}
    proof = {'format': 'mag_retail_domination_hospital_turret_proof_v1',
             'hospital_sources_sha256': sha(sources_raw), 'structure_sha256': structure_hash,
             'pose': pose, 'wire': wire, 'profiles': profile_proof}
    return rules, proof


def output_paths(inputs, paths):
    sources = {path.resolve() for path in inputs}
    targets = {path.resolve() for path in paths}
    require(len(sources) == len(inputs) and len(targets) == len(paths) and not targets.intersection(sources),
            'hospital turret output and input paths must be distinct')
    require(all(not target.is_relative_to(ROOT) for target in targets),
            'hospital turret outputs must be outside the public checkout')


def write_outputs(inputs, outputs, *, check):
    output_paths(inputs, [path for path, _ in outputs])
    if check:
        for path, data in outputs:
            require(path.read_bytes() == data, f'hospital turret output differs at {path}')
        return
    require(all(not path.exists() and not path.is_symlink() for path, _ in outputs),
            'hospital turret output already exists')
    for path, data in outputs:
        with path.open('xb') as stream:
            stream.write(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('elf', 'game-archive', 'main-archive', 'hospital-sources',
                 'item-catalog', 'item-bindings', 'structures', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--profile', metavar=('SPAWNS', 'HOSPITALS'), type=Path, nargs=2,
                        action='append', required=True, help='repeat once per Domination profile')
    parser.add_argument('--proof-output', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [args.elf, args.game_archive, args.main_archive, args.hospital_sources,
              args.item_catalog, args.item_bindings, args.structures,
              *(path for pair in args.profile for path in pair)]
    outputs = [args.output] + ([args.proof_output] if args.proof_output else [])
    output_paths(inputs, outputs)
    require(len(args.profile) == 6, 'expected six Domination profile pairs')
    elf = NATIVE['Elf'](args.elf, 'retail')
    game_raw, main_raw = args.game_archive.read_bytes(), args.main_archive.read_bytes()
    require(sha(game_raw) == GAME_ARCHIVE_SHA256 and sha(main_raw) == MAIN_ARCHIVE_SHA256,
            'wrong original retail game or main archive')
    game, main_archive = map(NATIVE['Psarc'], (game_raw, main_raw))
    rules, proof = project(
        args.hospital_sources.read_bytes(), args.item_catalog.read_bytes(), args.item_bindings.read_bytes(),
        args.structures.read_bytes(), [(a.read_bytes(), b.read_bytes()) for a, b in args.profile],
        lambda name: archive_member(game, name), lambda name: archive_member(main_archive, name),
        read_model, native_witness(elf))
    rendered = [(args.output, encode(rules))]
    if args.proof_output:
        rendered.append((args.proof_output, encode(proof)))
    write_outputs(inputs, rendered, check=args.check)
    print('Verified 6 retail Domination profiles, 96 hospital placements and 3 turret variants')


if __name__ == '__main__':
    main()
