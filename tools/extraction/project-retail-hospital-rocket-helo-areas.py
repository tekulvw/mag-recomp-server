#!/usr/bin/env python3
"""Project original operational Helo chassis bounds for hospital rocket damage."""
import argparse
import json
from pathlib import Path
import runpy


HERE = Path(__file__).resolve().parent
GROUND = runpy.run_path(str(HERE / 'project-retail-hospital-rocket-ground-bounds.py'))
HOSPITAL = GROUND['HOSPITAL']
NATIVE = GROUND['NATIVE']
encode, sha, require = (GROUND[name] for name in ('encode', 'sha', 'require'))
FACTIONS = ('Valor', 'Raven', 'SVER')


def source_variants(models_raw, damage_raw):
    models, damage = map(json.loads, (models_raw, damage_raw))
    elf = HOSPITAL['RETAIL_ELF_SHA256']
    require(models['schema'] == 'mag_private_retail_domination_helo_models_v1'
            and damage['schema'] == 'mag_domination_helo_damage_v1'
            and models['retail_elf_sha256'] == damage['retail_elf_sha256'] == elf
            and models['main_archive_sha256'] == HOSPITAL['MAIN_ARCHIVE_SHA256']
            and damage['models_sha256'] == sha(models_raw)
            and models['collision_layer'] == 22 and models['motion_type'] == 6
            and set(models['variants']) == set(FACTIONS)
            and set(damage['profiles']) == {'0', '1', '2'}, 'invalid Helo model/damage source pair')
    return [models['variants'][name] for name in FACTIONS]


def project(args):
    elf = NATIVE['Elf'](args.elf, 'retail')
    main_raw = args.main_archive.read_bytes()
    models_raw = args.helo_models.read_bytes()
    damage_raw = args.helo_damage.read_bytes()
    models = source_variants(models_raw, damage_raw)
    require(sha(main_raw) == HOSPITAL['MAIN_ARCHIVE_SHA256'], 'changed original Helo main archive')
    archive = NATIVE['Psarc'](main_raw)
    origin = {'retail_elf_sha256': HOSPITAL['RETAIL_ELF_SHA256'],
              'retail_main_sha256': sha(main_raw), 'helo_models_sha256': sha(models_raw),
              'helo_damage_sha256': sha(damage_raw)}
    spans = GROUND['native_witness'](elf)
    spans['helo_physics_uses_vehicle_bounds'] = {
        'start': '0x7f6030', 'end_exclusive': '0x7f6074',
        'sha256': sha(elf.read(0x7f6030, 0x44)[0]),
    }
    require(elf.read(0x7f6068, 4)[0] == bytes.fromhex('48084801'),
            'original Helo physics bounds call changed')
    projected, provenance = [], []
    for faction, model in enumerate(models):
        chassis = GROUND['read_chassis'](archive, {
            'archive_member': model['physics_member'],
            'member_sha256': model['physics_member_sha256'],
            'resource_token': f'{model["physics_token"]:08x}',
            'packfile_sha256': model['packfile_sha256'],
        })
        rear, front = GROUND['thresholds'](chassis['cached_shape_center_xyz'][2],
                                           chassis['cached_shape_half_extents_xyz'][2])
        row = {'faction': faction, 'packfile_sha256': chassis['packfile_sha256'],
               'rear_threshold_z': rear, 'front_threshold_z': front}
        projected.append(row)
        provenance.append({'binding': row, 'chassis': chassis})
    proof = encode({'schema': 'mag_domination_hospital_rocket_helo_area_proof_v1',
                    'source': origin, 'original_spans': spans, 'variants': provenance})
    rules = encode({'schema': 'mag_domination_hospital_rocket_helo_areas_v1',
                    'source': {**origin, 'area_proof_sha256': sha(proof)}, 'variants': projected})
    return rules, proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('elf', 'main-archive', 'helo-models', 'helo-damage', 'output', 'proof-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        HOSPITAL['output_paths']([args.elf, args.main_archive, args.helo_models, args.helo_damage],
                                 [args.output, args.proof_output])
        for path, raw in zip((args.output, args.proof_output), project(args), strict=True):
            if args.check:
                require(path.read_bytes() == raw, f'Helo area projection differs: {path}')
            else:
                with path.open('xb') as output:
                    output.write(raw)
    except (KeyError, TypeError, ValueError, OSError) as error:
        parser.error(str(error))


if __name__ == '__main__':
    main()
