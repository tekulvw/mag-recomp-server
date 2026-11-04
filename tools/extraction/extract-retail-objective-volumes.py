#!/usr/bin/env python3
"""Extract authored objective trigger boxes from every retail level archive.

Each ObjectiveTriggerProperties record names one trigger volume instance. The
original engine builds that volume as an oriented box: Translate is the centre,
RotateX/Y/Z are degrees composed as Rx*Ry*Rz for row vectors (the rows returned
by retail_map_helpers.euler_basis), and ScaleX/Y/Z are full edge lengths, so the
half-extents are half of each scale. A point is enclosed when, on every box
axis, its offset from the centre is within the half-extent plus the caller's
padding radius. The uniform Scale field is not used by that construction.

This reads static authored data only. It does not establish which character
reference point or padding the original client uses for the interaction prompt,
the client's facing gate, native trigger-index order, runtime pool IDs or live
acceptance. Output is game-derived data: write it outside the source checkout.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import runpy
import struct

TOOLS = Path(__file__).resolve().parent
FORMAT = 'mag_retail_objective_trigger_volumes'
BOX_CLASS = 'TriggerVolumeBoxProperties'
TRIGGER_CLASS = 'ObjectiveTriggerProperties'
LEVELS = 'PS3_GAME/USRDIR/run/mag/ps3/levels'
CLASS_HEADER = re.compile(rb'\xde\xad\xbe\xef\x00\x00\x00\x03....([A-Za-z_][A-Za-z_0-9:]*)\x00')
LIMITATIONS = [
    'Static authored data only; not live interaction acceptance.',
    'The character reference point and padding used by the original client prompt are not established here.',
    'The original client also applies an approach-angle gate that this catalog does not describe.',
    'Linked triggers are listed in level-file order; native trigger-index order is not established.',
]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def float32(value, label):
    require(type(value) in (int, float) and math.isfinite(value), 'non-finite ' + label)
    return struct.unpack('>f', struct.pack('>f', value))[0]


def box(fields, euler_basis):
    """Project one volume's authored transform; fields maps InstanceProperties names to values."""
    centre = [float32(fields['Translate' + axis], 'Translate' + axis) for axis in 'XYZ']
    rotate = [float32(fields['Rotate' + axis], 'Rotate' + axis) for axis in 'XYZ']
    scale = [float32(fields['Scale' + axis], 'Scale' + axis) for axis in 'XYZ']
    require(all(value > 0 for value in scale), 'trigger box scale must be positive')
    return {'centre': centre, 'rotate_degrees': rotate, 'scale': scale,
            'half_extents': [value * 0.5 for value in scale],
            'axes': [[float(v) for v in row] for row in euler_basis(rotate)]}


def classes_by_token(clv):
    """Map every version-7 property class record token to its class name."""
    result = {}
    for match in CLASS_HEADER.finditer(clv.b):
        header = match.start()
        if header >= 16 and clv.u32(header - 16) == 7:
            token = clv.u32(header - 4)
            name = match[1].decode()
            require(result.get(token, name) == name, 'ambiguous property class token')
            result[token] = name
    return result


def project_level(graph, instances, classes, euler_basis):
    """Join trigger definitions to their volume instances.

    graph is extract-retail-maps objective_graph output; instances are
    InstanceProperties tables; classes maps definition tokens to class names.
    """
    definitions = {row['definition_token']: row for row in graph['definitions']}
    objective_class = {row['name_token']: definitions[row['definition_token']]['class_name']
                       for row in graph['instances']}
    named = {}
    for table in instances:
        fields = {f['name']: f for f in table['fields']}
        name = fields.get('Name', {}).get('token')
        if name is not None:
            named.setdefault(name, []).append(table)
    triggers = []
    for row in graph['instances']:
        definition = definitions[row['definition_token']]
        if definition['class_name'] != TRIGGER_CLASS:
            continue
        volume_token = definition['fields']['ObjectiveTriggerVolume']['token']
        objective_token = definition['fields']['ObjectiveID']['token']
        require(objective_token in objective_class, 'trigger names an unknown objective ' + objective_token)
        matches = named.get(volume_token, [])
        require(len(matches) == 1, 'trigger volume instance missing or ambiguous ' + volume_token)
        volume = matches[0]
        volume_class = classes.get(volume['preceding_token'])
        require(volume_class == BOX_CLASS, 'unsupported trigger volume class %s' % volume_class)
        values = {f['name']: f['value'] for f in volume['fields']}
        trigger_values = {f['name']: f['value'] for f in row['fields']}
        triggers.append({
            'trigger_token': row['name_token'], 'objective_token': objective_token,
            'objective_class': objective_class[objective_token],
            'trigger_type_raw_hex': definition['fields']['TriggerType']['raw_hex'],
            'trigger_position': [float32(trigger_values['Translate' + axis], 'trigger Translate' + axis) for axis in 'XYZ'],
            'volume_token': volume_token, 'volume_class': volume_class,
            'box': box(values, euler_basis),
            'source': {'trigger_definition_offset': definition['offset'],
                       'volume_offset': volume['offset'], 'volume_sha256': volume['source_sha256']}})
    by_objective = {}
    for trigger in triggers:
        by_objective.setdefault(trigger['objective_token'], []).append(trigger)
    return {'objectives': [{'objective_token': token, 'objective_class': rows[0]['objective_class'],
                            'triggers': rows} for token, rows in by_objective.items()],
            'counts': {'objectives': len(by_objective), 'triggers': len(triggers)}}


def configured(game_archive, spawns):
    """Optional map identity annotation from the global settings archive."""
    if game_archive is None:
        return {}
    arc = spawns['Psarc'](game_archive.read_bytes())
    settings = arc.extract(arc.names.index('/run/mag/ps3/settings/gameproperties.zpr') + 1).decode()
    return {row['map']: {k: row[k] for k in ('display_name', 'mode', 'owner_faction')}
            for row in spawns['catalog_settings'](settings)}


def extract(disc, game_archive=None, only=None):
    spawns = runpy.run_path(str(TOOLS / 'extract-retail-spawns.py'))
    maps_tool = runpy.run_path(str(TOOLS / 'extract-retail-maps.py'))
    helpers = runpy.run_path(str(TOOLS / 'retail_map_helpers.py'))
    identity = configured(game_archive, spawns)
    paths = sorted((disc / LEVELS).glob('*.psarc'))
    if only:
        paths = [p for p in paths if p.stem in only]
        require(len(paths) == len(set(only)), 'requested map archive missing')
    require(paths, 'no original retail level archives found')
    maps = []
    for path in paths:
        raw_archive = path.read_bytes()
        archive = spawns['Psarc'](raw_archive)
        member = '/run/mag/ps3/levels/%s/%s.clv' % (path.stem, path.stem)
        data = archive.extract(archive.names.index(member) + 1)
        clv = spawns['retail_clv'](data)
        graph = maps_tool['objective_graph'](data, spawns)
        level = project_level(graph, clv.tables('InstanceProperties'), classes_by_token(clv), helpers['euler_basis'])
        maps.append({'map': path.stem, **identity.get(path.stem, {}),
                     'source': {'archive': LEVELS + '/' + path.name, 'archive_sha256': sha(raw_archive),
                                'clv_member': member, 'clv_sha256': sha(data)},
                     **level})
    return {'schema_version': 1, 'format': FORMAT, 'application_id': 20231,
            'volume_semantics': 'oriented_box_half_scale', 'maps': maps,
            'counts': {'maps': len(maps), 'objectives': sum(m['counts']['objectives'] for m in maps),
                       'triggers': sum(m['counts']['triggers'] for m in maps)},
            'limitations': LIMITATIONS}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--disc', type=Path, required=True)
    parser.add_argument('--game-archive', type=Path, help='Optional global game.psarc; adds map mode/faction identity')
    parser.add_argument('--map', action='append', help='Restrict to this level name; repeatable')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true', help='Require byte-identical existing output; write nothing')
    args = parser.parse_args()
    require(not args.output.resolve().is_relative_to(args.disc.resolve()), 'output must be outside the original disc')
    require(not args.output.resolve().is_relative_to(TOOLS.parents[1]), 'output must be outside the source checkout')
    data = extract(args.disc, args.game_archive, args.map)
    raw = (json.dumps(data, indent=1, allow_nan=False) + '\n').encode()
    if args.check:
        require(args.output.read_bytes() == raw, 'objective volume catalog differs from regeneration')
    else:
        args.output.write_bytes(raw)
    print(json.dumps(data['counts']))


if __name__ == '__main__':
    main()
