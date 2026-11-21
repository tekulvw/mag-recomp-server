"""Synthetic tests for the original Ground rocket hit-area projection."""
import runpy
from pathlib import Path
import unittest


TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-hospital-rocket-ground-bounds.py'


class HospitalRocketGroundProjectorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.projector = runpy.run_path(str(TOOL))

    def test_source_tolerance_and_asymmetric_bounds(self):
        rear, front = self.projector['thresholds'](1.0, 3.0)
        self.assertAlmostEqual(rear, -0.6633, places=6)
        self.assertAlmostEqual(front, 1.3233, places=6)
        with self.assertRaises(ValueError):
            self.projector['thresholds'](0.0, float('nan'))

    def test_source_inventory_checks_exact_roles_and_dynamic_templates(self):
        sources = [{'vehicle_type': role, 'PropertiesFileName': f'{role}-{faction}.zpr'}
                   for role in ('apc', 'truck') for faction in range(3)]
        bindings = [{'vehicle_type_token': self.projector['source_token'](role), 'faction': faction,
                     'template': f'{role}-{faction}'}
                    for role in ('apc', 'truck') for faction in range(3)]
        variants = self.projector['select_variants'](sources, bindings)
        self.assertEqual([(row[0], row[1]['faction']) for row in variants],
                         [(3, 0), (3, 1), (3, 2), (7, 0), (7, 1), (7, 2)])
        with self.assertRaises(ValueError):
            self.projector['select_variants'](sources, bindings[:-1])
        wrong = bindings[:]
        wrong[0] = {**wrong[0], 'template': 'truck-0'}
        with self.assertRaises(ValueError):
            self.projector['select_variants'](sources, wrong)
        with self.assertRaises(ValueError):
            self.projector['select_variants'](sources, [*bindings, bindings[0]])

    def test_runtime_fingerprint_must_bind_the_exact_source_bytes(self):
        with self.assertRaisesRegex(ValueError, 'source projection'):
            self.projector['verify_source_projection'](b'{"vehicles":[]}', {
                'source': {'shape_data_sha256': 'f' * 64},
            })
