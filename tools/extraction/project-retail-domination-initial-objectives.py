#!/usr/bin/env python3
"""Project qualified original Domination client bindings before MissionStart.

The private receipt models successful fresh client loading. This command checks
source joins and inventories and preserves the qualified slots in that receipt.
"""
import argparse
import json
from pathlib import Path
import runpy

COMMON = runpy.run_path(str(Path(__file__).with_name('project-retail-acquisition-initial-objectives.py')))
RETAIL_SHA = COMMON['RETAIL_SHA']
require, sha, encode, digest, integer, token = (
    COMMON[name] for name in ('require', 'sha', 'encode', 'digest', 'integer', 'token')
)
CLASSES = {
    'ObjectiveDemolitionStructuralProperties': 'ClientObjectiveDemolitionStructural',
    'ObjectiveSecureAndHoldProperties': 'ClientObjectiveSecureAndHold',
}
TRIGGER_CLASS = 'ObjectiveTriggerProperties'
MAPS = {'mag_baku_final', 'mag_canal_final', 'mag_unalaska_final'}
STATUS = 'ORIGINAL_CLIENT_FACTORY_AND_INITIAL_SUCCESSFUL_LOAD_TOKEN_SLOT_NET12_MODEL_BOUND'


def unique(rows, field):
    result = {row[field]: row for row in rows}
    require(len(result) == len(rows), f'Duplicate {field}')
    return result


def project(metadata_bytes, proof_bytes, native_replay_bytes, map_name):
    require(len(metadata_bytes) <= 32 * 1024**2 and len(proof_bytes) <= 4 * 1024**2
            and len(native_replay_bytes) <= 16 * 1024**2, 'Input exceeds bound')
    metadata, proof, native_replay = map(json.loads, (metadata_bytes, proof_bytes, native_replay_bytes))
    require(metadata['format'] == 'mag_retail_domination_map_metadata'
            and metadata['schema_version'] == 1 and metadata['mode_enum'] == 'GameType256',
            'Require retail Domination metadata')
    require(proof['schema'] == 'mag_domination_objective_initial_client_bindings_v1'
            and proof['status'] == native_replay['status'] == STATUS
            and proof['catalog_sha256'] == sha(metadata_bytes)
            and proof['native_proof_replay_sha256'] == sha(native_replay_bytes)
            and proof['original_elf_sha256'] == native_replay['original_elf_sha256']
            and proof['original_elf_sha256']['retail'] == RETAIL_SHA, 'Unqualified binding evidence')
    require(proof['source_model_binding_count'] == sum(len(row['objects']) for row in proof['maps']),
            'Proof inventory count mismatch')
    digest(proof['checker_sha256'])
    metadata_maps = {row['map']['map']: row for row in metadata['maps']}
    proof_maps = {row['map']['map']: row for row in proof['maps']}
    require(set(metadata_maps) == set(proof_maps) == MAPS and map_name in MAPS
            and len(metadata['maps']) == len(proof['maps']) == len(MAPS),
            'Require the three original Domination maps')
    authored, selected = metadata_maps[map_name], proof_maps[map_name]
    require(authored['map'] == selected['map'] and selected['map']['mode_enum'] == 'GameType256'
            and selected['source'] == authored['source'], 'Level source mismatch')
    pool = selected['native_initial_client_pool']
    require(pool['array'] == 12 and pool['capacity'] == 128 and pool['stride'] == 1872
            and pool['initial_load_success_scope'] is True and pool['runtime_observed'] is False,
            'Unqualified initial client pool')

    definitions = unique(authored['objectives']['definitions'], 'definition_token')
    instances = unique(authored['objectives']['instances'], 'name_token')
    classes = {name: definitions[row['definition_token']]['class_name'] for name, row in instances.items()}
    eligible = {name for name, cls in classes.items() if cls != TRIGGER_CLASS}
    require(all(classes[name] in CLASSES for name in eligible)
            and len(instances) - len(eligible) == selected['excluded_trigger_count'],
            'Objective class inventory mismatch')
    qualified = unique(selected['objects'], 'name_token')
    require(0 < len(qualified) <= 128 and set(qualified) == eligible, 'Incomplete family 12 inventory')
    objects, by_token, slots = [], {}, set()
    for name, row in qualified.items():
        cls, instance = classes[name], instances[name]
        require(row['class_name'] == cls and row['instance_source'] == instance['source']
                and row['definition_token'] == instance['definition_token'], 'Objective instance source mismatch')
        name_token = token(name)
        binding = row['native_initial_client_binding']
        index = integer(binding['client_pool_slot'], 127)
        require(binding['entity_family'] == binding['network_type'] == 12
                and binding['network_index'] == index
                and token(binding['packed_client_entity_id']) == (0x0c800000 | index)
                and binding['client_class'] == CLASSES[cls]
                and binding['source_model_only'] is True and binding['runtime_observed'] is False
                and integer(binding['initial_network_state'], 0) == 0, 'Inconsistent native objective binding')
        require(name_token not in by_token and index not in slots, 'Duplicate objective binding')
        by_token[name_token] = index
        slots.add(index)
        objects.append(dict(token=f'0x{name_token:08x}', object_index=index, initial_state=0))
    require(sorted(slots) == pool['expected_occupied_slots'] == list(range(len(eligible))),
            'Incomplete initial slot occupancy')

    squads = []
    addresses = {(row['team'], row['platoon'], row['squad']) for row in authored['squads']}
    require(len(addresses) == len(authored['squads']) == len(selected['squads']) == 32,
            'Require 32 retail squad links and lists')
    for squad in selected['squads']:
        team = integer(squad['team'], 1)
        platoon = integer(squad['platoon_id'], 3)
        slot = integer(squad['squad'], 3)
        index = 16 * team + 4 * platoon + slot
        require(index == squad['squad_index'] and (team, platoon, slot) in addresses,
                'Invalid squad address')
        addresses.remove((team, platoon, slot))
        entries = squad['ordered_source_entries']
        require(0 < len(entries) <= 128, 'Invalid squad objective list')
        objectives = [
            dict(object_index=by_token[token(entry['objective_token'])],
                 order=integer(entry['authored_order'], 0xffffffff))
            for entry in entries
        ]
        squads.append(dict(team=team, squad_index=index, initial_objective_index=65535,
                           authored_objectives=objectives))

    return dict(
        format='mag_retail_objective_catalog', schema_version=1, application_id=20231, protocol=0x6cc,
        game_type=2, map_name=map_name, entity_family=12, network_type=12,
        source=dict(authored_catalog_sha256=sha(metadata_bytes), binding_capture_sha256=sha(proof_bytes),
                    startup_proof_sha256=sha(proof_bytes), source_image_sha256=RETAIL_SHA,
                    capture_qualification='native_initial_load',
                    initial_load_scope='pre_mission_start_before_dynamic_creation'),
        objects=sorted(objects, key=lambda row: row['object_index']),
        squads=sorted(squads, key=lambda row: row['squad_index']),
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('metadata', 'bindings', 'native-replay', 'native-elf', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--map', choices=sorted(MAPS), required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        require(not output.is_relative_to(Path(__file__).resolve().parents[2]),
                'Output must be outside the source checkout')
        require(output not in {
            path.resolve() for path in (args.metadata, args.bindings, args.native_replay, args.native_elf)
        }, 'Output must not overwrite an input')
        require(sha(args.native_elf.read_bytes()) == RETAIL_SHA, 'Wrong original retail ELF')
        raw = encode(project(args.metadata.read_bytes(), args.bindings.read_bytes(),
                             args.native_replay.read_bytes(), args.map))
        if args.check:
            require(args.output.read_bytes() == raw, 'Initial objectives differ from reproduction')
        else:
            with args.output.open('xb') as stream:
                stream.write(raw)
    except (KeyError, OSError, TypeError, ValueError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
