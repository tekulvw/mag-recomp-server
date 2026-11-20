#!/usr/bin/env python3
"""Project proved initial client/Sim trigger vector indices and authored boxes."""
import argparse
import json
from pathlib import Path
import runpy

TOOLS = Path(__file__).resolve().parent
COMMON = runpy.run_path(str(TOOLS / 'project-retail-acquisition-initial-objectives.py'))
RETAIL_SHA = COMMON['RETAIL_SHA']
require, sha, encode, integer, token = (
    COMMON[name] for name in ('require', 'sha', 'encode', 'integer', 'token')
)
MAPS = {'mag_baku_final': 95, 'mag_canal_final': 108, 'mag_unalaska_final': 96}
STATUS = 'ORIGINAL_RETAIL_CLIENT_AND_SIM_INITIAL_TRIGGER_VECTOR_ORDER_BOUND'


def unique(rows, index):
    result = {index(row): row for row in rows}
    require(len(result) == len(rows), 'Duplicate source row identity')
    return result


def project(metadata_raw, trigger_raw, objective_raw):
    require(len(metadata_raw) <= 32 * 1024**2 and len(trigger_raw) <= 8 * 1024**2
            and len(objective_raw) <= 1_048_576, 'Input exceeds bound')
    metadata, proof, catalog = map(json.loads, (metadata_raw, trigger_raw, objective_raw))
    require(metadata['format'] == 'mag_retail_domination_map_metadata'
            and metadata['schema_version'] == 1 and metadata['mode_enum'] == 'GameType256'
            and catalog['format'] == 'mag_retail_objective_catalog' and catalog['schema_version'] == 1
            and catalog['game_type'] == 2 and catalog['entity_family'] == catalog['network_type'] == 12,
            'Wrong Domination source types')
    source = catalog['source']
    require(proof['schema'] == 'mag_domination_initial_trigger_bindings_v1'
            and proof['status'] == STATUS
            and proof['original_elf_sha256']['retail'] == source['source_image_sha256'] == RETAIL_SHA
            and proof['authored_metadata_sha256'] == source['authored_catalog_sha256'] == sha(metadata_raw)
            and proof['objective_binding_sha256'] == source['binding_capture_sha256']
            and source['capture_qualification'] == 'native_initial_load'
            and source['initial_load_scope'] == 'pre_mission_start_before_dynamic_creation',
            'Unqualified trigger or objective bindings')
    authored_maps = unique(metadata['maps'], lambda row: row['map']['map'])
    bound_maps = unique(proof['maps'], lambda row: row['map']['map'])
    name = catalog['map_name']
    require(set(authored_maps) == set(bound_maps) == MAPS.keys() and name in MAPS,
            'Incomplete original Domination proof maps')
    authored, qualified = authored_maps[name], bound_maps[name]
    require(authored['map'] == qualified['map'] and authored['source'] == qualified['source'],
            'Map/level source mismatch')
    volumes = unique([trigger for objective in authored['objective_trigger_volumes']
                      for trigger in objective['triggers']], lambda row: token(row['trigger_token']))
    bindings = unique(qualified['triggers'], lambda row: token(row['trigger_token']))
    objectives = unique(catalog['objects'], lambda row: token(row['token']))
    require(len(volumes) == MAPS[name] and volumes.keys() == bindings.keys(), 'Incomplete trigger inventory')
    indices = set()
    output = []
    for trigger_token, volume in volumes.items():
        row = bindings[trigger_token]
        objective_token = token(row['objective_token'])
        object_index = integer(row['objective_initial_client_network_index'], 127)
        index = integer(row['client_trigger_index'], 254)
        require(row['authored_volume'] == volume and objective_token == token(volume['objective_token'])
                and objectives[objective_token]['object_index'] == object_index,
                'Trigger volume/objective source mismatch')
        require(row['sim_trigger_index'] == index and row['family56_level_object_slot'] is None
                and (object_index, index) not in indices, 'Conflicting native trigger vector index')
        indices.add((object_index, index))
        output.append(dict(object_index=object_index, trigger_index=index, authored_volume=volume))
    indexed = sorted(output, key=lambda row: (row['object_index'], row['trigger_index']))
    seen = {}
    for row in indexed:
        count = seen.get(row['object_index'], 0)
        require(row['trigger_index'] == count, 'Incomplete per-objective trigger vector')
        seen[row['object_index']] = count + 1
    return dict(
        format='mag_domination_trigger_catalog_v1', schema_version=1, map_name=name,
        source=dict(metadata_sha256=sha(metadata_raw), objective_catalog_sha256=sha(objective_raw),
                    trigger_binding_sha256=sha(trigger_raw), native_elf_sha256=RETAIL_SHA,
                    scope='successful_initial_client_and_sim_level_loaded_registration'),
        triggers=indexed,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('metadata', 'bindings', 'objective-catalog', 'native-elf', 'output'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        require(not output.is_relative_to(TOOLS.parents[1]) and output not in {
            path.resolve() for path in (args.metadata, args.bindings, args.objective_catalog, args.native_elf)
        }, 'Output must be outside the source checkout and inputs')
        require(sha(args.native_elf.read_bytes()) == RETAIL_SHA, 'Wrong original retail ELF')
        raw = encode(project(args.metadata.read_bytes(), args.bindings.read_bytes(), args.objective_catalog.read_bytes()))
        if args.check:
            require(args.output.read_bytes() == raw, 'Trigger sidecar differs from reproduction')
        else:
            with output.open('xb') as file:
                file.write(raw)
    except (KeyError, OSError, ValueError, TypeError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
