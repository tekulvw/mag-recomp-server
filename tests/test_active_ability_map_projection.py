import runpy
import struct
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1] / 'tools' / 'extraction'
PROJECTOR = runpy.run_path(str(TOOLS / 'project-retail-active-ability-map.py'))


class AbilityMapProjectionTests(unittest.TestCase):
    def test_precision_projects_only_bound_category_nonzero_structural_targets(self):
        project = PROJECTOR.get('precision_target_rows')
        graph = {
            'definitions': [
                {'definition_token': 'structural', 'class_name': PROJECTOR['STRUCTURAL'],
                 'fields': {'Category': {'raw_hex': 'deadbeef000000010000000001'},
                            'OwnerTeam': {'value': 1}}},
                {'definition_token': 'uncategorized', 'class_name': PROJECTOR['STRUCTURAL'],
                 'fields': {'Category': {'raw_hex': 'deadbeef000000010000000000'},
                            'OwnerTeam': {'value': 0}}},
                {'definition_token': 'secure', 'class_name': 'ObjectiveSecureAndHoldProperties',
                 'fields': {'Category': {'raw_hex': 'deadbeef000000010000000001'},
                            'OwnerTeam': {'value': 0}}},
            ],
            'instances': [
                {'name_token': '0xa', 'definition_token': 'structural'},
                {'name_token': '0xb', 'definition_token': 'uncategorized'},
                {'name_token': '0xc', 'definition_token': 'secure'},
                {'name_token': '0xd', 'definition_token': 'structural'},
            ],
        }
        self.assertIsNotNone(project, 'ability maps must project qualified Precision Strike targets')
        self.assertEqual(project(graph, {0xc: 5, 0xb: 6, 0xa: 7}), [
            {'object_index': 7, 'team': 1},
        ])

    def test_damage_trigger_projects_linked_box_and_bound_vector_slot(self):
        project = PROJECTOR.get('damage_trigger_rows')
        axes = [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]]
        volume = {'trigger_token': '0xaa', 'objective_token': '0xbb',
                  'trigger_position': [1., 2., 3.],
                  'box': {'centre': [4., 5., 6.], 'axes': axes,
                          'half_extents': [2., 3., 4.], 'scale': [4., 6., 8.]}}
        fields = {'TriggerType': {'raw_hex': 'deadbeef000000010000000005'},
                  'StartActive': {'value': False}, 'DamageThreshold': {'value': 12.5}}
        graph = {
            'definitions': [{'definition_token': '0xdd', 'class_name': 'ObjectiveTriggerProperties',
                             'fields': fields}],
            'instances': [{'name_token': '0xaa', 'definition_token': '0xdd'}],
        }
        self.assertIsNotNone(project, 'ability maps must project damage triggers')
        self.assertEqual(
            project(graph, [volume], {0xbb: 7}, {0xaa: (7, 3)}),
            [{'object_index': 7, 'trigger_index': 3, 'trigger_type': 5,
              'start_active': False, 'damage_threshold': 12.5,
              'position': [4., 5., 6.],
              'box': {'centre': [4., 5., 6.], 'axes': axes, 'half_extents': [2., 3., 4.]}}],
        )
        with self.assertRaisesRegex(ValueError, 'qualified.*vector'):
            project(graph, [volume], {0xbb: 7}, {})
        with self.assertRaisesRegex(ValueError, 'objective.*slot'):
            project(graph, [volume], {0xbb: 7}, {0xaa: (8, 3)})
        fields['TriggerType']['raw_hex'] = 'deadbeef000000010000000003'
        self.assertEqual(project(graph, [volume], {0xbb: 7}, {}), [])

    def test_bounds_require_matching_terrain_and_empty_polygon(self):
        clv = bytearray(300)
        struct.pack_into('>IIIfI', clv, 0, 31, 0, 1, 17.0, 1)
        struct.pack_into('>H', clv, 40, 128)
        box = (12., 38., 24., 10., 4., 20.)
        struct.pack_into('>6f', clv, 170, *box)
        self.assertEqual(
            PROJECTOR['bounds'](clv, box),
            ({'min_x': 2., 'min_z': 4., 'max_x': 22., 'max_z': 44.}, [34., 42.]),
        )
        with self.assertRaisesRegex(ValueError, 'terrain.*header'):
            PROJECTOR['bounds'](clv, (11., *box[1:]))
        struct.pack_into('>I', clv, 194, 1)
        with self.assertRaisesRegex(ValueError, 'boundary polygon'):
            PROJECTOR['bounds'](clv, box)

    def test_catalog_join_checks_token_index_and_coverage(self):
        catalog = {
            'objects': [{'token': '0x4', 'object_index': 0}, {'token': '0x9', 'object_index': 1}],
        }
        self.assertEqual(PROJECTOR['catalog_indices'](catalog), {4: 0, 9: 1})
        catalog['objects'][1]['object_index'] = 0
        with self.assertRaisesRegex(ValueError, 'duplicate index'):
            PROJECTOR['catalog_indices'](catalog)

    def test_vector_rejects_truncated_and_duplicate_tokens(self):
        links = PROJECTOR['roster_tokens']
        self.assertEqual(links(struct.pack('>4I', 1, 2, 7, 9).hex()), (7, 9))
        with self.assertRaisesRegex(ValueError, 'length'):
            links(struct.pack('>3I', 1, 2, 7).hex())
        with self.assertRaisesRegex(ValueError, 'Duplicate'):
            links(struct.pack('>4I', 1, 2, 7, 7).hex())


if __name__ == '__main__':
    unittest.main()
