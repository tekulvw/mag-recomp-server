#!/usr/bin/env python3
"""Bind original Domination class/timings and proved trigger indices to a candidate catalog."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
import math
from pathlib import Path
import runpy

TOOLS = Path(__file__).resolve().parent
TRIGGERS = runpy.run_path(str(TOOLS / 'project-retail-domination-triggers.py'))
RETAIL_SHA = TRIGGERS['RETAIL_SHA']
require, sha, encode, integer, token = (
    TRIGGERS[key] for key in ('require', 'sha', 'encode', 'integer', 'token')
)
SECURE = 'ObjectiveSecureAndHoldProperties'
STRUCTURAL = 'ObjectiveDemolitionStructuralProperties'
CLASS = {SECURE: 'secure_hold', STRUCTURAL: 'structural'}
HOLD = ('OwnerTeam', 'CanDeactivate', 'TimeToActivateMs', 'TimeToCompleteMs', 'TimeToDeactivateMs')
REPAIR = ('OwnerTeam', 'CanRepair', 'TimeToRepairMs', 'MinTimeToRepairMs')


def single(rows, field, value):
    chosen = [row for row in rows if row[field] == value]
    require(len(chosen) == 1, f'Expected one source row for {value}')
    return chosen[0]


def fields(definition):
    return {name: row['value'] for name, row in definition['fields'].items() if 'value' in row}


def valid_bool(value, name):
    require(type(value) is bool, f'Expected authored bool {name}')
    return value


def groups(rows, keys):
    return Counter(tuple(row[key] for key in keys) for row in rows)


def verify_groups(placed_defs, proof, cls, keys):
    authored = groups([fields(definition) for definition in placed_defs if definition['class_name'] == cls], keys)
    observed = {tuple(row[key] for key in keys): integer(row['definitions'], 128) for row in proof}
    require(len(observed) == len(proof) and authored == observed, f'{cls} original property groups differ')


def source_rows(metadata, base, start, order, interactions, repair):
    source, name = base['source'], base['map_name']
    digest = source['authored_catalog_sha256']
    for receipt in (interactions, repair):
        require(receipt['retail_elf_sha256'] == RETAIL_SHA and receipt['metadata_sha256'] == digest,
                'Original interaction/repair source mismatch')
    require(start['schema'] == 'mag_domination_mission_start_requests_v1'
            and start['status'] == 'ORIGINAL_AUTHORED_AND_STATIC_NATIVE_STARTUP_REQUESTS_BOUND'
            and start['source']['bindings_sha256'] == source['binding_capture_sha256']
            and start['source']['native_program_sha256'] == RETAIL_SHA
            and start['source']['authored_metadata_sha256'] == digest,
            'Original MissionStart source mismatch')
    require(order['schema'] == 'mag_domination_structural_sim_schedule_v1'
            and order['status'] == 'RETAIL_SIM_SOURCE_ORDER_AND_COMPLETION_PREDICATE_BOUND'
            and order['retail_elf_sha256'] == RETAIL_SHA and order['metadata_sha256'] == digest
            and order['client_source_bindings_sha256'] == source['binding_capture_sha256']
            and order['completed_predicate'] == [5, 7],
            'Unqualified Sim schedule')
    stage = single(start['maps'], 'map_name', name)
    schedule = single(order['maps'], 'map', name)
    map_data, = [row for row in metadata['maps'] if row['map']['map'] == name]
    require(stage['source'] == map_data['source'], 'MissionStart level mismatch')
    by_slot = sorted(base['objects'], key=lambda row: row['object_index'])
    require([(slot, token(name)) for slot, name in enumerate(schedule['ordered_tokens'])]
            == [(row['object_index'], token(row['token'])) for row in by_slot],
            'Client and Sim objective bindings disagree')
    definitions = TRIGGERS['unique'](map_data['objectives']['definitions'], lambda row: row['definition_token'])
    instances = TRIGGERS['unique'](map_data['objectives']['instances'], lambda row: token(row['name_token']))
    expected = {name for name, row in instances.items() if definitions[row['definition_token']]['class_name'] in CLASS}
    require(expected == {token(row['token']) for row in by_slot}, 'Incomplete class inventory')
    startup = TRIGGERS['unique'](stage['objectives'], lambda row: token(row['token']))
    require(startup.keys() == expected, 'Incomplete MissionStart inventory')
    verify_groups(definitions.values(), interactions['authored_secure_hold_timing_groups'][name], SECURE, HOLD)
    verify_groups(definitions.values(), repair['structural_authored_rule_groups'][name], STRUCTURAL, REPAIR)
    return by_slot, definitions, instances, startup


def range_fields(position, boxes):
    # This field is required by the shared geometry schema. It is the geometric
    # bound of the supplied boxes, not a point-distance interaction threshold.
    radius = max(math.dist(position, b['centre']) + math.hypot(*b['half_extents']) for b in boxes)
    require(math.isfinite(radius) and radius > 0, 'Invalid authored volume envelope')
    return dict(position=position, trigger_count=len(boxes), maximum_distance_m=radius,
                distance_policy='authored_trigger_box', trigger_volumes=boxes)


def structural_repair(raw, owner):
    return dict(owner_team=owner, can_repair=valid_bool(raw['CanRepair'], 'CanRepair'),
                repair_ms=integer(raw['TimeToRepairMs'], 3_600_000),
                minimum_repair_ms=integer(raw['MinTimeToRepairMs'], 3_600_000))


def structural_demolition(raw, geometry, score):
    times = {key: integer(raw[source], 3_600_000) for key, source in (
        ('arm_ms', 'TimeToArmMs'), ('detonate_ms', 'TimeToDetonateMs'), ('defuse_ms', 'TimeToDefuseMs')
    )}
    require(all(times.values()), 'Invalid Structural action durations')
    return dict(**geometry, **score, **times,
                can_defuse=valid_bool(raw['CanDeactivate'], 'CanDeactivate'))


def bind_one(obj, definition, instance, intent, boxes):
    cls = definition['class_name']
    raw = fields(definition)
    require(intent['property_class'] == cls and intent['client_network_index'] == obj['object_index']
            and intent['property_table_sha256'] == definition['source_sha256']
            and intent['property_active'] == definition['fields']['Active']
            and intent['property_initial_active_state'] == definition['fields']['InitialActiveState']
            and intent['current_network_state_after_mission_start'] is None
            and intent['sim_network_index'] is None and obj['initial_state'] == 0,
            'Objective class/startup binding mismatch')
    # The placed InstanceProperties can hold authored transform values, but an
    # undocumented per-instance gameplay override would need its own proof.
    names = {row['name'] for row in instance['fields']}
    require(not names.intersection(raw), 'Unexpected instance gameplay property override')
    owner = integer(raw['OwnerTeam'], 1)
    score = dict(owner_team=owner, activation_score=integer(raw['ActivationScoreValue'], 32767),
                 deactivation_score=integer(raw['DeactivationScoreValue'], 32767))
    result = dict(**obj, native_class=CLASS[cls], scoring=score)
    if cls == STRUCTURAL:
        result['repair'] = structural_repair(raw, owner)
    if not boxes:
        return result
    pos = {row['name']: row['value'] for row in instance['fields']}
    geometry = range_fields([pos['Translate' + axis] for axis in 'XYZ'], boxes)
    if cls == SECURE:
        can_deactivate = valid_bool(raw['CanDeactivate'], 'CanDeactivate')
        result['interaction'] = dict(**geometry, owner_team=owner, can_deactivate=can_deactivate,
            activate_ms=integer(raw['TimeToActivateMs'], 3_600_000),
            complete_ms=integer(raw['TimeToCompleteMs'], 3_600_000),
            deactivate_ms=integer(raw['TimeToDeactivateMs'], 3_600_000))
        require(result['interaction']['activate_ms'] and result['interaction']['deactivate_ms'],
                'Invalid SecureAndHold action durations')
    else:
        result['demolition'] = structural_demolition(raw, geometry, score)
    return result


def project(metadata_raw, base_raw, trigger_raw, startup_raw, sim_raw, interaction_raw, repair_raw):
    require(len(metadata_raw) <= 32 * 1024**2 and all(len(item) <= 8 * 1024**2 for item in
            (base_raw, trigger_raw, startup_raw, sim_raw, interaction_raw, repair_raw)), 'Input exceeds bound')
    trigger_sidecar = TRIGGERS['project'](metadata_raw, trigger_raw, base_raw)
    metadata, base, start, order, interactions, repair = map(
        json.loads, (metadata_raw, base_raw, startup_raw, sim_raw, interaction_raw, repair_raw)
    )
    by_slot, definitions, instances, startup = source_rows(metadata, base, start, order, interactions, repair)
    geometry = defaultdict(list)
    for row in trigger_sidecar['triggers']:
        volume = row['authored_volume']
        require(volume['trigger_type_raw_hex'] == 'deadbeef000000010000000003',
                'Unexpected original interaction trigger type')
        geometry[row['object_index']].append({
            field: volume['box'][field] for field in ('centre', 'axes', 'half_extents')
        })
    rows = []
    for obj in by_slot:
        key = token(obj['token'])
        instance = instances[key]
        rows.append(bind_one(obj, definitions[instance['definition_token']], instance,
                             startup[key], geometry.get(obj['object_index'], [])))
    source = {key: value for key, value in base['source'].items() if key != 'initial_load_scope'}
    source.update(capture_qualification='native_source_gameplay', startup_proof_sha256=sha(startup_raw))
    return dict(base, source=source, objects=rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    fields = ('metadata', 'initial', 'trigger-bindings', 'startup', 'sim-schedule', 'interaction-proof', 'repair-proof')
    for name in (*fields, 'native-elf', 'output'):
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        inputs = [getattr(args, field.replace('-', '_')) for field in fields]
        output = args.output.resolve()
        require(not output.is_relative_to(TOOLS.parents[1])
                and output not in {p.resolve() for p in [*inputs, args.native_elf]},
                'Output must be outside the source checkout and inputs')
        with args.native_elf.open('rb') as image:
            require(hashlib.file_digest(image, 'sha256').hexdigest() == RETAIL_SHA, 'Wrong original retail ELF')
        result = encode(project(*(path.read_bytes() for path in inputs)))
        if args.check:
            require(output.read_bytes() == result, 'Gameplay objectives differ from reproduction')
        else:
            with output.open('xb') as file:
                file.write(result)
    except (KeyError, TypeError, OSError, ValueError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
