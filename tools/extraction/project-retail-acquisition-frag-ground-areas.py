#!/usr/bin/env python3
"""Project source-bound Ground frag damage areas for Acquisition and Domination."""
import argparse
import json
from pathlib import Path
import runpy


HERE = Path(__file__).resolve().parent
GROUND = runpy.run_path(str(HERE / 'project-retail-hospital-rocket-ground-bounds.py'))
HOSPITAL = GROUND['HOSPITAL']
NATIVE = GROUND['NATIVE']
encode, sha, require = (GROUND[name] for name in ('encode', 'sha', 'require'))
MODES = {
    'acquisition': {'apc': 3, 'truck': 7, 'escort': 9},
    'domination': {'apc': 3, 'truck': 7},
}
WHEEL_WITNESSES = {
    'damage_pair_walks_both_roots',
    'damage_vehicle_resolution_flag_zero',
    'debris_query_layer_and_registration',
    'operational_wheel_separate_add_to_world',
    'vehicle_shape_userdata_fallback',
    'wheel_box_userdata_and_phantom_creation',
    'wheel_packed_identity_layer_and_allocation',
}


def select_variants(sources, bindings, *, mode='acquisition'):
    return GROUND['select_variants'](sources, bindings, roles=MODES[mode])


def bind_overlap(elf, wheel, source_raw, dynamic_raw, mode):
    require(wheel['schema'] == f'retail_{mode}_frag_wheel_overlap_proof_v1'
            and wheel['retail_elf_sha256'] == HOSPITAL['RETAIL_ELF_SHA256']
            and wheel['vehicle_source_sha256'] == sha(source_raw)
            and wheel['dynamic_catalog_sha256'] == sha(dynamic_raw)
            and WHEEL_WITNESSES <= wheel['original_spans'].keys(),
            'Frag wheel overlap source mismatch')
    for name, span in wheel['original_spans'].items():
        start, end = (int(span[key], 16) for key in ('start', 'end_exclusive'))
        require(0 < end - start <= 64 * 1024
                and sha(elf.read(start, end - start)[0]) == span['sha256'],
                f'Frag wheel overlap span changed: {name}')
    rows = wheel['variants']
    result = {(row['spawn_type'], row['faction']): row for row in rows}
    expected = {(role, faction) for role in MODES[mode].values() for faction in range(3)}
    require(len(rows) == len(result) and result.keys() == expected,
            'incomplete original Ground overlap proof')
    return result


def project(args):
    elf = NATIVE['Elf'](args.elf, 'retail')
    main_raw = args.main_archive.read_bytes()
    source_raw = args.vehicle_sources.read_bytes()
    dynamic_raw = args.dynamic_catalog.read_bytes()
    wheel_raw = args.wheel_overlap_proof.read_bytes()
    sources, dynamic, wheel = map(json.loads, (source_raw, dynamic_raw, wheel_raw))
    GROUND['verify_source_projection'](source_raw, dynamic)
    overlap = bind_overlap(elf, wheel, source_raw, dynamic_raw, args.mode)
    require(sha(main_raw) == sources['sources']['main_archive']['sha256'] == HOSPITAL['MAIN_ARCHIVE_SHA256'],
            'Frag original main archive mismatch')
    archive = NATIVE['Psarc'](main_raw)
    source = {'retail_elf_sha256': HOSPITAL['RETAIL_ELF_SHA256'],
              'retail_main_sha256': sha(main_raw), 'dynamic_catalog_sha256': sha(dynamic_raw),
              'wheel_overlap_proof_sha256': sha(wheel_raw)}
    templates = HOSPITAL['indexed'](dynamic['templates'], 'id')
    variants, chassis_proof = [], []
    selected = select_variants(sources['vehicles'], dynamic['runtime_bindings']['vehicles'], mode=args.mode)
    for role, binding, original in selected:
        faction, name = binding['faction'], binding['template']
        chassis = GROUND['read_chassis'](archive, original['physics']['chassis'], select_operational=True)
        confirmed = overlap[role, faction]
        require(confirmed['vehicle_type_token'] == binding['vehicle_type_token']
                and confirmed['template'] == name
                and confirmed['chassis_packfile_sha256'] == chassis['packfile_sha256']
                and confirmed['roots'] == ['chassis', 'wheel'], 'Ground variant overlap proof mismatch')
        roots = templates[name]['roots']
        require(templates[name]['kind'] == 'ground_vehicle'
                and [(r['id'], r['layer'], r['shape']['kind']) for r in roots]
                == [('chassis', 22, 'hull'), ('wheel', 22, 'box')], 'Ground overlap topology mismatch')
        rear, front = GROUND['thresholds'](chassis['cached_shape_center_xyz'][2],
                                           chassis['cached_shape_half_extents_xyz'][2])
        variant = {**binding, 'spawn_type': role, 'packfile_sha256': chassis['packfile_sha256'],
                   'rear_threshold_z': rear, 'front_threshold_z': front,
                   'chassis_root': 'chassis', 'wheel_root': 'wheel'}
        variants.append(variant)
        chassis_proof.append({'binding': variant, 'chassis': chassis})
    proof = encode({'schema': f'mag_{args.mode}_frag_ground_area_proof_v1', 'source': source,
                    'vehicle_sources_sha256': sha(source_raw), 'original_spans': GROUND['native_witness'](elf),
                    'variants': chassis_proof})
    return encode({'schema': f'mag_{args.mode}_frag_ground_areas_v1',
                   'source': {**source, 'bounds_proof_sha256': sha(proof)}, 'variants': variants}), proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('elf', 'main-archive', 'vehicle-sources', 'dynamic-catalog', 'wheel-overlap-proof', 'output', 'proof-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--mode', choices=MODES, default='acquisition')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        HOSPITAL['output_paths'](
            [args.elf, args.main_archive, args.vehicle_sources, args.dynamic_catalog, args.wheel_overlap_proof],
            [args.output, args.proof_output],
        )
        for path, raw in zip((args.output, args.proof_output), project(args), strict=True):
            if args.check:
                require(path.read_bytes() == raw, f'Frag Ground areas differ: {path}')
            else:
                with path.open('xb') as output:
                    output.write(raw)
    except (KeyError, TypeError, ValueError, OSError) as error:
        parser.error(str(error))


if __name__ == '__main__':
    main()
