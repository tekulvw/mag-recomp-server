import runpy
import struct
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1] / 'tools' / 'extraction'
projector = runpy.run_path(str(TOOLS / 'project-retail-active-ability-static-ray.py'))


class StaticRayProjectionTests(unittest.TestCase):
    def test_authored_vertical_bounds_use_center_and_half_extent(self):
        clv = bytearray(194)
        struct.pack_into('>IIIfI', clv, 0, 31, 0, 1, 17.0, 1)
        struct.pack_into('>H', clv, 40, 128)
        struct.pack_into('>6f', clv, 170, 0, 50, 0, 1, 13, 1)
        self.assertEqual(projector['authored_vertical_bounds'](clv), [37.0, 63.0])

    def test_keeps_root_and_child_filters_distinct_and_accounts_for_omissions(self):
        instances = [
            {'id': 'root:mesh-0', 'triangles': [[0, 1, 2]]},
            {'id': 'root:mesh-1', 'triangles': [[0, 1, 2]]},
        ]
        source = {
            'root:mesh-0': (9, 0, 2),
            'root:mesh-1': (11, 0, 1),
            'other:mesh-0': (9, 0, 1),
        }
        removed = {
            'root:mesh-0': [1],
            'other:mesh-0': [0],
        }
        projector['enrich_instances'](instances, source, removed)
        self.assertEqual(instances[0]['collision']['filter_info'], 9)
        self.assertEqual(instances[1]['collision']['filter_info'], 11)
        self.assertEqual(instances[0]['collision']['ray_child_filter_info'], 0)
        self.assertEqual(instances[0]['collision']['convex_radius'], 0.)

    def test_rejects_missing_geometry_or_unqualified_source(self):
        source = {'root:mesh-0': (9, 0, 1)}
        with self.assertRaisesRegex(ValueError, 'omission'):
            projector['enrich_instances']([], source, {})
        with self.assertRaisesRegex(ValueError, 'source mesh'):
            projector['enrich_instances'](
                [{'id': 'unknown', 'triangles': [[0, 1, 2]]}], source, {},
            )
