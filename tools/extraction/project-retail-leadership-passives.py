#!/usr/bin/env python3
"""Project disk-backed retail leadership modifiers; does not infer native consumers.

Input is the maintained extract-retail-rules.py output. --check never writes;
normal mode creates a new file exclusively. No game resources are distributed.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MEMBER = 'scripts/character_leadership_abilities.zpr'
KINDS = {'kCharModReloadTime', 'kCharModChemicalDamageTaken', 'kCharModMovementSpeed',
         'kCharModIncapacitatedTime', 'kCharModExplosiveDamageTaken',
         'kCharModAcousticDetectionRange', 'kCharModAutoRepair', 'kCharModDemolitionTime',
         'kCharModDamageTaken', 'kCharModDetectionSpeed', 'kCharModAutoHeal'}

def require(condition, message):
    if not condition: raise ValueError(message)

def number(value):
    require(type(value) in (int, float) and math.isfinite(value), 'nonfinite or nonnumeric authored value')
    return value

def project(raw, native=None):
    require(native is not None, 'independent native consumer witness is required')
    require(native.get('schema_version') == 1 and native.get('application_id') == 20231, 'unsupported native consumer witness')
    require(native.get('retail_elf_sha256') == '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c', 'wrong retail consumer witness')
    policy = native['consumer_constants']
    require(all(type(policy.get(k)) is int and 0 < policy[k] <= 65535 for k in ('refresh_hold_ms', 'removal_grace_ms')), 'invalid native timing constants')
    require(raw.get('schema_version') == 1 and raw.get('application_id') == 20231, 'wrong rules schema/application')
    resource = raw['resources'][MEMBER]
    table = resource['parameters']['CharacterLeadershipAbilityTable']
    maximum = number(table['MaxSeparationRange'])
    require(maximum > 0, 'nonpositive authored separation range')
    levels = []
    source_levels = table['LeadershipAbilityLevels']
    require(set(source_levels) == {f'CharacterLeadershipAbilityLevel{x}' for x in range(4)}, 'unsupported leadership level set')
    for level in range(4):
        row = source_levels[f'CharacterLeadershipAbilityLevel{level}']['CharacterLeadershipAbilityLevel']
        modifiers = row.get('LeadershipModifiers', {})
        require(set(modifiers) == {f'CharacterLeadershipModifier{x}' for x in range(len(modifiers))}, 'noncontiguous leadership modifier order')
        projected = []
        seen = set()
        for ordinal in range(len(modifiers)):
            entry = modifiers[f'CharacterLeadershipModifier{ordinal}']['CharacterLeadershipModifier']
            kind = entry['Modifier']
            require(kind in KINDS and kind not in seen, 'unknown or duplicate leadership modifier')
            seen.add(kind)
            projected.append({'kind': kind, 'value': number(entry['ModifierValue'])})
        levels.append({'level': level, 'name': row['Name'], 'modifiers': projected})
    require(not levels[0]['modifiers'] and all(row['modifiers'] for row in levels[1:]), 'invalid leader/normal modifier rows')
    return {'schema_version': 1, 'application_id': 20231,
            'source': {'member': MEMBER, 'member_sha256': resource['sha256'],
                       'archive_sha256': raw['source_archive_sha256'],
                       'extractor': 'tools/extraction/extract-retail-rules.py',
                       'projector': 'tools/extraction/project-retail-leadership-passives.py'},
            'max_separation_range': maximum,
            'refresh_hold_ms': policy['refresh_hold_ms'], 'removal_grace_ms': policy['removal_grace_ms'], 'levels': levels}

def encoded(value): return (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rules', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--native-witness', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [v for k, v in vars(args).items() if k != "output" and isinstance(v, Path)]
    if args.output.resolve() in {v.resolve() for v in inputs}:
        parser.error("output must not replace source inputs")
    output = encoded(project(json.loads(args.rules.read_text()), json.loads(args.native_witness.read_text())))
    if args.check:
        require(args.output.read_bytes() == output, 'leadership projection differs')
    else:
        with args.output.open('xb') as stream: stream.write(output)
    print(json.dumps({'output': str(args.output), 'bytes': len(output), 'sha256': hashlib.sha256(output).hexdigest(), 'checked': args.check}))

if __name__ == '__main__': main()
