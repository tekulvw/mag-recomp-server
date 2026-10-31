#!/usr/bin/env python3
"""Project the native squad-leader election policy from extracted retail rules.

Weights come from scripts/mag_squad_matchmaking_properties.zpr as retained by
extract-retail-rules.py. The default record is local configuration for a
character whose saved leadership fields Horizon has not forwarded: native
MagNetCharacterRecord::Clear supplies leadership level 1; squad nomination 1 and
zero matches-since-last-command are explicit local choices, not native evidence.
--check never writes; normal mode creates a new file exclusively.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MEMBER = 'scripts/mag_squad_matchmaking_properties.zpr'

def require(condition, message):
    if not condition: raise ValueError(message)

def weight(table, key):
    value = table.get(key)
    require(type(value) in (int, float) and math.isfinite(value) and value >= 0, f'invalid authored {key}')
    return float(value)

def project(raw, default_record):
    require(raw.get('schema_version') == 1 and raw.get('application_id') == 20231, 'wrong rules schema/application')
    resource = raw['resources'][MEMBER]
    table = resource['parameters']['MagSquadMatchmakingConfigSPropertyTable']
    sha = resource['sha256']
    require(len(sha) == 64 and all(c in '0123456789abcdef' for c in sha), 'invalid member hash')
    return {'schema_version': 2, 'application_id': 20231, 'squad_leader': 'native',
            'native_election': {'source_sha256': sha,
                                'squad_level_weight': weight(table, 'SquadLeader_LevelWeight'),
                                'squad_mslc_weight': weight(table, 'SquadLeader_MSLCWeight'),
                                'default_record': default_record},
            'development_bindings': []}

def encoded(value): return (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rules', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--default-record', type=Path, required=True, help='explicit local leadership fallback policy JSON')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    record = json.loads(args.default_record.read_text())
    require(set(record) == {'leadership_level', 'nominations', 'matches_since_last_command'}, 'unexpected fallback policy fields')
    require(all(type(v) is int and 0 <= v <= 2**32-1 for v in record.values()), 'invalid fallback policy values')
    require(args.output.resolve() not in {args.rules.resolve(), args.default_record.resolve()}, 'output would replace input')
    output = encoded(project(json.loads(args.rules.read_text()), record))
    if args.check:
        require(args.output.read_bytes() == output, 'role policy projection differs')
    else:
        with args.output.open('xb') as stream: stream.write(output)
    print(json.dumps({'output': str(args.output), 'bytes': len(output), 'sha256': hashlib.sha256(output).hexdigest(), 'checked': args.check}))

if __name__ == '__main__': main()
