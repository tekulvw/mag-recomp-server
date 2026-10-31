#!/usr/bin/env python3
"""Export retained authored loadout defaults without guessing stable item IDs."""
import argparse
import hashlib
import json
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def render(SOURCE):
    source_bytes = SOURCE.read_bytes()
    source = json.loads(source_bytes)
    if source['faction'] != 0 or source['class'] != 21 or not 1 <= len(source['loadouts']) <= 5:
        raise ValueError('unsupported retained authored faction/class defaults')
    rows = []
    for ordinal, loadout in enumerate(source['loadouts']):
        if loadout['index'] != ordinal or len(loadout['slots']) != 22:
            raise ValueError('authored loadout ordinal/slot coverage changed')
        items = []
        for slot, item in enumerate(loadout['slots']):
            value = item['item_id']
            if item['slot'] != slot or (value is not None and (type(value) is not int or not 0 <= value <= 1023)):
                raise ValueError('authored loadout item ordinal invalid')
            items.append(-1 if value is None else value)
        update_id = loadout['update_id']
        if type(update_id) is not int or not 0 <= update_id <= 255:
            raise ValueError('authored update ID invalid')
        rows.append({'update_id': update_id, 'items': items})
    result = {'application_id': 20230, 'factions': [{'faction': source['faction'], 'loadouts': rows}],
              'source_sha256': source['source_sha256'],
              'retained_defaults_sha256': hashlib.sha256(source_bytes).hexdigest(),
              'retained_defaults_path': SOURCE.name}
    return (json.dumps(result, indent=2) + '\n').encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [args.source]
    if args.output.resolve() in {v.resolve() for v in inputs}:
        parser.error('output must not replace source inputs')
    payload = render(*inputs)
    if args.check:
        if args.output.read_bytes() != payload:
            raise ValueError('catalog differs from source regeneration')
    else:
        with args.output.open('xb') as stream:
            stream.write(payload)

if __name__ == '__main__':
    main()
