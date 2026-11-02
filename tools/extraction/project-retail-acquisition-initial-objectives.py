#!/usr/bin/env python3
"""Project externally qualified initial client-objective bindings, never ordinals.

The evidence concerns fresh client entities AND freshly constructed NetObjects,
before MissionStart/wire/dynamic mutation. It is not post-script startup or live
occupancy. Hash/structure validation does not independently certify native code.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re

RETAIL_SHA = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    return (json.dumps(value, indent=2, allow_nan=False)+'\n').encode()


def digest(value):
    require(isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None, 'Invalid SHA256')
    return value


def integer(value, maximum):
    require(type(value) is int and 0 <= value <= maximum, 'Invalid unsigned integer')
    return value


def token(value):
    if isinstance(value, str):
        require(re.fullmatch('0x[0-9a-fA-F]{1,8}', value) is not None, 'Invalid token')
        value = int(value, 16)
    require(integer(value, 0xffffffff) != 0, 'Null token')
    return value


def project(spawn_bytes, proof_bytes):
    require(len(spawn_bytes) <= 1_048_576 and len(proof_bytes) <= 4_194_304, 'Input exceeds bound')
    spawn, proof = json.loads(spawn_bytes), json.loads(proof_bytes)
    require(spawn['format'] == 'mag_spawn_catalog' and spawn['schema_version'] == 1
            and spawn['match_profile']['game_type'] == 1 and len(spawn['maps']) == 1, 'Require Acquisition spawns')
    require(proof['schema'] == 'mag_acquisition_objective_initial_client_bindings_v1'
            and proof['status'] == 'ORIGINAL_CLIENT_FACTORY_AND_INITIAL_SUCCESSFUL_LOAD_TOKEN_SLOT_NET12_MODEL_BOUND',
            'Unqualified client binding evidence')
    require(proof['source_model_binding_count'] == sum(len(m['objectives']) for m in proof['maps']),
            'Proof inventory count mismatch')
    name = spawn['maps'][0]['name']
    source = json.loads(spawn['source'])
    require(source['map'] == spawn['active']['map_name'] == name, 'Spawn map mismatch')
    maps = [m for m in proof['maps'] if m['map']['map'] == name]
    require(len(maps) == 1 and maps[0]['map']['mode_enum'] == 'GameType128', 'Require one Acquisition proof map')
    selected = maps[0]
    require(source['level_archive_sha256'] == digest(selected['source']['archive_sha256'])
            and source['level_clv_sha256'] == digest(selected['source']['clv_sha256']), 'Level hash mismatch')
    pool = selected['native_initial_client_pool']
    require(pool['array'] == 12 and pool['capacity'] == 128 and pool['initial_load_success_scope'] is True
            and pool['runtime_observed'] is False, 'Unqualified initial client pool')
    objects, by_token, slots = [], {}, set()
    for row in selected['objectives']:
        # Candidate ordinals and unresolved top-level runtime fields are never used.
        binding = row['native_initial_client_binding']
        name_token = token(row['name_token'])
        index = integer(binding['client_pool_slot'], 127)
        require(binding['entity_family'] == binding['network_type'] == 12
                and binding['network_index'] == index
                and token(binding['packed_client_entity_id']) == (0x0c800000 | index)
                and binding['source_model_only'] is True and binding['runtime_observed'] is False
                and type(binding['initial_network_state']) is int and binding['initial_network_state'] == 0,
                'Inconsistent client/network/state binding')
        require(name_token not in by_token and index not in slots, 'Duplicate objective binding')
        by_token[name_token] = index; slots.add(index)
        objects.append(dict(token=f'0x{name_token:08x}', object_index=index,
                            initial_state=binding['initial_network_state']))
    require(0 < len(objects) <= 128 and sorted(slots) == sorted(pool['expected_occupied_slots'])
            and len(pool['expected_occupied_slots']) == len(slots), 'Incomplete initial pool inventory')
    squads, addresses = [], set()
    require(len(selected['squads']) == 16, 'Require sixteen Acquisition squad lists')
    for squad in selected['squads']:
        team, platoon, slot = (integer(squad[k], maximum) for k, maximum in
                               [('team', 1), ('platoon_id', 1), ('squad', 3)])
        index = team*16 + platoon*4 + slot
        require(index == squad['squad_index'] and index not in addresses, 'Invalid squad address')
        addresses.add(index)
        entries = squad['ordered_source_entries']
        require(0 < len(entries) <= 128, 'Invalid squad objective list')
        authored = []
        for entry in entries:
            name_token = token(entry['objective_token'])
            require(name_token in by_token, 'Squad references an unbound objective')
            authored.append(dict(object_index=by_token[name_token], order=integer(entry['authored_order'], 0xffffffff)))
        # All qualified constructor states are inactive. Do not promote desired
        # state or authored Active into a current state/HUD target.
        squads.append(dict(team=team, squad_index=index, initial_objective_index=65535,
                           authored_objectives=authored))
    objects.sort(key=lambda row: row['object_index']); squads.sort(key=lambda row: row['squad_index'])
    return dict(format='mag_retail_objective_catalog', schema_version=1, application_id=20231, protocol=0x6cc,
        game_type=1, map_name=name, entity_family=12, network_type=12,
        source=dict(authored_catalog_sha256=digest(proof['catalog_sha256']),
                    binding_capture_sha256=sha(proof_bytes), startup_proof_sha256=sha(proof_bytes),
                    source_image_sha256=digest(proof['original_elf_sha256']['retail']),
                    capture_qualification='native_initial_load',
                    initial_load_scope='pre_mission_start_before_dynamic_creation'),
        objects=objects, squads=squads)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('spawn-catalog', 'bindings', 'native-elf', 'output'):
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    require(args.output.resolve() not in {p.resolve() for p in (args.spawn_catalog, args.bindings, args.native_elf)},
            'Output must not overwrite an input')
    try:
        elf_sha = sha(args.native_elf.read_bytes())
        require(elf_sha == RETAIL_SHA, 'Wrong original retail ELF')
        proof_bytes = args.bindings.read_bytes()
        require(json.loads(proof_bytes)['original_elf_sha256']['retail'] == elf_sha, 'Binding ELF mismatch')
        raw = encode(project(args.spawn_catalog.read_bytes(), proof_bytes))
        if args.check:
            require(args.output.read_bytes() == raw, 'Initial objectives differ from reproduction')
        else:
            args.output.write_bytes(raw)
    except (KeyError, TypeError, ValueError) as error:
        parser.exit(1, str(error)+'\n')


if __name__ == '__main__':
    main()
