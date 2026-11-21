#!/usr/bin/env python3
"""Add source-bound FRAGO geometry to a private retail objective catalog.

The objective anchors are the current server inputs: an existing interaction
or demolition position, or the authored instance translation. Matching the
native terrain-snap and dynamic placement algorithms is separate work.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import struct

ROOT = Path(__file__).resolve().parents[2]
RETAIL_SHA = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
# Both retail ObjectiveProperties constructors load this f32 before the definition
# property FRAGOBonusRange can replace it, including with an explicit zero.
DEFAULT_RADIUS_VA = 0x1b3be68
CATALOG_LIMIT = 8 * 1024**2
AUTHORED_LIMIT = 64 * 1024**2
ELF_LIMIT = 64 * 1024**2
ELF_HEADER = struct.Struct('>16sHHIQQQIHHHHHH')
ELF_PROGRAM_HEADER = struct.Struct('>IIQQQQQQ')
SECURE = 'ObjectiveSecureAndHoldProperties'
DEMOLITION = 'ObjectiveDemolitionStructuralProperties'
ARRIVAL = 'ObjectiveArrivalProperties'
TRIGGER = 'ObjectiveTriggerProperties'
MODE_CLASSES = {1: ('GameType128', {SECURE, DEMOLITION, ARRIVAL}),
                2: ('GameType256', {SECURE, DEMOLITION})}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def digest(value, label):
    require(isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value),
            f'Invalid {label} SHA256')
    return value


def token(value, label):
    require(isinstance(value, str) and re.fullmatch('0x[0-9a-fA-F]{8}', value),
            f'Invalid {label} token')
    parsed = int(value, 16)
    require(parsed != 0, f'Null {label} token')
    return parsed


def unique(rows, key, label):
    result = {}
    for row in rows:
        binding = key(row)
        require(binding not in result, f'Duplicate {label}')
        result[binding] = row
    return result


def parse_document(raw, limit, label):
    require(len(raw) <= limit, f'{label} exceeds input bound')

    def reject_duplicate_keys(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, f'Duplicate JSON key {key} in {label}')
            result[key] = value
        return result

    return json.loads(raw, object_pairs_hook=reject_duplicate_keys)


def float32(value, label):
    error = f'Invalid {label}: expected finite f32'
    require(type(value) in (float, int), error)
    try:
        rounded = struct.unpack('>f', struct.pack('>f', value))[0]
    except (OverflowError, struct.error) as cause:
        raise ValueError(error) from cause
    require(math.isfinite(rounded), error)
    return rounded


def nonnegative_float32(value, label):
    rounded = float32(value, label)
    require(value >= 0, f'Invalid {label}: expected nonnegative f32')
    return rounded


def authored_float(prop, label):
    require(prop['type'] == 'float', f'Invalid authored {label} type')
    value = float32(prop['value'], label)
    encoded = prop['raw_hex']
    require(isinstance(encoded, str) and encoded.lower() == struct.pack('>f', value).hex(),
            f'Inconsistent authored {label} encoding')
    return value


def elf_load_f32(image, address):
    """Read a big-endian f32 from a uniquely file-backed ELF64 PT_LOAD."""
    require(len(image) >= ELF_HEADER.size, 'Truncated ELF header')
    header = ELF_HEADER.unpack_from(image)
    require(header[0].startswith(b'\x7fELF\x02\x02\x01')
            and header[8] == ELF_HEADER.size, 'Expected big-endian ELF64 header')
    offset, entry_size, count = header[5], header[9], header[10]
    require(entry_size == ELF_PROGRAM_HEADER.size and count > 0
            and ELF_HEADER.size <= offset <= len(image) - count * entry_size,
            'Invalid ELF program header table')
    matches = []
    for row in range(count):
        kind, _, file_offset, virtual, _, file_size, memory_size, _ = (
            ELF_PROGRAM_HEADER.unpack_from(image, offset + row * entry_size)
        )
        if kind != 1:  # PT_LOAD
            continue
        require(file_size <= memory_size and file_offset <= len(image) - file_size,
                'Invalid ELF load segment')
        relative = address - virtual
        if 0 <= relative <= file_size - 4:
            matches.append(file_offset + relative)
    require(len(matches) == 1, 'Expected one file-backed ELF load segment for the native radius')
    return struct.unpack_from('>f', image, matches[0])[0]


def source_instances(authored, mode, name):
    mode_enum, target_classes = MODE_CLASSES[mode]
    maps = [row for row in authored['maps'] if row['map']['map'] == name]
    require(len(maps) == 1, f'Expected one authored map for {name}')
    selected = maps[0]
    require(selected['map']['mode_enum'] == mode_enum, 'Wrong authored map mode')

    graph = selected['objectives']
    definitions = unique(graph['definitions'],
                         lambda row: token(row['definition_token'], 'definition'),
                         'authored definition token')
    instances = unique(graph['instances'], lambda row: token(row['name_token'], 'instance'),
                       'authored instance token')
    used_definitions = set()
    targets = {}
    for name_token, instance in instances.items():
        definition_token = token(instance['definition_token'], 'definition')
        require(definition_token in definitions, 'Missing authored objective definition token')
        used_definitions.add(definition_token)
        definition = definitions[definition_token]
        cls = definition['class_name']
        require(cls == TRIGGER or cls in target_classes, f'Unsupported authored objective class {cls}')
        if cls in target_classes:
            targets[name_token] = (definition['fields'], instance['fields'])
    require(used_definitions == definitions.keys(), 'Unplaced authored objective definition')
    return targets


def objective_position(obj, placement):
    runtime = []
    for section in ('interaction', 'demolition'):
        rule = obj.get(section)
        if rule is None:
            continue
        position = rule.get('position')
        require(isinstance(position, list) and len(position) == 3, 'Incomplete runtime position')
        runtime.append([float32(value, 'runtime position') for value in position])
    if runtime:
        require(len(runtime) == 1 or runtime[0] == runtime[1], 'Ambiguous runtime position')
        return runtime[0]

    axes = ('TranslateX', 'TranslateY', 'TranslateZ')
    require(all(axis in placement for axis in axes), 'Incomplete authored translation')
    return [authored_float(placement[axis], axis) for axis in axes]


def project(objective_raw, authored_raw, progression_raw, default_radius):
    """Project using a supplied native radius or a reader called on first use."""
    objective = parse_document(objective_raw, CATALOG_LIMIT, 'objective catalog')
    authored = parse_document(authored_raw, AUTHORED_LIMIT, 'authored catalog')
    authored_sha = sha(authored_raw)
    source = objective['source']
    require(source['authored_catalog_sha256'] == authored_sha, 'Wrong authored catalog SHA256')
    require(source['source_image_sha256'] == RETAIL_SHA, 'Wrong objective catalog source image')

    mode = objective['game_type']
    require(type(mode) is int and mode in MODE_CLASSES
            and objective['format'] == 'mag_retail_objective_catalog'
            and objective['schema_version'] == 1 and objective['application_id'] == 20231
            and objective['protocol'] == 0x6cc and objective['entity_family'] == objective['network_type'] == 12,
            'Expected a retail Acquisition or Domination objective catalog')
    name = objective['map_name']
    require(isinstance(name, str) and name, 'Invalid objective catalog map name')

    require(authored['schema_version'] == 1 and (authored.get('application_id') == 20231
            or (mode == 2 and authored.get('format') == 'mag_retail_domination_map_metadata')),
            'Wrong authored catalog identity')
    progression = parse_document(progression_raw, CATALOG_LIMIT, 'progression catalog')
    require(progression['schema_version'] == 1 and progression['application_id'] == 20231,
            'Wrong retail progression catalog identity')
    archive = digest(authored['source_archive_sha256'], 'authored source archive')
    xp_sources = progression['source_sha256']
    require(xp_sources['game.psarc'] == archive, 'Wrong progression source archive')
    digest(xp_sources['experience.zpr'], 'progression experience member')
    vehicle_range = nonnegative_float32(progression['table']['FRAGOBonus_VehicleRange'], 'vehicle range')

    targets = source_instances(authored, mode, name)
    bindings = unique(objective['objects'], lambda row: token(row['token'], 'objective'),
                      'objective token')
    slots = unique(objective['objects'], lambda row: row['object_index'], 'objective slot')
    require(slots and all(type(index) is int and 0 <= index < 128 for index in slots),
            'Invalid objective slot inventory')
    require(bindings.keys() == targets.keys(), 'Incomplete or unexpected objective token inventory')

    geometry = []
    for index, obj in sorted(slots.items()):
        definition, fields = targets[token(obj['token'], 'objective')]
        placement = unique(fields, lambda row: row['name'], 'authored instance field')
        require('FRAGOBonusRange' not in placement, 'Unsupported instance FRAGOBonusRange override')
        if 'FRAGOBonusRange' in definition:
            radius = authored_float(definition['FRAGOBonusRange'], 'FRAGOBonusRange')
            require(radius >= 0, 'Invalid authored FRAGOBonusRange: expected nonnegative f32')
        else:
            if callable(default_radius):
                default_radius = default_radius()
            radius = nonnegative_float32(default_radius, 'native objective radius')
        geometry.append({'object_index': index, 'position': objective_position(obj, placement), 'radius': radius})

    objective['frago'] = {
        'schema_version': 1, 'map_name': name, 'game_type': mode,
        'source': {'authored_catalog_sha256': authored_sha,
                   'progression_catalog_sha256': sha(progression_raw),
                   'source_image_sha256': RETAIL_SHA},
        'vehicle_range': vehicle_range,
        'objectives': geometry,
    }
    return objective


def read_bounded(path, limit):
    require(path.is_file(), f'Expected a regular file: {path}')
    with path.open('rb') as source:
        raw = source.read(limit + 1)
    require(len(raw) <= limit, f'Input exceeds bound: {path}')
    return raw


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('objective-catalog', 'authored-catalog', 'progression-catalog', 'retail-elf', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true', help='compare the generated bytes with the existing output')
    args = parser.parse_args()
    try:
        inputs = (args.objective_catalog, args.authored_catalog, args.progression_catalog, args.retail_elf)
        output = args.output.resolve()
        require(not output.is_relative_to(ROOT) and output not in {path.resolve() for path in inputs},
                'Output must be outside the public checkout and inputs')
        image = read_bounded(args.retail_elf, ELF_LIMIT)
        require(sha(image) == RETAIL_SHA, 'Wrong original retail ELF')

        projected = project(read_bounded(args.objective_catalog, CATALOG_LIMIT),
                            read_bounded(args.authored_catalog, AUTHORED_LIMIT),
                            read_bounded(args.progression_catalog, CATALOG_LIMIT),
                            lambda: elf_load_f32(image, DEFAULT_RADIUS_VA))
        raw = (json.dumps(projected, indent=2, allow_nan=False) + '\n').encode()
        if args.check:
            require(read_bounded(output, len(raw)) == raw, 'FRAGO rules differ from reproduction')
        else:
            with args.output.open('xb') as destination:
                destination.write(raw)
    except (AttributeError, KeyError, OSError, TypeError, ValueError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
