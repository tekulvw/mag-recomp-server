#!/usr/bin/env python3
"""Add each spawn location's authored facing to a projected spawn catalog.

The gameplay runtime sends a respawn's facing as a packed 10-bit yaw:
atan2(forward.x, forward.z) / 2pi * 1024, so 0 faces +z and 256 faces +x.
A location without `yaw` keeps the previous fixed facing 0.

The facing comes from the all-map extractor's authored `rotation_xyz` (degrees),
matched by the location's token. The rotation is composed as Rz * Ry * Rx (X
applied first) and the character's forward is that rotation applied to +z; this
composition matched the client's own SpawnPoint orientations exactly, including
pitched and rolled points, in qualified captures. Positions,
indices, kinds and every other field are left unchanged.
"""
import argparse
import json
import math
from pathlib import Path


def require(condition, message):
    if not condition:
        raise SystemExit(message)


def forward(rotation_xyz):
    """Forward (+z) of the rotation Rz(c) * Ry(b) * Rx(a), angles in degrees."""
    a, b, c = (math.radians(float(v)) for v in rotation_xyz)
    # Rx(a) takes +z to (0, -sin a, cos a); Ry(b) then Rz(c) turn that vector.
    x, y, z = math.cos(a) * math.sin(b), -math.sin(a), math.cos(a) * math.cos(b)
    return (x * math.cos(c) - y * math.sin(c), x * math.sin(c) + y * math.cos(c), z)


def packed_yaw(rotation_xyz):
    require(isinstance(rotation_xyz, list) and len(rotation_xyz) == 3
            and all(isinstance(v, (int, float)) and math.isfinite(v) for v in rotation_xyz),
            'Invalid authored rotation')
    fx, _, fz = forward(rotation_xyz)
    require(math.hypot(fx, fz) > 1e-6, 'Authored facing has no horizontal direction')
    turn = math.atan2(fx, fz) % math.tau
    return round(turn / math.tau * 1024) % 1024


def produce(catalog, source):
    require(catalog.get('format') == 'mag_spawn_catalog', 'Not a spawn catalog')
    name = catalog['active']['map_name']
    maps = [m for m in catalog['maps'] if m['name'] == name]
    sources = [m for m in source['maps'] if m['map']['map'] == name]
    require(len(maps) == 1 and len(sources) == 1, 'Map missing or duplicated: ' + name)
    rotations = {}
    for point in sources[0]['points']:
        token = int(point['name_token'], 16)
        require(token not in rotations, 'Duplicate authored token')
        rotations[token] = point['rotation_xyz']
    result = json.loads(json.dumps(catalog))
    for location in [m for m in result['maps'] if m['name'] == name][0]['locations']:
        require(location['token'] in rotations, f"Location {location['index']} has no authored point")
        yaw = packed_yaw(rotations[location['token']])
        require(location.get('yaw', yaw) == yaw, f"Location {location['index']} has a different yaw")
        location['yaw'] = yaw
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--catalog', type=Path, required=True, help='projected mag_spawn_catalog JSON')
    parser.add_argument('--source', type=Path, required=True,
                        help='all-map spawn extraction (extract-retail-spawns.py output)')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true', help='Fail unless OUTPUT already matches')
    args = parser.parse_args()
    require(args.output.resolve() not in (args.catalog.resolve(), args.source.resolve()),
            'Output must not replace an input')
    value = produce(json.loads(args.catalog.read_bytes()), json.loads(args.source.read_bytes()))
    raw = json.dumps(value, indent=2) + '\n'
    if args.check:
        require(args.output.read_text() == raw, 'Output does not match')
    else:
        require(not args.output.exists(), 'Refusing existing output; use --check')
        args.output.write_text(raw)
    print(json.dumps({'map': value['active']['map_name'],
                      'locations': sum(len(m['locations']) for m in value['maps']
                                       if m['name'] == value['active']['map_name'])}))


if __name__ == '__main__':
    main()
