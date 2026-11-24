#!/usr/bin/env python3
"""Write a collision coverage sidecar bound to one collision world's SHA-256.

The sidecar (format mag_collision_coverage_v1) declares verified physics regions
for bot navigation without changing the reviewed world bytes, so the world hash
and every receipt bound to it stay valid. Writing one is an operator assertion
that the world represents the ground and static obstacles inside each region.
This tool records that assertion; it does not and cannot verify it.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import runpy

PREPARE = runpy.run_path(str(Path(__file__).with_name('prepare.py')))
require = PREPARE['require']


def whole_world_region(world, world_sha256, margin_xz, margin_y):
    """The f32 extent of every world vertex, padded by the given margins."""
    _, report = PREPARE['prepare'](world, world_sha256, reference=True)
    bounds = copy.deepcopy(report['source_geometry_bounds'])
    for axis, margin in ((0, margin_xz), (1, margin_y), (2, margin_xz)):
        bounds['min'][axis] -= margin
        bounds['max'][axis] += margin
    return bounds


def make_sidecar(world, world_sha256, assertion, *, whole_world=False, regions=(), margin_xz=0.0, margin_y=3.0):
    require(isinstance(assertion, str) and assertion.strip(), 'an operator assertion is required')
    require(margin_xz >= 0 and margin_y >= 0, 'margins must be nonnegative')
    boxes = [dict(min=list(r[:3]), max=list(r[3:])) for r in regions]
    if whole_world:
        boxes.insert(0, whole_world_region(world, world_sha256, margin_xz, margin_y))
    require(boxes, 'supply --whole-world or at least one --region')
    statement = f'operator assertion: {assertion.strip()}'
    sidecar = dict(
        format=PREPARE['COVERAGE_FORMAT'],
        map_name=world['metadata']['map_name'],
        world_sha256=world_sha256,
        basis='operator_assertion',
        provenance=[statement, 'written by tools/navmesh/make-coverage-sidecar.py'],
        physics_regions=[dict(min=box['min'], max=box['max'], ground_verified=True,
                              obstacles_verified=True, provenance=[statement]) for box in boxes],
    )
    PREPARE['validate_coverage_sidecar'](sidecar, world_sha256, world['metadata']['map_name'])
    return sidecar


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--world', required=True, type=Path)
    parser.add_argument('--expected-world-sha256', required=True)
    parser.add_argument('--output', required=True, type=Path, help='New sidecar file')
    parser.add_argument('--operator-assertion', required=True,
                        help='Required acknowledgement, recorded verbatim: why the regions are claimed as verified')
    parser.add_argument('--whole-world', action='store_true',
                        help='One region spanning every world vertex, padded by the margins')
    parser.add_argument('--margin-xz', type=float, default=0.0, help='Whole-world horizontal padding, metres')
    parser.add_argument('--margin-y', type=float, default=3.0,
                        help='Whole-world vertical padding, metres (standing clearance)')
    parser.add_argument('--region', action='append', nargs=6, type=float, default=[],
                        metavar=('MIN_X', 'MIN_Y', 'MIN_Z', 'MAX_X', 'MAX_Y', 'MAX_Z'))
    args = parser.parse_args()
    require(not args.output.exists(), 'refusing existing output file')
    require(args.world.is_file() and args.world.stat().st_size <= 64 * 1024 * 1024,
            'world file exceeds existing runtime limit')
    raw = args.world.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    require(digest == args.expected_world_sha256, 'world source hash mismatch')
    sidecar = make_sidecar(json.loads(raw), digest, args.operator_assertion, whole_world=args.whole_world,
                           regions=args.region, margin_xz=args.margin_xz, margin_y=args.margin_y)
    encoded = (json.dumps(sidecar, indent=2, allow_nan=False) + '\n').encode()
    require(len(encoded) <= PREPARE['MAX_COVERAGE_BYTES'], 'sidecar exceeds the 64 KiB limit')
    with args.output.open('xb') as output:
        output.write(encoded)
    print(json.dumps(dict(output=str(args.output), sidecar_sha256=hashlib.sha256(encoded).hexdigest(),
                          world_sha256=digest, regions=sidecar['physics_regions'])))


if __name__ == '__main__':
    main()
