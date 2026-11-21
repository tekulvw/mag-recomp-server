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

    def test_optional_materials_preserve_native_null_and_use_explicit_terrain_tags(self):
        instances = [
            {'id': 'compiled:root', 'vertices_xyz': [[0, 1, 2]], 'triangles': [[0, 1, 2]]},
            {'id': 'terrain:root', 'vertices_xyz': [[2, 1, 0]], 'triangles': [[0, 1, 2]]},
        ]
        filters = {row['id']: (9, 0, 1) for row in instances}
        original = [row.copy() for row in instances]
        material = projector['compiled_material']
        self.assertEqual(material([6, 255, 4], 1), 255)
        annotations = {
            'compiled:root': {'material_index': material([6, 255, 4], 1)},
            'terrain:root': {'terrain_material': True},
        }
        projector['enrich_instances'](instances, filters, {}, materials=annotations)
        self.assertEqual(instances[0].pop('material_index'), 255)
        self.assertIs(instances[1].pop('terrain_material'), True)
        for new, previous in zip(instances, original, strict=True):
            self.assertEqual(new.pop('collision')['filter_info'], 9)
            self.assertEqual(new, previous)

    def test_material_projection_rejects_unknown_slots_subparts_and_missing_coverage(self):
        material = projector['compiled_material']
        for indices, subpart in [([9], None), ([9], 1), ([9], -1), ([9], True), ([27], 0), ([256], 0)]:
            with self.subTest(indices=indices, subpart=subpart), self.assertRaises(ValueError):
                material(indices, subpart)
        with self.assertRaisesRegex(ValueError, 'material inventory'):
            projector['enrich_instances'](
                [{'id': 'root', 'triangles': [[0, 1, 2]]}], {'root': (9, 0, 1)}, {}, materials={},
            )
        with self.assertRaisesRegex(ValueError, 'Existing material binding differs'):
            projector['enrich_instances'](
                [{'id': 'root', 'triangles': [[0, 1, 2]], 'terrain_material': True}],
                {'root': (9, 0, 1)}, {}, materials={'root': {'material_index': 4}},
            )
