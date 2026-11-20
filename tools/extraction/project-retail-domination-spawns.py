#!/usr/bin/env python3
"""Install source-model fresh retail Domination SpawnPoint IDs in a private profile.

The projection moves indices only. Spawn eligibility, positions and authored
active state still come from the existing profile.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import runpy

TOOLS = Path(__file__).resolve().parent
COMMON = runpy.run_path(str(TOOLS / 'project-retail-acquisition-initial-objectives.py'))
BUILDER = runpy.run_path(str(TOOLS / 'build-retail-spawn-catalog.py'))
RETAIL_SHA = COMMON['RETAIL_SHA']
require, sha, encode, digest, integer, token = (
    COMMON[name] for name in ('require', 'sha', 'encode', 'digest', 'integer', 'token')
)
STATUS = 'ORIGINAL_RETAIL_SPAWN_FAMILY_35_INITIAL_SUCCESSFUL_LOAD_MODEL'
FACTORY_STATUS = 'ORIGINAL_NATIVE_FACTORY_FAMILY_AND_INITIAL_LOAD_ALLOCATION_MODEL_BOUND'
MAPS = {'mag_baku_final', 'mag_canal_final', 'mag_unalaska_final'}
COUNT = 112


def unique(rows, field):
    result = {row[field]: row for row in rows}
    require(len(result) == len(rows), f'Duplicate {field}')
    return result


def check_source(catalog, proof, factory_bytes):
    factory = json.loads(factory_bytes)
    require(catalog['format'] == 'mag_spawn_catalog' and catalog['schema_version'] == 1
            and catalog['match_profile']['game_type'] == 2 and len(catalog['maps']) == 1,
            'Require an explicit Domination spawn profile')
    require(proof['schema'] == 'mag_domination_spawn_native_initial_bindings_v1'
            and proof['status'] == STATUS and proof['native_elf_sha256'] == RETAIL_SHA
            and proof['native_factory_proof_sha256'] == sha(factory_bytes)
            and factory['schema'] == 'mag_acquisition_spawn_native_initial_bindings_v1'
            and factory['status'] == FACTORY_STATUS
            and factory['original_elf_sha256']['retail'] == RETAIL_SHA,
            'Unqualified initial spawn proof')
    pool, layout = proof['pool'], factory['native_factory']
    require(pool['runtime_observed'] is False
            and (integer(pool['family'], 35), integer(pool['capacity'], 128), integer(pool['stride'], 224))
            == (35, 128, 224)
            and (layout['object_type'], layout['capacity'], layout['spawn_stride']) == (35, 128, 224)
            and layout['property_class'] == 'SpawnPointProperties'
            and layout['automatic_creation_id'] == '2380ffff',
            'Unexpected native SpawnPoint pool')
    for field in ('checker_sha256', 'topology_checker_sha256', 'spawn_extractor_sha256'):
        digest(proof[field])
    world = catalog['maps'][0]
    source = json.loads(catalog['source'])
    name = world['name']
    require(name in MAPS and name == source['map'] == catalog['active']['map_name']
            and world['runtime_indices_verified'] is False and 'runtime_indices_basis' not in world,
            'Require the original authored-index Domination profile')
    map_proofs = {row['map']['map']: row for row in proof['maps']}
    require(len(proof['maps']) == len(MAPS) and set(map_proofs) == MAPS, 'Incomplete Domination proof maps')
    selected = map_proofs[name]
    require(selected['map']['mode_enum'] == 'GameType256'
            and BUILDER['FACTIONS'][selected['map']['owner_faction']] == world['owner_faction']
            and source['game_archive_sha256'] == proof['game_archive_sha256'] == BUILDER['GAME_ARCHIVE_SHA256']
            and source['game_rules_source']['sha256'] == proof['settings_sha256'] == BUILDER['SETTINGS_SHA256']
            and source['level_archive_sha256'] == digest(selected['source']['archive_sha256'])
            and source['level_clv_sha256'] == digest(selected['source']['clv_sha256']),
            'Original game or level source mismatch')
    return world, source, selected


def index_points(world, selected):
    points = selected['points']
    locations = unique(world['locations'], 'index')
    require(world['spawn_count'] == len(points) == len(locations) == COUNT
            and set(locations) == set(range(COUNT)), 'Incomplete spawn inventory')
    by_token = {token(row['name_token']): row for row in points}
    require(len(by_token) == COUNT and set(by_token) == {token(row['token']) for row in locations.values()},
            'Spawn token inventory mismatch')
    remap, points_by_index = {}, {}
    for index, row in locations.items():
        point = by_token[token(row['token'])]
        slot = integer(point['initial_client_pool_slot'], COUNT - 1)
        spawn_type = integer(point['spawn_type'], 255)
        user_class = integer(point['user_class'], 1)
        category = integer(point['category'], 255)
        integer(point['team'], 1)
        expected = BUILDER['KINDS'].get(category) if spawn_type == user_class == 0 else None
        require(point['authored_catalog_id'] == f"{world['name']}:{index:03d}"
                and point['authored_object_id'] == 0xffffffff
                and token(point['initial_client_object_id']) == (0x23800000 | slot)
                and row['active'] is point['active'] and type(point['active']) is bool
                and row['kind'] == (expected or 'unsupported'),
                'Spawn identity or authored policy mismatch')
        points_by_index[index] = point
        remap[index] = slot
    require(set(remap.values()) == set(range(COUNT)), 'Duplicate or missing native pool slots')
    for field, key in (('spawn_types', 'spawn_type'), ('user_classes', 'user_class')):
        require(selected[field] == dict(Counter(str(point[key]) for point in points)),
                f'Native {field} inventory mismatch')
    return locations, remap, points_by_index


def check_squad_sources(world, source, locations, points, active):
    squads = unique(world['squads'], 'index')
    require(set(squads) == set(range(32)) and not world.get('forward_spawns'),
            'Expected 32 authored squad links and no forward-spawn rules')
    members = {index: {} for index in squads}
    unsupported = []
    for index, point in points.items():
        links = []
        for link in point['squad_links']:
            team = integer(link['team'], 1)
            platoon = integer(link['platoon_id'], 3)
            squad = BUILDER['SQUADS'][link['squad']]
            slot = integer(link['slot'], 7)
            address = 16 * team + 4 * platoon + squad
            require(team == point['team'] and token(link['point_token']) == token(point['name_token'])
                    and slot not in members[address], 'Invalid native squad membership')
            members[address][slot] = index
            links.append([team, platoon, squad, slot])
        if locations[index]['kind'] == 'unsupported':
            unsupported.append(dict(index=index, spawn_type=point['spawn_type'], category=point['category'],
                                    user_class=point['user_class'], active=point['active'], squad_links=links))
    require(sorted(unsupported, key=lambda row: row['index']) == sorted(
        source['unsupported_spawn_links'], key=lambda row: row['index']), 'Unsupported spawn provenance mismatch')

    fallbacks = unique(source['default_spawn_fallbacks'], 'squad_index')
    expected_fallbacks = set()
    for address, squad in squads.items():
        team, platoon, number = address // 16, address % 16 // 4, address % 4
        require((squad['team'], squad['platoon'], squad['squad']) == (team, platoon, number)
                and squad['faction'] == (world['owner_faction'] if team else None),
                'Squad identity mismatch')
        ordered = members[address]
        require(set(ordered) == set(range(len(ordered))), 'Noncontiguous authored squad slots')
        available = [ordered[slot] for slot in sorted(ordered)
                     if locations[ordered[slot]]['active'] and locations[ordered[slot]]['kind'] != 'unsupported']
        require(available and available == squad['available'], 'Squad available list differs from authored links')
        defaults = [index for index in available if points[index]['category'] == 0]
        if not defaults:
            require(team == 0 and 0 in ordered and 1 in ordered
                    and points[ordered[0]]['spawn_type'] == 3 and points[ordered[0]]['category'] == 0
                    and points[ordered[0]]['user_class'] == 0 and points[ordered[0]]['active'] is True
                    and points[ordered[1]]['category'] == 1 and ordered[1] in available,
                    'Unqualified APC insertion substitute')
            defaults = [ordered[1]]
            require(fallbacks[address]['default_spawn'] == defaults[0], 'Wrong APC fallback point')
            expected_fallbacks.add(address)
        require(defaults == [squad['default_spawn']], 'Squad default differs from authored policy')
    require(set(fallbacks) == expected_fallbacks, 'Unexpected APC fallback provenance')
    selected = squads[active['squad_index']]
    require(all(active[field] == selected[field] for field in ('available', 'default_spawn')),
            'Active squad projection mismatch')


def project(spawn_bytes, proof_bytes, factory_bytes):
    require(len(spawn_bytes) <= 1_048_576 and len(proof_bytes) <= 4 * 1024**2
            and len(factory_bytes) <= 16 * 1024**2, 'Input exceeds bound')
    catalog, proof = map(json.loads, (spawn_bytes, proof_bytes))
    world, source, selected = check_source(catalog, proof, factory_bytes)
    locations, remap, points = index_points(world, selected)
    active = catalog['active']
    check_squad_sources(world, source, locations, points, active)

    for index, location in locations.items():
        location['index'] = remap[index]
    world['locations'].sort(key=lambda row: row['index'])
    for squad in [active, *world['squads']]:
        squad['available'] = [remap[index] for index in squad['available']]
        squad['default_spawn'] = remap[squad['default_spawn']]
    for row in source['unsupported_spawn_links']:
        row['index'] = remap[row['index']]
    for row in source['default_spawn_fallbacks']:
        row['default_spawn'] = remap[row['default_spawn']]
    source.update(note='Authored pre-terrain-snap positions; fresh client family-35 load model',
                  authored_spawn_catalog_sha256=sha(spawn_bytes), native_spawn_bindings_sha256=sha(proof_bytes))
    catalog['source'] = json.dumps(source, sort_keys=True, separators=(',', ':'))
    require(len(catalog['source'].encode()) <= 8192, 'Runtime spawn source exceeds bound')
    world['runtime_indices_verified'] = True
    world['runtime_indices_basis'] = 'Original retail fresh successful compiled client load pass 2, family 35'
    return catalog


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('spawn-catalog', 'bindings', 'factory-proof', 'native-elf', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        require(not output.is_relative_to(TOOLS.parents[1]) and output not in {
            path.resolve() for path in (args.spawn_catalog, args.bindings, args.factory_proof, args.native_elf)
        }, 'Output must be outside the source checkout and inputs')
        with args.native_elf.open('rb') as image:
            require(hashlib.file_digest(image, 'sha256').hexdigest() == RETAIL_SHA, 'Wrong original retail ELF')
        raw = encode(project(args.spawn_catalog.read_bytes(), args.bindings.read_bytes(), args.factory_proof.read_bytes()))
        if args.check:
            require(args.output.read_bytes() == raw, 'Spawn profiles differ from reproduction')
        else:
            with args.output.open('xb') as stream:
                stream.write(raw)
    except (KeyError, OSError, TypeError, ValueError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
