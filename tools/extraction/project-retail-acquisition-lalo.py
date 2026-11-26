#!/usr/bin/env python3
"""Make an Acquisition spawn catalog's LALO drop zones selectable parachute points.

Each Acquisition map authors one attacking platoon's LALO drop zones
(SpawnPoint type 5, category 2, one per squad). They start inactive. Each is the
forward spawn of a passive objective that the defenders' anti-aircraft
structure completes when it is destroyed, and resets when it is repaired. The
gameplay runtime already routes that activation through the structural
bindings. This tool changes only how the spawn catalog describes the points:
`kind: "unsupported"` becomes `kind: "parachute"` with a `parachute` pose, so
the published availability can show them and a respawn can use them.

The pose is the authored drop zone itself:

- `position` stays the authored world position, which is the airborne drop
  height. Nothing is snapped to the terrain.
- `forward` is the authored rotation applied to +z (Rz * Ry * Rx, X first, the
  same composition as add-spawn-facing.py). Each later jumper of the same wave
  starts one spacing further along it.
- `spacing_m` is 10 m, the game's drop spacing for a zone authored with the
  default radius 1. A zone with any other radius is refused.
- `yaw` is the forward's heading in the 10-bit wire convention
  (atan2(x, z) / 2pi * 1024), truncated toward zero.

Every other field is unchanged. The tool refuses a catalog whose drop zones do
not match their authored identity, or that is already projected.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import runpy

FACING = runpy.run_path(str(Path(__file__).with_name('add-spawn-facing.py')))
LALO_TYPE = 5
DROP_CATEGORY = 2
DEFAULT_RADIUS = 1.0
DEFAULT_SPACING_M = 10.0
SOURCE_LIMIT = 8192


def require(condition, message):
    if not condition:
        raise SystemExit(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def drop_pose(rotation_xyz):
    require(isinstance(rotation_xyz, list) and len(rotation_xyz) == 3
            and all(isinstance(v, (int, float)) and math.isfinite(v) for v in rotation_xyz),
            'Invalid authored drop-zone rotation')
    forward = [float(v) for v in FACING['forward'](rotation_xyz)]
    require(abs(sum(v * v for v in forward) - 1.0) < 1e-9, 'Nonunit drop-zone forward')
    require(math.hypot(forward[0], forward[2]) > 1e-6, 'Drop zone has no horizontal heading')
    turn = math.atan2(forward[0], forward[2]) % math.tau
    yaw = int(turn / math.tau * 1024) & 1023
    return {'forward': forward, 'spacing_m': DEFAULT_SPACING_M, 'yaw': yaw}


def source_points(source, name):
    """Accept an all-map extraction ({maps: [...]}) or one map's extraction."""
    maps = source.get('maps', [source])
    rows = [m for m in maps if isinstance(m, dict) and m.get('map', {}).get('map') == name]
    require(len(rows) == 1, 'Authored map missing or duplicated: ' + name)
    points = {}
    for point in rows[0]['points']:
        token = int(point['name_token'], 16)
        require(token not in points, 'Duplicate authored token')
        points[token] = point
    return points


def produce(catalog, source, source_raw=b''):
    require(catalog.get('format') == 'mag_spawn_catalog', 'Not a spawn catalog')
    require((catalog.get('match_profile') or {}).get('game_type') == 1,
            'Require an Acquisition spawn catalog')
    name = catalog['active']['map_name']
    result = json.loads(json.dumps(catalog))
    pools = [m for m in result['maps'] if m['name'] == name]
    require(len(pools) == 1, 'Active map missing or duplicated: ' + name)
    pool = pools[0]
    require(pool.get('runtime_indices_verified') is True, 'Require verified pool order')
    provenance = json.loads(result['source'])
    links = {row['index']: row for row in provenance.get('unsupported_spawn_links', [])}
    authored = source_points(source, name)
    available = {index for squad in pool.get('squads', []) for index in squad['available']}
    available.update(result['active']['available'])
    projected = []
    for location in pool['locations']:
        point = authored.get(location['token'])
        require(point is not None, f"Location {location['index']} has no authored point")
        if point['spawn_type'] != LALO_TYPE:
            continue
        index = location['index']
        link = links.get(index)
        require(location['kind'] == 'unsupported' and location.get('active') is False
                and 'parachute' not in location and index not in available,
                f'Drop zone {index} is not an unprojected inactive point')
        require(link is not None and link['spawn_type'] == LALO_TYPE
                and link['category'] == DROP_CATEGORY and link['user_class'] == 0
                and link['active'] is False and len(link['squad_links']) == 1
                and link['squad_links'][0][0] == 0,
                f'Drop zone {index} differs from its recorded identity')
        require(point['active'] is False and point['user_class'] == 0 and point['team'] == 0
                and point['category'] == DROP_CATEGORY and point['snap_to_terrain'] is False
                and len(point['squad_links']) == 1,
                f'Drop zone {index} is not an attacker squad drop zone')
        require(point['radius'] == DEFAULT_RADIUS, f'Drop zone {index} has a non-default radius')
        require([location['position'][axis] for axis in 'xyz'] == point['position_xyz'],
                f'Drop zone {index} differs from its authored position')
        location['kind'] = 'parachute'
        location['parachute'] = drop_pose(point['rotation_xyz'])
        projected.append(index)
    require(projected, 'Map has no unprojected drop zones')
    provenance['acquisition_lalo'] = {
        'tool': 'tools/extraction/project-retail-acquisition-lalo.py',
        'source_sha256': sha(source_raw), 'points': projected,
        'spacing_m': DEFAULT_SPACING_M}
    result['source'] = json.dumps(provenance, sort_keys=True, separators=(',', ':'),
                                  allow_nan=False)
    require(len(result['source'].encode()) <= SOURCE_LIMIT, 'Spawn source exceeds bound')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--catalog', type=Path, required=True,
                        help='projected Acquisition mag_spawn_catalog JSON')
    parser.add_argument('--source', type=Path, required=True,
                        help='spawn extraction (extract-retail-spawns.py all-map or per-map output)')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true', help='Fail unless OUTPUT already matches')
    args = parser.parse_args()
    require(args.output.resolve() not in (args.catalog.resolve(), args.source.resolve()),
            'Output must not replace an input')
    source_raw = args.source.read_bytes()
    value = produce(json.loads(args.catalog.read_bytes()), json.loads(source_raw), source_raw)
    raw = json.dumps(value, indent=2) + '\n'
    if args.check:
        require(args.output.read_text() == raw, 'Output does not match')
    else:
        require(not args.output.exists(), 'Refusing existing output; use --check')
        args.output.write_text(raw)
    pool = next(m for m in value['maps'] if m['name'] == value['active']['map_name'])
    print(json.dumps({'map': pool['name'], 'parachute': [
        p['index'] for p in pool['locations'] if p['kind'] == 'parachute']}))


if __name__ == '__main__':
    main()
