#!/usr/bin/env python3
# Adapted project-owned projector; precursor source SHA256: 5bd66a3f8446594083ee091727646c4f13d44fd9b857ad9170fb6ce1b1324f63
"""Project native character blast inputs from exact retained original evidence.

Offline only. Default mode creates a new configuration exclusively; --check
compares without writing. --retain-dev-literal reproduces the bounded original
DEV literal witness from the hash-checked ELF, without execution or disassembly.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys

DEV_SHA256 = 'efea4d5ef0d14915c75382a56c23f5006fd6916b4080b8a386de01603358a09d'
RETAIL_SHA256 = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
WITNESS_SHA256 = {
    'comparison.json': 'a6cf6066606612e394d4173d9ac156eaff625715b5411714a0c622433ecfb078',
    'blast-callback.json': '07aac7addf8902cf63877cd87b30d42173159a2426a9165e0e544664e7e42858',
    'blast-armor-literal.json': 'dab57aaeded4ac5cf0c16ce59561d5c7868f624893f608247ba4cb8cc669f30f',
    'toc-cells.json': 'e361fd30bfb1b7c36793fed151a95167cf1d9fd49f9f4c68ff492ab75ca3e336',
    'dev/dev-blast-armor-literal.json': 'fb85dbaee1c8d61d9577d3dfbdb6613cdc6b04c0f8564bf3a01903afbc3aecb2',
}


def check(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    return (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()


def retain_dev_literal(path):
    """Retain only one four-byte file-backed original DEV literal cell."""
    raw = path.read_bytes()
    check(sha(raw) == DEV_SHA256, 'wrong_original_dev_elf')
    check(raw[:7] == b'\x7fELF\x02\x02\x01'
          and struct.unpack_from('>H', raw, 18)[0] == 21, 'wrong_dev_elf_encoding')
    offset = struct.unpack_from('>Q', raw, 32)[0]
    size, count = struct.unpack_from('>HH', raw, 54)
    check(size >= 56 and offset + size * count <= len(raw), 'invalid_program_headers')
    matches = []
    address = 0x01b9cef4
    for index in range(count):
        kind, flags, file_offset, base, _, length, memory_length, _ = struct.unpack_from(
            '>IIQQQQQQ', raw, offset + index * size)
        if kind == 1 and base <= address and address + 4 <= base + length:
            check(length <= memory_length and file_offset + length <= len(raw), 'invalid_segment')
            position = file_offset + address - base
            matches.append({'address': f'{address:08x}', 'file_offset': position,
                            'segment_index': index, 'segment_flags': flags,
                            'raw_hex': raw[position:position + 4].hex(),
                            'value_be': int.from_bytes(raw[position:position + 4], 'big')})
    check(len(matches) == 1, 'wrong_dev_literal_mapping')
    return {'schema_version': 1, 'source_build': 'dev', 'original_elf_sha256': DEV_SHA256,
            'width': 4, 'total_read_bytes': 4, 'reads': matches,
            'provenance': {'source': 'Exact hash-checked original ELF file bytes',
                           'mapping': 'ELF64 big-endian PPC64 unique file-backed PT_LOAD',
                           'extractor': 'gameplay/tools/project-character-blast-rules.py --retain-dev-literal'},
            'limits': ['File bytes only; no runtime memory or live pointer-value proof.',
                       'The original checker finding supplies the DEV callback/TOC association.']}


def project(inputs, checker_path):
    """Accept only the exact reviewed witnesses, independently checking local bytes."""
    check(set(inputs) == set(WITNESS_SHA256), 'incomplete_witness_set')
    for name, digest in WITNESS_SHA256.items():
        check(sha(inputs[name]) == digest, 'unreviewed_witness:' + name)
    records = {name: json.loads(raw) for name, raw in inputs.items()}
    report = records['comparison.json']
    check(report['dev_elf_sha256'] == DEV_SHA256 and report['retail_elf_sha256'] == RETAIL_SHA256,
          'wrong_original_builds')
    check(sha(checker_path.read_bytes()) == report['checker_sha256'], 'unreviewed_original_checker')
    callback = records['blast-callback.json']
    check(callback['original_elf_sha256'] == RETAIL_SHA256, 'wrong_callback_build')
    rows = callback['instructions']
    words = {int(row['address'], 16): int(row['word'], 16) for row in rows}
    start, end = int(callback['start'], 16), int(callback['end_exclusive'], 16)
    check(len(words) == len(rows) and list(words) == list(range(start, end, 4)), 'invalid_instruction_addresses')
    raw = b''.join(word.to_bytes(4, 'big') for word in words.values())
    check(sha(raw) == callback['instruction_bytes_sha256'], 'invalid_instruction_bytes')
    # Exact witness SHA256 pins above qualify the callback; no instruction words are embedded.

    def literal(name, address, expected_build):
        record = records[name]
        check(record['original_elf_sha256'] == expected_build and record['width'] == 4,
              'wrong_literal_build_or_width')
        matching = [row for row in record['reads'] if row['address'] == f'{address:08x}']
        check(len(matching) == 1, 'missing_literal_cell')
        cell = bytes.fromhex(matching[0]['raw_hex'])
        check(len(cell) == 4 and int.from_bytes(cell, 'big') == matching[0]['value_be'], 'wrong_literal_bytes')
        return struct.unpack('>f', cell)[0]

    toc = int(report['blast_prepared_armor_inputs']['retail_literal']['interpreted_toc'], 16)
    rating = literal('toc-cells.json', toc + (words[0x108b7dc] & 0xffff), RETAIL_SHA256)
    prevention = literal('blast-armor-literal.json', toc + (words[0x108b960] & 0xffff), RETAIL_SHA256)
    check(literal('dev/dev-blast-armor-literal.json', 0x1b9cef4, DEV_SHA256) == prevention,
          'dev_retail_literal_mismatch')
    damage = {'armor_rating_reduction': rating, 'armor_prevention_reduction': prevention,
              'hit_location': words[0x108b968] & 0xffff, 'damage_type': words[0x108b93c] & 0xffff}
    finding = report['blast_prepared_armor_inputs']
    expected = {key: finding[name] for key, name in (
        ('armor_rating_reduction', 'prepared_rating_reduction'),
        ('armor_prevention_reduction', 'prepared_prevention_reduction'),
        ('hit_location', 'prepared_hit_location'), ('damage_type', 'prepared_damage_type'))}
    check(damage == expected, 'original_checker_disagreement')
    retained = {row['path'].split('/')[-1]: row['sha256'] for row in report['witnesses']}
    check(all(retained[name] == WITNESS_SHA256[name] for name in (
        'blast-callback.json', 'blast-armor-literal.json', 'toc-cells.json')), 'checker_witness_disagreement')
    return {'schema_version': 1, 'applications': [20230, 20231], 'character_damage': damage,
            'provenance': {
                'scope': finding['scope'], 'original_dev_elf_sha256': DEV_SHA256,
                'original_retail_elf_sha256': RETAIL_SHA256,
                'original_checker': {'path': 'tools/extraction/check-retail-combat-native.py',
                                     'sha256': report['checker_sha256'],
                                     'finding': 'blast_prepared_armor_inputs'},
                'witnesses': [{'path': 'external-witness/' + name,
                               'sha256': digest} for name, digest in WITNESS_SHA256.items()],
                'retail_callback': {'address': callback['start'], 'instruction_bytes_sha256': sha(raw)},
                'retail_literal': finding['retail_literal'], 'dev_literal': finding['dev_literal'],
                'projector_sha256': sha(Path(__file__).read_bytes())},
            'limitations': [
                'These are callback-prepared character DamageData inputs before downstream virtual hooks.',
                'Authored characterARedux remains unchanged and is distinct from the native callback input.',
                'TOC associations use the retained original checker finding; file cells corroborate that interpretation.',
                'This projection does not establish collision, cover, damage admission, modifiers, normalization, runtime ammo aliases or live acceptance.',
                'Existing retail combat authority gates are not changed by this configuration.']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path, help='qualified external witness bundle; acquisition/checker not supplied')
    parser.add_argument('--checker', type=Path, help='external original checker source matching comparison.json')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--retain-dev-literal', action='store_true')
    parser.add_argument('--dev-elf', type=Path)
    args = parser.parse_args()
    output = args.output
    if args.retain_dev_literal and args.dev_elf is None:
        parser.error('--retain-dev-literal requires --dev-elf')
    if not args.retain_dev_literal and (args.evidence is None or args.checker is None):
        parser.error('projection requires --evidence and --checker')
    try:
        if args.retain_dev_literal:
            check(output.resolve() != args.dev_elf.resolve(), 'input_output_collision')
            result = retain_dev_literal(args.dev_elf)
        else:
            paths = {name: args.evidence / name for name in WITNESS_SHA256}
            check(all(output.resolve() != path.resolve() for path in [*paths.values(), args.checker]), 'input_output_collision')
            result = project({name: path.read_bytes() for name, path in paths.items()}, args.checker)
        encoded = encode(result)
        if args.check:
            check(output.read_bytes() == encoded, 'projection_differs')
        else:
            with output.open('xb') as stream:
                stream.write(encoded)
    except (ValueError, OSError, KeyError, TypeError, struct.error) as exc:
        print(json.dumps({'projected': False, 'failure': str(exc)}), file=sys.stderr)
        return 1
    print(json.dumps({'projected': True, 'check': args.check, 'output': str(output)}))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
