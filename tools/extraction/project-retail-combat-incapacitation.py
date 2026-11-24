#!/usr/bin/env python3
"""Add authored incapacitated body multipliers to an existing projected combat catalog.

A combat catalog projected before body rows carried `incapacitated_multiplier`
cannot enable `--incapacitation-rules`. This tool reads each row's authored
downed-damage multiplier from the exact item catalog the combat catalog names in
`provenance.catalog_sha256`, checks that the row's living multiplier and armor
area still match the same authored body entry, and writes a new catalog. Every
other value is kept unchanged. It never invents a multiplier.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import sys

BODY_RESOURCE = 'collision_info.lua'
BODY_POINTER = '/parameters/CharacterCollisionData/CharacterStyles/CharacterStyle0/CharacterStyle/m_bodyData'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def finite(value, minimum=0.0):
    return type(value) in (int, float) and math.isfinite(value) and value >= minimum


def project(combat_raw, items_raw):
    combat = json.loads(combat_raw)
    items = json.loads(items_raw)
    require(type(combat) is dict and type(items) is dict, 'malformed_input')
    provenance = combat.get('provenance')
    require(type(provenance) is dict and provenance.get('catalog_sha256') == sha(items_raw),
            'item_catalog_does_not_match_combat_provenance')
    try:
        bodies = items['resources'][BODY_RESOURCE]['parameters']['CharacterCollisionData'][
            'CharacterStyles']['CharacterStyle0']['CharacterStyle']['m_bodyData']
    except (KeyError, TypeError) as exc:
        raise ValueError('missing_authored_body_data') from exc
    rows = combat.get('body')
    require(type(rows) is list and len(rows) == 16, 'requires_16_body_rows')
    require(type(bodies) is dict and set(bodies) == {f'CollisionBodyInfo{i}' for i in range(16)},
            'unexpected_authored_body_count')
    seen = set()
    for row in rows:
        require(type(row) is dict and type(row.get('location')) is int and 0 <= row['location'] < 16
                and row['location'] not in seen, 'invalid_body_location')
        location = row['location']
        seen.add(location)
        source = row.get('source')
        require(type(source) is dict and source.get('file') == BODY_RESOURCE
                and source.get('pointer') == f'{BODY_POINTER}/CollisionBodyInfo{location}/CollisionBodyInfo',
                'body_row_source_mismatch')
        authored = bodies[f'CollisionBodyInfo{location}']
        require(type(authored) is dict and type(authored.get('CollisionBodyInfo')) is dict,
                'malformed_authored_body')
        authored = authored['CollisionBodyInfo']
        require(authored.get('armorArea') == row.get('armor_area')
                and authored.get('hitpointMult') == row.get('multiplier'),
                'living_body_row_differs_from_authored_entry')
        value = authored.get('incapMult')
        require(finite(value), 'invalid_authored_incapacitated_multiplier')
        if 'incapacitated_multiplier' in row:
            require(row['incapacitated_multiplier'] == value, 'existing_incapacitated_multiplier_differs')
        row['incapacitated_multiplier'] = value
    provenance['incapacitated_multiplier_source'] = {
        'item_catalog_sha256': sha(items_raw), 'resource': BODY_RESOURCE,
        'pointer': BODY_POINTER, 'field': 'incapMult',
        'input_combat_sha256': sha(combat_raw),
        'projector_sha256': sha(Path(__file__).read_bytes()),
    }
    return combat


def encode(value):
    return (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--combat', type=Path, required=True)
    parser.add_argument('--item-catalog', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        require(args.output.resolve() not in {args.combat.resolve(), args.item_catalog.resolve()},
                'input_output_collision')
        result = encode(project(args.combat.read_bytes(), args.item_catalog.read_bytes()))
        if args.check:
            require(args.output.read_bytes() == result, 'projection_differs')
        else:
            fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, 'wb') as stream:
                stream.write(result)
    except (ValueError, OSError) as exc:
        print(json.dumps({'projected': False, 'failure': str(exc)}), file=sys.stderr)
        return 1
    print(json.dumps({'projected': True, 'check': args.check, 'output': str(args.output)}))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
