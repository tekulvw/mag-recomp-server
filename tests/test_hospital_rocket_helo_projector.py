"""Synthetic checks for the Helo rocket-area source bindings."""
import json
from pathlib import Path
import runpy
import unittest


TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-hospital-rocket-helo-areas.py'


class HeloRocketAreasProjectionTests(unittest.TestCase):
    def test_reads_all_variants_and_rejects_wrong_damage_document_binding(self):
        tool = runpy.run_path(str(TOOL))
        models = json.dumps({
            'schema': 'mag_private_retail_domination_helo_models_v1',
            'retail_elf_sha256': tool['HOSPITAL']['RETAIL_ELF_SHA256'],
            'main_archive_sha256': tool['HOSPITAL']['MAIN_ARCHIVE_SHA256'],
            'collision_layer': 22, 'motion_type': 6,
            'variants': {faction: {'packfile_sha256': letter * 64} for faction, letter
                         in zip(('Raven', 'Valor', 'SVER'), 'abc', strict=True)},
        }).encode()
        damage = {
            'schema': 'mag_domination_helo_damage_v1',
            'retail_elf_sha256': tool['HOSPITAL']['RETAIL_ELF_SHA256'],
            'models_sha256': tool['sha'](models),
            'profiles': {str(i): {} for i in range(3)},
        }
        rows = tool['source_variants'](models, json.dumps(damage).encode())
        self.assertEqual([row['packfile_sha256'][0] for row in rows], ['b', 'a', 'c'])
        damage['models_sha256'] = 'd' * 64
        with self.assertRaisesRegex(ValueError, 'source pair'):
            tool['source_variants'](models, json.dumps(damage).encode())

    def test_reference_bounds_use_exact_shared_native_f32_conversion(self):
        tool = runpy.run_path(str(TOOL))
        rear, front = tool['GROUND']['thresholds'](-2.0, 5.0)
        self.assertAlmostEqual(rear, -2.3133, places=6)
        self.assertAlmostEqual(front, .9933, places=6)
