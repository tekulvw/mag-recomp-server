#!/usr/bin/env python3
"""Layer authored XP classifications onto a private retail objective catalog."""

import argparse
import json
from pathlib import Path
import runpy

TOOLS = Path(__file__).resolve().parent
FRAGO = runpy.run_path(str(TOOLS / 'project-retail-frago-rules.py'))
reward_rules = runpy.run_path(str(TOOLS / 'project-retail-objective-runtime.py'))['reward_rules']
RETAIL_SHA = FRAGO['RETAIL_SHA']
MODE_CLASSES = FRAGO['MODE_CLASSES']
RUNTIME_CLASSES = {FRAGO['SECURE']: 'secure_hold', FRAGO['DEMOLITION']: 'structural', FRAGO['ARRIVAL']: None}
CATALOG_LIMIT, AUTHORED_LIMIT, ELF_LIMIT = (
    FRAGO[key] for key in ('CATALOG_LIMIT', 'AUTHORED_LIMIT', 'ELF_LIMIT')
)
require, sha, token, unique, parse_document, read_bounded, source_instances = (
    FRAGO[key] for key in ('require', 'sha', 'token', 'unique', 'parse_document',
                          'read_bounded', 'source_instances')
)


def classifications(selected, target_classes):
    # The shared classifier uses the last instance property. Reject ambiguous
    # inputs before handing it those otherwise supported overrides.
    for instance in selected['objectives']['instances']:
        overrides = [row for row in instance['fields'] if row['name'] in ('Category', 'SubType')]
        unique(overrides, lambda row: row['name'],
               'authored Category or SubType instance override')
    try:
        return reward_rules(selected, target_classes)
    except KeyError as error:
        raise ValueError(f'Missing authored reward field {error.args[0]}') from error
    except (AttributeError, TypeError, ValueError) as error:
        raise ValueError(f'Invalid authored Category or SubType classification: {error}') from error


def attach_reward(obj, reward, expected_class):
    native_class = obj.get('native_class')
    demolition = reward['demolition']
    require(native_class is None or native_class == expected_class, 'Conflicting objective runtime class')
    require((obj.get('demolition') is None or demolition)
            and (obj.get('interaction') is None or not demolition),
            'Conflicting objective runtime class and interaction rules')
    if 'reward' in obj:
        existing = obj['reward']
        require(isinstance(existing, dict) and existing.keys() == reward.keys()
                and all(type(existing[key]) is type(value) and existing[key] == value
                        for key, value in reward.items()),
                'Conflicting existing objective reward')
    else:
        obj['reward'] = reward


def project(objective_raw, metadata_raw):
    objective = parse_document(objective_raw, CATALOG_LIMIT, 'objective catalog')
    metadata = parse_document(metadata_raw, AUTHORED_LIMIT, 'authored metadata')
    mode = objective['game_type']
    require(objective['format'] == 'mag_retail_objective_catalog'
            and type(mode) is int and mode in MODE_CLASSES
            and all(type(objective[key]) is int and objective[key] == expected
                    for key, expected in (('schema_version', 1), ('application_id', 20231),
                                          ('protocol', 1740), ('entity_family', 12), ('network_type', 12))),
            'Expected a retail Acquisition or Domination objective catalog')
    source = objective['source']
    require(source['authored_catalog_sha256'] == sha(metadata_raw), 'Wrong authored metadata SHA256')
    require(source['source_image_sha256'] == RETAIL_SHA, 'Wrong objective catalog source image')
    require(source['capture_qualification'] == 'native_source_gameplay'
            and source.get('initial_load_scope') is None, 'Wrong objective catalog gameplay qualification')
    name = objective['map_name']
    require(isinstance(name, str) and name, 'Invalid objective catalog map name')

    authored_app = metadata.get('application_id')
    require(type(metadata['schema_version']) is int and metadata['schema_version'] == 1
            and ((type(authored_app) is int and authored_app == 20231)
                 or ('application_id' not in metadata and mode == 2
                     and metadata.get('format') == 'mag_retail_domination_map_metadata')),
            'Wrong authored metadata identity')
    mode_enum, target_classes = MODE_CLASSES[mode]
    require(metadata.get('mode_enum', mode_enum) == mode_enum, 'Wrong authored metadata mode')
    targets = source_instances(metadata, mode, name)
    selected = next(row for row in metadata['maps'] if row['map']['map'] == name)
    bindings = unique(objective['objects'], lambda row: token(row['token'], 'objective'),
                      'objective token')
    slots = unique(objective['objects'], lambda row: row['object_index'], 'objective slot')
    require(slots and all(type(index) is int and 0 <= index < 128 for index in slots),
            'Invalid objective slot inventory')
    require(bindings.keys() == targets.keys(), 'Incomplete or unexpected objective token inventory')

    reward_by_token = classifications(selected, target_classes)
    graph = selected['objectives']
    definition_classes = {row['definition_token'].lower(): row['class_name'] for row in graph['definitions']}
    for instance in graph['instances']:
        name_token = token(instance['name_token'], 'instance')
        if name_token in bindings:
            native_class = RUNTIME_CLASSES[definition_classes[instance['definition_token'].lower()]]
            attach_reward(bindings[name_token], reward_by_token[f'0x{name_token:08x}']['reward'], native_class)
    return objective


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('metadata', 'objectives', 'native-elf', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true', help='compare generated bytes with the existing output')
    args = parser.parse_args()
    try:
        inputs = (args.metadata, args.objectives, args.native_elf)
        output = args.output.resolve()
        require(not output.is_relative_to(TOOLS.parents[1]) and output not in {p.resolve() for p in inputs},
                'Output must be outside the public checkout and inputs')
        require(sha(read_bounded(args.native_elf, ELF_LIMIT)) == RETAIL_SHA, 'Wrong original retail ELF')
        result = project(read_bounded(args.objectives, CATALOG_LIMIT),
                         read_bounded(args.metadata, AUTHORED_LIMIT))
        raw = (json.dumps(result, indent=2, allow_nan=False) + '\n').encode()
        if args.check:
            require(read_bounded(output, len(raw)) == raw, 'Objective reward projection differs from reproduction')
        else:
            with args.output.open('xb') as destination:
                destination.write(raw)
    except (AttributeError, KeyError, OSError, TypeError, ValueError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
