#!/usr/bin/env python3
"""Preserve authored skill/attachment requirements as configuration, without unlocking them."""
import argparse
import hashlib
import json
from pathlib import Path
import runpy

ROOT = Path(__file__).resolve().parents[2]


def render(SOURCE, ITEMS):
    raw = SOURCE.read_bytes(); item_bytes = ITEMS.read_bytes()
    parser = runpy.run_path(str(ROOT/'tools/extraction/extract-item-catalog.py'))
    table = parser['normalize'](parser['Parser'](raw.decode(),'character_skills.zpr').parse())['MagCharacterSkillTable']
    catalog = json.loads(item_bytes)
    if catalog['application_id'] != 20230: raise ValueError('prototype item catalog required')
    bindings = {}
    for ordinal,item in catalog['items'].items():
        if item.get('name') and item.get('definition'):
            bindings.setdefault(item['name'],[]).append(int(ordinal))
    # Retain all authored fields, including disabled/reserved rows, prerequisites,
    # faction availability and modifiers. Preserve alias IDs rather than guess.
    result = {'schema_version':1,'application_id':20230,
              'status':'authored research configuration; purchases and modifier application not implemented',
              'source_sha256':{SOURCE.name:hashlib.sha256(raw).hexdigest(),
                               ITEMS.name:hashlib.sha256(item_bytes).hexdigest()},
              'equipment_token_item_ids':bindings,'table':table}
    return (json.dumps(result,indent=2)+'\n').encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--items', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [args.source, args.items]
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
