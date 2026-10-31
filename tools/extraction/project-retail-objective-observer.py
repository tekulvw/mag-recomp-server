#!/usr/bin/env python3
"""Project the bounded objective_v1 passive observer configuration.

Reuses the qualified manager root and caps of the maintained spawn observer.
Does not launch an observer, touch guest memory, or activate a runtime catalog.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def load(path):
    raw = path.read_bytes()
    require(len(raw) <= 4 * 1024 * 1024, 'input exceeds bounded JSON size')
    return json.loads(raw), sha(raw)


def token(value):
    require(isinstance(value, str) and re.fullmatch(r'0x[0-9a-fA-F]{8}', value), 'invalid hex32 token')
    return value.lower()


def fields(rows):
    require(len({row['name'] for row in rows}) == len(rows), 'duplicate authored field')
    return {row['name']: row.get('decoded_value', row['value']) for row in rows}


def authored_entities(graph):
    require(graph['schema_version'] == 1 and graph['format'] == 'mag_retail_sabotage_objective_metadata'
            and graph['application_id'] == 20231 and graph['map']['mode'] == 'Sabotage', 'wrong authored catalog identity')
    definitions = {token(row['definition_token']): row for row in graph['objectives']['definitions']}
    require(len(definitions) == len(graph['objectives']['definitions']), 'duplicate objective definition')
    controls = graph['control_argument_bindings']
    require(len(controls) == 1 and controls[0]['function'] == 'ObjectiveControl', 'ambiguous objective control')
    roles = {token(row['Object']): row['parameter'] for row in controls[0]['bindings']}
    require(len(roles) == len(controls[0]['bindings']) == 10, 'ambiguous objective roles')
    objectives, triggers, seen = [], [], set()
    for row in graph['objectives']['instances']:
        name = token(row['name_token'])
        require(name not in seen, 'duplicate authored instance token')
        seen.add(name)
        definition_token = token(row['definition_token'])
        definition = definitions[definition_token]
        if definition['class_name'] == 'ObjectiveTriggerProperties':
            triggers.append(name)
            continue
        require(definition['class_name'] in ('ObjectiveSecureAndHoldProperties', 'ObjectiveDemolitionStructuralProperties'),
                'unqualified objective definition class')
        require(name in roles, 'missing objective control role')
        raw = bytes.fromhex(definition['fields']['InitialActiveState']['raw_hex'])
        require(len(raw) == 13 and raw[:8] == bytes.fromhex('deadbeef00000001') and raw[8:12] == bytes(4), 'invalid authored initial enum')
        initial = int.from_bytes(raw[-4:], 'big')
        require(initial in (0, 1, 2, 3), 'unknown authored initial state')
        active = definition['fields']['Active']['value']
        require(type(active) is bool, 'invalid authored Active gate')
        objectives.append({'raw_token_expected': name, 'definition_token': definition_token,
                           'authored_initial_state': initial, 'authored_active': active, 'role': roles[name]})
    require(len(objectives) == 10 and len(triggers) == 3 and set(roles) == {r['raw_token_expected'] for r in objectives},
            'expected ten objective entities and three excluded triggers')
    return objectives, sorted(triggers)


def emit(args, value, inputs):
    output = args.output
    require(output.resolve() not in {p.resolve() for p in inputs}, 'output must not replace an input')
    require(args.check or not output.exists(), 'refusing existing output; use --check')
    raw = (json.dumps(value, indent=2, sort_keys=True) + '\n').encode()
    if args.check:
        require(output.read_bytes() == raw, 'projection reproduction mismatch')
    else:
        with output.open('xb') as stream:
            stream.write(raw)
    print(json.dumps({'output': str(output), 'sha256': sha(raw)}))


def produce(args):
    graph, graph_sha = load(args.catalog)
    spawn, spawn_sha = load(args.spawn_observer)
    pool, pool_sha = load(args.pool_proof)
    binding, binding_sha = load(args.binding_proof)
    startup, startup_sha = load(args.startup_proof)
    require(spawn['format'] == 'mag_passive_spawn_observer' and spawn['schema_version'] == 1
            and spawn.get('observation_profile', 'spawn_v1') == 'spawn_v1', 'unqualified manager observer source')
    original = pool['original_sha256']
    require(original == binding['original_sha256'] == startup['original_sha256'] == spawn['source']['original_elf_sha256'],
            'proof original-image identity mismatch')
    require(graph['map']['map'] == spawn['source']['map'], 'observer map mismatch')
    require(graph['source']['clv_sha256'] == spawn['source']['map_clv_sha256']
            and graph['source']['level_archive_sha256'] == spawn['source']['map_archive_sha256'], 'observer map source mismatch')
    objectives, triggers = authored_entities(graph)
    layout = copy.deepcopy(spawn['layout'])
    flow = pool['dataflow']
    require(flow['array_index'] == flow['configured_id_family'] == 12, 'objective family proof mismatch')
    layout['family']['type'] = flow['configured_id_family']
    layout['family']['object_id_prefix'] = int(flow['fresh_objective_id_prefix'], 16)
    layout['family']['record_stride'] = flow['array_entry_bytes']
    layout['object_fields'] = {
        'raw_token_candidate': {'offset': int(flow['entity_raw_token_offset'], 16), 'width': 4,
                                'authority': 'original entity-pool creation-info copy proof'},
        'object_id': {'offset': int(flow['entity_id_offset'], 16), 'width': 4,
                      'authority': 'original entity-pool ID copy and ClientObjective getter proof'},
        'net_object_pointer': {'offset': int(binding['identity']['net_object_member'], 16), 'width': 4,
                               'authority': 'original ClientObjective initialization binding proof'},
    }
    policy = copy.deepcopy(spawn['diagnostic_policy'])
    policy['object_prefix_bytes'] = 204
    policy['required_results'] = [
        'raw manager/family headers and complete bitmap before/after',
        'all allocated rows, including inactive and unmatched extras',
        'raw field bytes, observed slot/ObjectID/tokenCandidate/netObjectPointer',
        'authored10, unique matches, missing/extra/duplicate/ambiguous rows; three excluded trigger tokens',
        'exact app/PID/start/nonce/source/map identity and integrity status',
    ]
    require(policy['stability'] == 'UNKNOWN_CONCURRENT_BEST_EFFORT'
            and not any(policy[key] for key in ('automatic_catalog_activation', 'invoke_guest_getters', 'write_guest_memory')),
            'passive observer policy mismatch')
    # Native observer's strict JSON reader accepts unsigned numbers only.
    # Preserve the qualified signed provenance sentinel as text; the complete
    # original proof remains pinned by startup_proof_sha256 above.
    startup_metadata = copy.deepcopy(startup['dataflow'])
    require(startup_metadata['initializer_sequence_index_request'] == -1,
            'changed signed startup sequence sentinel')
    startup_metadata['initializer_sequence_index_request'] = '-1'
    return {'schema_version': 1, 'format': 'mag_passive_spawn_observer', 'observation_profile': 'objective_v1',
            'purpose': 'Concurrent objective diagnostic only; token candidates require independent capture review before catalog activation.',
            'layout': layout, 'diagnostic_policy': policy, 'authored_points': objectives, 'excluded_trigger_tokens': triggers,
            'source': {'map': graph['map']['map'], 'catalog_sha256': graph_sha,
                       'spawn_observer_sha256': spawn_sha, 'entity_pool_proof_sha256': pool_sha,
                       'binding_proof_sha256': binding_sha, 'startup_proof_sha256': startup_sha,
                       'original_elf_sha256': original, 'map_archive_sha256': graph['source']['level_archive_sha256'],
                       'map_clv_sha256': graph['source']['clv_sha256'], 'projector_sha256': sha(Path(__file__).read_bytes())},
            'source_checks': {'entity_pool': flow, 'binding_identity': binding['identity'],
                              'startup_dataflow': startup_metadata, 'manager_source': spawn['source']},
            'unresolved': ['Qualified static manager root is not independent current-runtime root proof.',
                           'UNKNOWN_CONCURRENT_BEST_EFFORT: no owner barrier or stable map-load serialization is established.',
                           'No guessed authored ordinal-to-slot mapping or automatic activation is permitted.']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--catalog', type=Path, required=True)
    parser.add_argument('--spawn-observer', type=Path, required=True)
    parser.add_argument('--pool-proof', type=Path, required=True)
    parser.add_argument('--binding-proof', type=Path, required=True)
    parser.add_argument('--startup-proof', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        emit(args, produce(args), [args.catalog, args.spawn_observer, args.pool_proof, args.binding_proof,
                                  args.startup_proof, Path(__file__)])
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))


if __name__ == '__main__':
    main()
