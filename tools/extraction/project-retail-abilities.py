#!/usr/bin/env python3
"""Project authored retail command availability/timing. Does not enable effects.

Input is the maintained extract-retail-rules.py output. Numeric protocol identities
are independently established by original native evidence; authored balance values
are read from the input, never supplied as defaults here.
"""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
COMMANDS = {
    'ClusterBomb': 'cluster_bomb_properties',
    'PrecisionStrike': 'ability_precision_strike_properties',
    'Sensor': 'ability_sensor_properties',
    'GasBombardment': 'ability_gas_bombardment_properties',
    'StrafingRun': 'ability_strafing_run_properties',
    'GuidedArtillery': 'ability_guided_artillery_properties',
}


def check(ok, reason):
    if not ok:
        raise ValueError(reason)


def unique(pairs):
    result = {}
    for key, value in pairs:
        check(key not in result, 'duplicate_json_key:' + key)
        result[key] = value
    return result


def duration(table, key):
    value = table[key]
    check(type(value) is int and 0 < value < 2**32, 'invalid_duration:' + key)
    return value


def elements(table, prefix=''):
    check(type(table) is dict, 'invalid_list')
    result = []
    for i, (key, value) in enumerate(table.items()):
        check(key == 'Element' + str(i) and set(value) == {'element'}, 'invalid_element_order')
        item = value['element']
        check(type(item) is str and item.startswith(prefix), 'invalid_element_type')
        result.append(item[len(prefix):])
    check(len(set(result)) == len(result), 'duplicate_ability')
    return result


def project(raw):
    source = json.loads(raw, object_pairs_hook=unique,
                        parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
    check(source['schema_version'] == 1 and source['application_id'] == 20231,
          'unsupported_rules_source')
    resources = source['resources']
    member = 'scripts/abilities/global_ability_properties.zpr'
    global_props = resources[member]['parameters']['GlobalAbilityProps']
    modes = []
    for players in (64, 128, 256):
        for side, native_side in [('attacker', 'Attacker'), ('defender', 'Defender')]:
            squad = elements(global_props[f'{native_side}SquadCommandTypes{players}'], 'kAbilityActionType')
            platoon = elements(global_props[f'{native_side}PlatoonCommandTypes{players}'], 'kAbilityActionType')
            check(all(item in COMMANDS for item in squad + platoon), 'unknown_command_ability')
            modes.append({'players': players, 'side': side, 'squad': squad, 'platoon': platoon,
                          'strategic': elements(global_props[f'{native_side}StrategicTypes{players}'],
                                                'kStrategicType')})
    commands = []
    for name, stem in COMMANDS.items():
        path = 'scripts/abilities/' + stem + '.zpr'
        entry = resources[path]
        check(len(entry['parameters']) == 1, 'ambiguous_ability_table:' + path)
        table = next(iter(entry['parameters'].values()))
        commands.append({'ability': name, 'source_member': path, 'source_sha256': entry['sha256'],
                         'cooldown_ms': duration(table, 'CoolDown_ms'),
                         'delay_ms': duration(table, 'Delay_ms')})
    strategic = []
    for path, entry in resources.items():
        if not path.startswith('scripts/abilities/strategic_'):
            continue
        name, table = next(iter(entry['parameters'].items()))
        check(name.startswith('Strategic') and name.endswith('PropsTable'), 'unknown_strategic_table')
        strategic.append({'ability': name[len('Strategic'):-len('PropsTable')],
                          'source_member': path, 'source_sha256': entry['sha256'],
                          'cooldown_ms': duration(table, 'CoolDown_ms'),
                          'duration_ms': duration(table, 'Duration_ms')})
    return {'schema_version': 1, 'application_id': 20231,
            'rules_sha256': hashlib.sha256(raw).hexdigest(),
            'source_archive_sha256': source['source_archive_sha256'],
            'global_source_member': member, 'global_source_sha256': resources[member]['sha256'],
            'command_global_cooldown_ms': duration(global_props, 'CommandGlobalCoolDown_ms'),
            'strategic_global_cooldown_ms': duration(global_props, 'StrategicGlobalCoolDown_ms'),
            'authority': {'effects': False, 'target_validation': False, 'ability_replication': False},
            'modes': modes, 'commands': commands, 'strategic': strategic}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rules', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [v for k, v in vars(args).items() if k != "output" and isinstance(v, Path)]
    if args.output.resolve() in {v.resolve() for v in inputs}:
        parser.error("output must not replace source inputs")
    output = (json.dumps(project(args.rules.read_bytes()), indent=2, allow_nan=False) + '\n').encode()
    if args.check:
        check(args.output.read_bytes() == output, 'retail_abilities_projection_changed')
    else:
        args.output.write_bytes(output)
    print('retail abilities: six command definitions, six mode/side lists, effects disabled')


if __name__ == '__main__':
    main()
