#!/usr/bin/env python3
"""Read the authored incapacitation threshold and bleed-out time from the retail archive.

The two character gameplay scalars become the gameplay server's
`--incapacitation-rules` input. Physical-state exclusions are not authored in
that resource, so the output lists none; supply them only from separate
qualified evidence. No game execution, native code or runtime capture is used.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import runpy
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'collision'))
from archive import Psarc

ARCHIVE_SHA256 = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'
MEMBER = '/run/mag/ps3/scripts/character_gameplay.zpr'
FIELDS = {
    'incapacitated_damage_threshold_hp': 'fIncapacitatedDamageThreshold',
    'incapacitated_duration_seconds': 'fIncapacitatedTime',
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def project(member):
    """Return rules from one parsed member; each named scalar must occur exactly once."""
    parser = runpy.run_path(str(Path(__file__).with_name('extract-item-catalog.py')))
    resource = parser['normalize'](parser['Parser'](member.decode('utf-8-sig'), 'character_gameplay.zpr').parse())

    def authored(name):
        found = []

        def walk(value, path):
            if isinstance(value, dict):
                for key, child in value.items():
                    if key == name:
                        found.append((path + '/' + key, child))
                    walk(child, path + '/' + key)
            elif isinstance(value, list):
                for index, child in enumerate(value):
                    walk(child, path + '/' + str(index))
        walk(resource, '')
        if (len(found) != 1 or type(found[0][1]) not in (int, float)
                or not math.isfinite(found[0][1]) or found[0][1] <= 0):
            raise ValueError('Require one finite positive authored scalar ' + name)
        return found[0]

    rules = {'schema_version': 1}
    pointers = {}
    for output, name in FIELDS.items():
        pointer, value = authored(name)
        rules[output] = float(value)
        pointers[output] = pointer
    rules['excluded_character_states'] = []
    rules['provenance'] = {
        'game_archive_sha256': ARCHIVE_SHA256, 'member': MEMBER, 'member_sha256': sha(member),
        'json_pointers': pointers,
        'excluded_character_states': 'not authored in this member; none supplied',
        'projector_sha256': sha(Path(__file__).read_bytes()),
    }
    return rules


def produce(archive_path):
    raw = archive_path.read_bytes()
    if sha(raw) != ARCHIVE_SHA256:
        raise ValueError('Original retail archive SHA256 mismatch')
    archive = Psarc(raw)
    if MEMBER not in archive.names:
        raise ValueError('Missing character gameplay member')
    return project(archive.extract(archive.names.index(MEMBER) + 1))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        if args.output.resolve() == args.game_archive.resolve():
            raise ValueError('input_output_collision')
        payload = (json.dumps(produce(args.game_archive), indent=2, allow_nan=False) + '\n').encode()
        if args.check:
            if args.output.read_bytes() != payload:
                raise ValueError('incapacitation_rules_differ')
        else:
            with args.output.open('xb') as stream:
                stream.write(payload)
    except (ValueError, OSError) as exc:
        print(json.dumps({'projected': False, 'failure': str(exc)}), file=sys.stderr)
        return 1
    print(json.dumps({'projected': True, 'check': args.check, 'output': str(args.output)}))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
