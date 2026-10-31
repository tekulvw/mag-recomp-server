#!/usr/bin/env python3
"""Convert retail map collision to neutral files using the maintained map manifest.

This retail-only entry point needs neither a prototype ZIP nor mag_decomp outputs.
Resource-local vertices/triangles are offline extraction, not assembled world data.
Never emits native Havok binaries; the game server must not consume originals.
"""
import argparse
import json
from pathlib import Path

from catalog import Catalog, dump, file_sha, sha

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--disc', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='New private bulk-output directory')
    parser.add_argument('--only-map')
    parser.add_argument('--summary-output', type=Path, help='Optional compact metadata report to track in Git')
    args = parser.parse_args()
    if args.output.resolve().is_relative_to(args.disc.resolve()) or (args.summary_output and args.summary_output.resolve().is_relative_to(args.disc.resolve())):
        raise ValueError('Outputs must be outside the original disc tree')
    manifest = json.loads(args.manifest.read_text())
    if manifest['application_id'] != 20231 or manifest['schema_version'] != 1:
        raise ValueError('Expected maintained retail map schema1')
    selected = [m for m in manifest['maps'] if not args.only_map or m['map']['map'] == args.only_map]
    if not selected:
        raise ValueError('No matching packaged retail map')
    if args.output.exists():
        raise ValueError('Refusing to reuse a bulk extraction directory')
    # Validate every source before creating any output.
    for row in selected:
        relative = Path(row['source']['archive'])
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('Expected disc-relative original archive path')
        if file_sha(args.disc / relative) != row['source']['archive_sha256']:
            raise ValueError('Original level archive differs from pinned metadata: ' + str(relative))
    args.output.mkdir(parents=True)
    catalog = Catalog(args.output, retain_raw=False, include_instance_evidence=False)
    maps = []
    for row in selected:
        relative = row['source']['archive']
        maps.append(catalog.map((args.disc / relative).read_bytes(), 'retail-bcus98110-01.00',
                                row['map']['map'], {'archive': relative}, row['source']['archive_sha256']))
    totals = {'maps': len(maps), 'unique_packfiles': len(catalog.known),
              'unique_resource_members': len(catalog.resources),
              'converted_mesh_references': sum(m['counts']['converted_meshes'] for m in maps),
              'triangle_references': sum(m['counts']['triangles'] for m in maps),
              'unsupported_object_references': sum(m['counts']['unsupported_objects'] for m in maps),
              'unsupported_collision_object_references': sum(m['counts']['unsupported_collision_objects'] for m in maps)}
    limitations = ['Resource-local neutral conversion only; world placement, terrain identity, materials/filters, hierarchy and dynamic state remain unresolved.',
                   'Shared main.psarc resources are not scanned by this map-only entry point; full shared inventory is a separate required boundary.',
                   'No retail physics-world query or game-server integration acceptance is claimed.',
                   'Only .zrb/.hsx/.hkx/.hkt members are scanned; excluded members and invalid candidates remain in each map report.']
    report = {'format': 'mag_collision_inventory', 'schema_version': 1, 'application_id': 20231,
              'source_manifest_sha256': file_sha(args.manifest), 'only_map': args.only_map,
              'source_manifests': {str(args.manifest.resolve()): file_sha(args.manifest)},
              'tool_sha256': {name: file_sha(ROOT / name) for name in ('tools/collision/catalog.py', 'tools/collision/archive.py', 'tools/collision/packfile.py', 'tools/collision/extract-retail.py', 'tools/collision/zrb_records.py')},
              'maps': maps, 'shared_sources': [], 'totals': totals,
              'neutral_geometry': 'explicit vertices/triangles only; no native Havok payloads', 'limits': limitations}
    dump(args.output / 'coverage.json', report)
    if args.summary_output:
        summary = {'schema_version': 1, 'application_id': 20231,
                   'source_manifest_sha256': file_sha(args.manifest), 'totals': totals,
                   'maps': [{'map': m['map'], 'source': m['source'], 'counts': m['counts'],
                             'manifest': m['manifest'], 'world_assembly': m['world_assembly']} for m in maps],
                   'geometry_outputs': len(catalog.known), 'coverage_complete': False, 'limitations': limitations}
        args.summary_output.write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps({'coverage': str(args.output / 'coverage.json'), 'totals': totals}), flush=True)


if __name__ == '__main__':
    main()
