#!/usr/bin/env python3
"""Join reviewed retail token/slot bindings with authored objective startup data.

No ordinal fallback, original binary resource loading, Lua execution, client
launch or automatic catalog activation is performed. Capture normalization is
an independent review step; this tool consumes its explicit qualified result.
"""
import argparse
import json
import math
from pathlib import Path
import re
import runpy
import struct

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / 'tools/extraction/project-retail-objective-observer.py'
_api = runpy.run_path(str(HELPER))
require, load, token, fields, emit = (_api[k] for k in ('require', 'load', 'token', 'fields', 'emit'))


def uint(value, maximum, label):
    require(type(value) is int and 0 <= value <= maximum, 'invalid ' + label)
    return value


def digest(value, label):
    require(isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value), 'invalid ' + label)
    return value


def float32(value, label):
    require(type(value) in (int, float) and math.isfinite(value)
            and abs(value) <= 3.4028234663852886e38, 'invalid ' + label)
    return struct.unpack('>f', struct.pack('>f', value))[0]


def interactions(graph, maximum_distance):
    # This point-distance limit is a development policy, not a recovered
    # original trigger-volume range. Only linked secure-and-hold and structural
    # demolition objectives qualify; unlinked proxy objectives remain excluded.
    radius = float32(maximum_distance, 'interaction radius')
    require(radius > 0, 'interaction radius must be positive')
    definitions = {token(row['definition_token']): row for row in graph['objectives']['definitions']}
    linked = {}
    for row in graph['trigger_links']:
        name = token(row['objective_instance_token'])
        linked[name] = linked.get(name, 0) + 1
    result = {}
    for row in graph['objectives']['instances']:
        name = token(row['name_token'])
        definition = definitions[token(row['definition_token'])]
        class_name = definition['class_name']
        if name not in linked or class_name not in (
                'ObjectiveSecureAndHoldProperties', 'ObjectiveDemolitionStructuralProperties'):
            continue
        prop = {key: value['value'] for key, value in definition['fields'].items() if 'value' in value}
        prop.update(fields(row['fields']))
        can_deactivate = prop['CanDeactivate']
        require(type(can_deactivate) is bool, 'invalid authored CanDeactivate')
        common = {
            'owner_team': uint(prop['OwnerTeam'], 1, 'authored OwnerTeam'),
            'position': [float32(prop['Translate' + axis], 'authored Translate' + axis) for axis in 'XYZ'],
            'trigger_count': uint(linked[name], 255, 'linked trigger count'),
            'maximum_distance_m': radius,
            'distance_policy': 'development_point_distance',
        }
        if class_name == 'ObjectiveSecureAndHoldProperties':
            result[name] = {'interaction': dict(common,
                activate_ms=uint(prop['TimeToActivateMs'], 0xffffffff, 'authored TimeToActivateMs'),
                complete_ms=uint(prop['TimeToCompleteMs'], 0xffffffff, 'authored TimeToCompleteMs'),
                deactivate_ms=uint(prop['TimeToDeactivateMs'], 0xffffffff, 'authored TimeToDeactivateMs'),
                can_deactivate=can_deactivate)}
        else:
            timings = {key: uint(prop[source], 3_600_000, 'authored ' + source)
                       for key, source in [('arm_ms', 'TimeToArmMs'),
                                           ('detonate_ms', 'TimeToDetonateMs'),
                                           ('defuse_ms', 'TimeToDefuseMs')]}
            require(all(timings.values()), 'demolition timings must be positive')
            result[name] = {'demolition': dict(common, **timings,
                can_defuse=can_deactivate,
                activation_score=uint(prop['ActivationScoreValue'], 0xffffffff, 'authored ActivationScoreValue'),
                deactivation_score=uint(prop['DeactivationScoreValue'], 0xffffffff, 'authored DeactivationScoreValue'))}
    return result


def reward_rules(graph):
    """Keep authored reward classification on disk, including unlinked proxies.

    DemolitionStructural's reviewed native virtual type is 1. The Boolean
    records that class fact; category/subtype remain separate authored enums.
    XP amounts continue to come from the progression catalog.
    """
    definitions = {token(row['definition_token']): row for row in graph['objectives']['definitions']}

    def enum(prop, maximum, label):
        require(prop.get('type') == 'enum', 'invalid authored ' + label + ' type')
        encoded = bytes.fromhex(prop['raw_hex'])
        require(len(encoded) == 13 and encoded[:9] == bytes.fromhex('deadbeef0000000100'),
                'invalid authored ' + label + ' encoding')
        return uint(int.from_bytes(encoded[9:], 'big'), maximum, 'authored ' + label)

    result = {}
    for row in graph['objectives']['instances']:
        definition = definitions[token(row['definition_token'])]
        class_name = definition['class_name']
        if class_name not in (
                'ObjectiveSecureAndHoldProperties', 'ObjectiveDemolitionStructuralProperties'):
            continue
        prop = dict(definition['fields'])
        # Preserve any authored per-instance enum override, with the same
        # strict encoded representation as its definition.
        for override in row['fields']:
            if override['name'] in ('Category', 'SubType'):
                prop[override['name']] = override
        result[token(row['name_token'])] = {'reward': {
            'category': enum(prop['Category'], 4, 'Category'),
            'subtype': enum(prop['SubType'], 255, 'SubType'),
            'demolition': class_name == 'ObjectiveDemolitionStructuralProperties',
        }}
    return result


def scoring(graph):
    """Include unlinked control proxies: dummy C supplies the first 600 points."""
    definitions = {token(row['definition_token']): row for row in graph['objectives']['definitions']}
    result = {}
    for row in graph['objectives']['instances']:
        definition = definitions[token(row['definition_token'])]
        if definition['class_name'] not in (
                'ObjectiveSecureAndHoldProperties', 'ObjectiveDemolitionStructuralProperties'):
            continue
        prop = {key: value['value'] for key, value in definition['fields'].items() if 'value' in value}
        prop.update(fields(row['fields']))
        result[token(row['name_token'])] = {'scoring': {
            'owner_team': uint(prop['OwnerTeam'], 1, 'scoring OwnerTeam'),
            'activation_score': uint(prop['ActivationScoreValue'], 0xffffffff, 'ActivationScoreValue'),
            'deactivation_score': uint(prop['DeactivationScoreValue'], 0xffffffff, 'DeactivationScoreValue'),
        }}
    return result


def match_rules(rules):
    resource = rules['resources']['settings/gameproperties.zpr']
    types = resource['parameters']['GameCreationPropertiesTable']['m_gameCreationTypes']
    selected = [(name, row['GameCreationType']) for name, row in types.items()
                if row['GameCreationType']['m_gameType'] == 'GameType64']
    require(len(selected) == 1, 'ambiguous Sabotage game rules')
    name, creation = selected[0]
    authored = creation['m_gameRules']
    return ({
        'end_game_delay_ms': uint(authored['EndGameDelay'], 3_600_000, 'EndGameDelay'),
        'major_victory_percent': uint(authored['MajorVictoryPercent'], 100, 'MajorVictoryPercent'),
    }, {
        'game_rules_sha256': digest(resource['sha256'], 'game rules resource SHA256'),
        'game_rules_json_pointer': '/GameCreationPropertiesTable/m_gameCreationTypes/'
                                  + name + '/GameCreationType/m_gameRules',
    })


def control_rules(graph, bound):
    """Project the reviewed literal control subset; never evaluate Lua branches."""
    controls = graph['control_argument_bindings']
    require(len(controls) == 1 and controls[0]['function'] == 'ObjectiveControl',
            'ambiguous progression control')
    control = controls[0]
    roles = {row['parameter']: token(row['Object']) for row in control['bindings']}
    require(len(roles) == len(control['bindings']), 'duplicate progression role')

    def function(member, name):
        resources = [r for r in graph['original_resources'] if r['member'] == member]
        require(len(resources) == 1, 'unresolved control resource')
        rows = [f for f in resources[0]['converted_properties']['functions'] if f['name'] == name]
        require(len(rows) == 1, 'unresolved control function')
        return rows[0]

    def index(role):
        require(role in roles and roles[role] in bound, 'unresolved progression objective')
        return bound[roles[role]]['object_index']

    def state_calls(func, state):
        result = []
        for call in func['calls_in_source_order']:
            if call['api'] != 'Mission_SetObjectiveState':
                continue
            args = call['arguments']
            require(len(args) == 3 and all(a['kind'] == 'symbol' for a in args)
                    and args[1]['name'] == 'ZScript.Caller', 'unsupported objective state call')
            if args[2]['name'] == 'ZScript.ObjectiveState.kObjectiveState' + state:
                result.append(args[0]['name'])
        require(len(result) == len(set(result)), 'duplicate objective state target')
        return result

    def bleed(func):
        calls = [c for c in func['calls_in_source_order'] if c['api'] == 'Game_SetScoreBleed']
        require(len(calls) == 1 and len(calls[0]['arguments']) == 3
                and all(a['kind'] == 'integer' and type(a['value']) is int
                        for a in calls[0]['arguments']), 'unsupported score bleed call')
        seconds, points, team = [a['value'] for a in calls[0]['arguments']]
        return (uint(seconds, 3600, 'score bleed seconds') * 1000,
                uint(points, 32767, 'score bleed points'), uint(team, 1, 'score bleed team'))

    func = function(control['script_member'], control['function'])
    require(func['parameters'] == list(roles), 'control parameters differ from binding order')
    conditions = func['conditions_in_source_order']
    require(len(conditions) == 2 and all(c['operator'] == 'all' for c in conditions),
            'unsupported progression conditions')
    prerequisites = []
    for comparison in conditions[1]['comparisons']:
        require(comparison['operator'] == 'equal'
                and comparison['state_symbol'] == 'ZScript.ObjectiveState.kObjectiveStateCompleted',
                'unsupported progression prerequisite')
        prerequisites.append(comparison['objective_parameter'])
    require(prerequisites == ['objective_a', 'objective_b']
            and conditions[0]['comparisons'] == [
                {'objective_parameter': role, 'operator': 'not_equal',
                 'state_symbol': 'ZScript.ObjectiveState.kObjectiveStateLocked'}
                for role in prerequisites], 'unsupported progression guard')
    locked, completed = state_calls(func, 'Locked'), state_calls(func, 'Completed')
    require(locked == ['objective_a', 'objective_b', 'objective_a_sat', 'objective_b_sat']
            and completed == ['spawn_a', 'spawn_b', 'spawn_c', 'spawn_d', 'dummy_c']
            and state_calls(func, 'Inactive') == ['objective_c'], 'unsupported progression effects')

    definitions = {token(r['definition_token']): r for r in graph['objectives']['definitions']}
    instances = {token(r['name_token']): r for r in graph['objectives']['instances']}
    scripts = {}
    for row in graph['script_objects']:
        for instance in row['instances']:
            name = token(fields(instance['fields'])['Name'])
            require(name not in scripts, 'duplicate control script instance')
            scripts[name] = fields(row['fields'])

    def objective_property(role, prop):
        definition = definitions[token(instances[roles[role]]['definition_token'])]
        return token(definition['fields'][prop]['token'])

    require(scripts[token(control['script_instance_token'])]['ServerFunction'] == control['function'],
            'unresolved main control script')
    delayed = scripts[objective_property('dummy_c', 'CompletedScript')]['ServerEntryPoints']
    require(len(delayed) == 1, 'ambiguous delayed control entry')
    entry = fields(delayed[0]['fields'])
    final = function('/run/mag/ps3/scripts/' + entry['Script'], entry['Function'])
    require(final['parameters'] == ['objective_c']
            and state_calls(final, 'Deactivated') == ['objective_c']
            and len(entry['Params']) == 1
            and token(fields(entry['Params'][0]['fields'])['Object']) == roles['objective_c'],
            'unsupported delayed objective binding')
    delay_ms = uint(entry['TimeDelay'], 3_600_000, 'progression delay')
    require(delay_ms > 0, 'progression delay must be positive')

    interval_ms, points, team = bleed(func)
    require(interval_ms > 0 and points > 0, 'score bleed must be positive')
    for property_name, expected_points, expected_function in [
            ('ActivatedScript', 0, 'TimerControlHold'),
            ('DeactivatedScript', points, 'TimerControlStart')]:
        script = scripts[objective_property('objective_c', property_name)]
        require(script['ServerFunction'] == expected_function, 'unsupported clock callback')
        callback = function('/run/mag/ps3/scripts/' + script['ServerScript'], script['ServerFunction'])
        require(bleed(callback) == (interval_ms, expected_points, team), 'inconsistent clock callback')
    for property_name in ('DeactivatingScript', 'CompletedScript'):
        require(objective_property('objective_c', property_name) == '0x00000000',
                'unsupported demolition clock callback')
    return {
        'progression': {'prerequisites': [index(r) for r in prerequisites],
                        'lock': [index(r) for r in locked], 'complete': [index(r) for r in completed],
                        'delayed_objective': index('objective_c'), 'delay_ms': delay_ms},
        'score_bleed': {'team': team, 'interval_ms': interval_ms, 'points': points,
                       'pause_objective': index('objective_c'),
                       # Known native ObjectiveState_t Activated, independently proved
                       # by check-retail-objective-states.py; not an authored enum value.
                       'pause_state': 3,
                       # Native structural demolition keeps Activated's hold
                       # in effect through Deactivating4 and Completed7. Those
                       # authored callbacks are null; only Deactivated1 resumes
                       # through TimerControlStart. Match-end is separate policy.
                       'pause_states': [3, 4, 7]},
    }


def produce(args):
    local_end_delay = getattr(args, 'local_end_game_delay_ms', None)
    if local_end_delay is not None:
        require(getattr(args, 'include_interactions', False),
                'local end-game delay requires --include-interactions')
        uint(local_end_delay, 60_000, 'local end-game delay')
        require(local_end_delay > 0, 'local end-game delay must be positive')
    graph, graph_sha = load(args.catalog)
    binding, _ = load(args.bindings)
    startup, startup_sha = load(args.startup_proof)
    objectives, triggers = _api['authored_entities'](graph)
    interaction_by_token = (interactions(graph, getattr(args, 'interaction_radius_m', 5.0))
                            if getattr(args, 'include_interactions', False) else {})
    scoring_by_token = scoring(graph) if getattr(args, 'include_interactions', False) else {}
    reward_by_token = reward_rules(graph) if getattr(args, 'include_interactions', False) else {}
    map_name = graph['map']['map']
    require(binding['format'] == 'mag_retail_objective_slot_bindings'
            and type(binding['schema_version']) is int and binding['schema_version'] == 1,
            'wrong normalized bindings format/schema')
    require(type(binding['application_id']) is int and binding['application_id'] == graph['application_id'] == 20231,
            'wrong application identity')
    require(binding['map_name'] == map_name and type(binding['entity_family']) is int and binding['entity_family'] == 12,
            'wrong map or entity family')
    require(binding['qualification'] == 'bounded_concurrent_token_slot_capture', 'unqualified token/slot capture')
    image_sha = digest(binding['source_image_sha256'], 'source image SHA256')
    capture_sha = digest(binding['capture_sha256'], 'capture SHA256')
    require(startup['schema_version'] == 1 and startup['status'] == 'STATIC_RETAIL_DESIRED_STATE_MAPPING_AND_GATED_MISSION_START_REQUEST_PROVED',
            'unqualified startup proof')
    dataflow = startup['dataflow']
    mapping = dataflow['desired_mapping']
    require(set(mapping) == {'0', '1', '2', '3'} and all(type(v) is int and 0 <= v <= 12 for v in mapping.values()),
            'invalid startup desired mapping')
    inactive = uint(dataflow['initializer_network_state_request'], 12, 'initializer network state')
    require(dataflow['transition_request_is_not_network_publication'] is True
            and dataflow['authored_active_bool_offset'] == '40', 'unqualified separate Active gate')
    rows = binding['objects']
    require(isinstance(rows, list) and len(rows) == len(objectives), 'missing or extra objective bindings')
    expected = {row['raw_token_expected']: row for row in objectives}
    bound, indices = {}, set()
    for row in rows:
        name = token(row['token'])
        require(name not in triggers, 'trigger token is not an objective entity')
        require(name in expected, 'unknown objective token')
        require(name not in bound, 'duplicate objective token')
        index = uint(row['object_index'], 127, 'objective object_index')
        require(index not in indices, 'duplicate objective slot')
        packed = row['packed_id']
        if isinstance(packed, str):
            packed = int(token(packed), 16)
        packed = uint(packed, 0xffffffff, 'packed objective ID')
        require(packed == 0x0c800000 | index, 'packed objective family/slot mismatch')
        authored = expected[name]
        state = mapping[str(authored['authored_initial_state'])] if authored['authored_active'] else inactive
        bound[name] = {'token': name, 'object_index': index, 'initial_state': state}
        if name in interaction_by_token:
            bound[name].update(interaction_by_token[name])
        if name in scoring_by_token:
            bound[name].update(scoring_by_token[name])
        if name in reward_by_token:
            bound[name].update(reward_by_token[name])
        indices.add(index)
    require(set(bound) == set(expected), 'missing objective token bindings')
    platoons = {}
    for row in graph['platoons']:
        prop = fields(row['fields'])
        require(len(row['instances']) == 1, 'ambiguous authored platoon instance')
        instance = fields(row['instances'][0]['fields'])
        name = token(instance['Name'])
        require(name not in platoons, 'duplicate platoon instance')
        platoons[name] = (uint(prop['Team'], 1, 'platoon Team'), uint(prop['ID'], 3, 'platoon ID'))
    squads, seen_squads = [], set()
    for row in graph['squad_objective_lists']:
        prop = fields(row['fields'])
        platoon = token(prop['Platoon'])
        require(platoon in platoons, 'unresolved squad platoon instance')
        team, platoon_id = platoons[platoon]
        squad = uint(prop['Squad'], 3, 'authored Squad')
        index = 16 * team + 4 * platoon_id + squad
        require(index not in seen_squads, 'duplicate squad list')
        seen_squads.add(index)
        authored_list, initial = [], None
        for authored in prop['SquadObjectives']:
            target = fields(authored['fields'])
            name = token(target['Objective'])
            require(name in bound, 'unresolved authored squad objective')
            objective = bound[name]
            authored_list.append({'object_index': objective['object_index'],
                                  'order': uint(target['Order'], 255, 'authored Order')})
            # Startup availability uses native Deactivated1/Activated3. Order
            # remains opaque metadata; duplicate source-list entries survive.
            if initial is None and objective['initial_state'] in (1, 3):
                initial = objective['object_index']
        require(initial is not None, 'squad has no initially available authored objective')
        squads.append({'team': team, 'squad_index': index, 'initial_objective_index': initial,
                       'authored_objectives': authored_list})
    require(len(squads) == 8 and {s['team'] for s in squads} == {0, 1}, 'expected eight both-team authored squads')
    result = {'schema_version': 1, 'format': 'mag_retail_objective_catalog', 'application_id': 20231,
            'protocol': 1740, 'game_type': 0, 'map_name': map_name, 'entity_family': 12, 'network_type': 12,
            'source': {'authored_catalog_sha256': graph_sha, 'binding_capture_sha256': capture_sha,
                       'startup_proof_sha256': startup_sha, 'source_image_sha256': image_sha,
                       'capture_qualification': binding['qualification']},
            'objects': sorted(bound.values(), key=lambda row: row['object_index']),
            'squads': sorted(squads, key=lambda row: row['squad_index'])}
    if getattr(args, 'include_interactions', False):
        result.update(control_rules(graph, bound))
        rules, rules_sha = load(args.rules)
        require(rules['application_id'] == graph['application_id'], 'wrong rules application identity')
        result['match_rules'], provenance = match_rules(rules)
        if local_end_delay is not None:
            result['match_rules']['local_end_game_delay_ms'] = local_end_delay
        result['source'].update(provenance, rules_catalog_sha256=rules_sha)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--catalog', type=Path, required=True)
    parser.add_argument('--bindings', type=Path, required=True, help='Reviewed normalized token/slot capture JSON')
    parser.add_argument('--startup-proof', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--include-interactions', action='store_true',
                        help='Include interactions, progression, score bleed, objective scoring/reward classification and match rules')
    parser.add_argument('--rules', type=Path, required=True)
    parser.add_argument('--local-end-game-delay-ms', type=int,
                        help='Explicit local post-match delay override, 1..60000ms; requires --include-interactions')
    parser.add_argument('--interaction-radius-m', type=float, default=5.0,
                        help='Development point-distance limit in metres; not recovered original range (default: 5.0)')
    args = parser.parse_args()
    try:
        emit(args, produce(args), [args.catalog, args.bindings, args.startup_proof, args.rules, HELPER, Path(__file__)])
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))


if __name__ == '__main__':
    main()
