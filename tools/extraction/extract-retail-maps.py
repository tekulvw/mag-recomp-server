#!/usr/bin/env python3
"""Extract portable all-map retail spawn, bunker-exit and objective metadata.

No native Havok/PS3 resources are emitted. Objective property bytes remain typed
raw authored values where semantics are unknown; no runtime pool IDs are guessed.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import runpy
import struct

ROOT = Path(__file__).resolve().parents[2]
TOOLS = Path(__file__).resolve().parent
GAME_SHA = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def objective_graph(data, helpers):
    clv = helpers['retail_clv'](data)
    definitions = {}
    marker = rb'\xde\xad\xbe\xef\x00\x00\x00\x03....(Objective[A-Za-z_0-9]*Properties)\x00'
    for match in re.finditer(marker, data):
        header = match.start()
        if header < 16 or clv.u32(header - 16) != 7:
            continue
        classname, cursor = clv.string(header + 8)
        end, token = clv.u32(header - 8), clv.u32(header - 4)
        if end > len(data) or end <= cursor or token in definitions:
            raise ValueError('Invalid/duplicate objective definition boundary')
        fields = {}
        while cursor < end and data[cursor] == 1:
            name, cursor = clv.string(cursor + 1)
            kind, cursor = clv.string(cursor)
            size = clv.u32(cursor)
            if name in fields or size < 4 or cursor + size > end:
                raise ValueError('Invalid objective field boundary/identity')
            raw = data[cursor + 4:cursor + size]
            row = {'type': kind, 'raw_hex': raw.hex(), 'offset': cursor + 4}
            if kind in ('uint32', 'int32') and len(raw) == 4:
                row['value'] = int.from_bytes(raw, 'big', signed=kind == 'int32')
            elif kind == 'float' and len(raw) == 4:
                row['value'] = struct.unpack('>f', raw)[0]
                if not math.isfinite(row['value']):
                    raise ValueError('Nonfinite objective scalar')
            elif kind == 'bool' and len(raw) == 1:
                if raw[0] not in (0, 1):
                    raise ValueError('Invalid objective bool')
                row['value'] = bool(raw[0])
            elif kind in ('util::Token', 'util::LocalizedString') and len(raw) == 4:
                row['token'] = '0x' + raw.hex()
            fields[name] = row
            cursor += size
        if cursor + 1 != end or data[cursor] != 0:
            raise ValueError('Invalid objective record terminator')
        definitions[token] = {'definition_token': f'0x{token:08x}', 'class_name': classname,
                              'fields': fields, 'offset': header - 16, 'end': end,
                              'source_sha256': sha(data[header - 16:end]), 'runtime_pool_id': None}
    instances, all_instances = [], clv.tables('InstanceProperties')
    for inst in all_instances:
        if inst['preceding_token'] in definitions:
            fields = {f['name']: f['value'] for f in inst['fields']}
            instances.append({'name_token': fields['Name'], 'definition_token': f"0x{inst['preceding_token']:08x}",
                              'fields': inst['fields'], 'source': helpers['evidence'](inst), 'runtime_pool_id': None})
    if len({i['name_token'] for i in instances}) != len(instances):
        raise ValueError('Duplicate objective instance identity')
    return {'definitions': list(definitions.values()), 'instances': instances,
            'scope': 'All version7 Objective*Properties records and directly linked InstanceProperties records; script-created objects and runtime IDs unresolved'}


def extract(disc, game_archive):
    x = runpy.run_path(str(TOOLS / 'extract-retail-spawns.py'))
    h = runpy.run_path(str(TOOLS / 'retail_map_helpers.py'))
    game = game_archive.read_bytes()
    if sha(game) != GAME_SHA:
        raise ValueError('Wrong original retail game.psarc')
    arc = x['Psarc'](game)
    def member(name):
        return arc.extract(arc.names.index('/run/mag/ps3/' + name) + 1)
    configured = x['catalog_settings'](member('settings/gameproperties.zpr').decode())
    by_name = {row['map']: row for row in configured}
    for faction, level in re.findall(r'TrainingMission(FactionType\w+)\s*=\s*"([^"]+)"', member('scripts/training_missions.zpr').decode()):
        by_name[level] = {'map': level, 'display_name': 'Training', 'owner_faction': x['FACTIONS'][faction],
                         'owner_faction_enum': faction, 'mode': 'Training', 'mode_enum': 'TrainingMission',
                         'faction_basis': 'Configured faction Training level; team assignment is not inferred'}
    registry = h['vehicle_registry'](h['parse_zpr'](member('scripts/vehicles.zpr').decode()))
    properties, sources = {}, {}
    for name in arc.names:
        if name.startswith('/run/mag/ps3/scripts/vehicle/') and (name.endswith('_defensive_emplacement.zpr') or name.endswith('/red_defensive_emplacement_no_turret.zpr')):
            content = arc.extract(arc.names.index(name) + 1)
            properties[Path(name).name] = h['dismount_properties'](h['parse_zpr'](content.decode()))
            sources[name] = {'bytes': len(content), 'sha256': sha(content)}
    paths = sorted((disc / 'PS3_GAME/USRDIR/run/mag/ps3/levels').glob('*.psarc'))
    if not paths:
        raise ValueError('No original retail level archives found')
    maps = []
    for path in paths:
        if path.stem not in by_name:
            raise ValueError('Physical map missing authored settings: ' + path.name)
        row = x['extract_map'](path, by_name[path.stem])
        row['source']['archive'] = 'PS3_GAME/USRDIR/run/mag/ps3/levels/' + path.name
        row['runtime_pool_indices_verified'] = False
        level = x['Psarc'](path.read_bytes())
        data = level.extract(level.names.index(row['source']['clv_member']) + 1)
        graph = objective_graph(data, h)
        links = h['objective_links'](data)
        bunkers = []
        for point in row['points']:
            bunker = h['catalog_point'](point, registry, properties)
            if bunker:
                bunker['objective_links'] = links.get(point['name_token'], [])
                bunkers.append(bunker)
        row['bunkers'] = bunkers
        row['objectives'] = graph
        maps.append(row)
    return {'schema_version': 1, 'application_id': 20231, 'source_archive_sha256': GAME_SHA,
            'property_sources': sources, 'maps': maps,
            'configured_but_not_packaged': sorted(set(by_name) - {p.stem for p in paths}),
            'counts': {'maps': len(maps), 'spawns': sum(len(m['points']) for m in maps),
                       'bunkers': sum(len(m['bunkers']) for m in maps),
                       'exit_candidates': sum(len(b['candidates']) for m in maps for b in m['bunkers']),
                       'objective_definitions': sum(len(m['objectives']['definitions']) for m in maps),
                       'objective_instances': sum(len(m['objectives']['instances']) for m in maps)},
            'limitations': ['Static authored data only; not runtime activation, replication pool ordering or playable acceptance.',
                            h['TRANSFORM_ASSUMPTION'], 'All exit floor/clearance/runtime-selection fields remain unknown.',
                            'Objective enum meaning, script behavior and dynamic creation remain separate analysis.',
                            'Collision shapes, world placement, terrain snapping and dynamic destruction are not supplied by this metadata catalog.']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--disc', type=Path, required=True)
    parser.add_argument('--game-archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true', help='Regenerate in memory; never write output')
    args = parser.parse_args()
    if args.output.resolve() == args.game_archive.resolve() or args.output.resolve().is_relative_to(args.disc.resolve()):
        raise ValueError('Output must be outside the original disc and cannot replace the archive')
    data = extract(args.disc, args.game_archive)
    raw = (json.dumps(data, indent=2, allow_nan=False) + '\n').encode()
    if args.check:
        if args.output.read_bytes() != raw:
            raise ValueError('Retail map catalog differs from original regeneration')
    else:
        args.output.write_bytes(raw)
    print(json.dumps(data['counts']))


if __name__ == '__main__':
    main()
