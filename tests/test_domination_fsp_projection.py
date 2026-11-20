"""Full authored FSP membership must survive filtered startup availability."""
import hashlib
import json
from pathlib import Path
import runpy
import unittest


class FspProjectionTests(unittest.TestCase):
    def setUp(self):
        path = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-domination-fsp.py'
        self.assertTrue(path.is_file(), 'FSP projector is missing')
        self.project = runpy.run_path(str(path))['project']
        self.metadata = {
            'format': 'mag_retail_domination_map_metadata', 'schema_version': 1,
            'mode_enum': 'GameType256', 'maps': [{
                'map': {'map': 'synthetic_fsp'},
                'spawns': [{'name_token': f'0x{n:08x}', 'team': 0 if n != 11 else 1,
                            'user_class': 0, 'category': 0 if n != 12 else 2}
                           for n in (10, 11, 12)],
                'squads': [{'team': i // 16, 'platoon': i % 16 // 4, 'squad': i % 4,
                            'spawns': ['0x0000000a', '0x0000000c'] if i < 16 else ['0x0000000b']}
                           for i in range(32)],
                'objectives': {
                    'definitions': [{'definition_token': '0x00000032', 'fields': {
                        'FSPType': {'type': 'enum', 'raw_hex': 'deadbeef000000010000000001'},
                        'ForwardSpawnPoint': {'token': '0x0000000c'}}}],
                    'instances': [{'name_token': '0x00000064', 'definition_token': '0x00000032'}]},
                'forward_spawns': [{'objective_token': '0x00000064', 'spawn_token': '0x0000000c'}],
            }],
        }
        self.spawn = {'maps': [{'name': 'synthetic_fsp', 'runtime_indices_verified': True,
                               'locations': [{'token': n, 'index': i} for i, n in enumerate((10, 11, 12))]}],
                      'match_profile': {'game_type': 2}}

    def invoke(self):
        metadata = json.dumps(self.metadata).encode()
        spawn = json.dumps(self.spawn).encode()
        objectives = json.dumps({'map_name': 'synthetic_fsp', 'game_type': 2,
                                 'source': {'authored_catalog_sha256': hashlib.sha256(metadata).hexdigest()},
                                 'objects': [{'token': '0x00000064', 'object_index': 0}]}).encode()
        return self.project(metadata, spawn, objectives)

    def test_preserves_full_membership_and_source_order(self):
        output = self.invoke()
        self.assertEqual(output['squads'][0], {'index': 0, 'points': [10, 12], 'category_zero': 10})
        self.assertEqual(output['squads'][31], {'index': 31, 'points': [11], 'category_zero': 11})
        self.assertEqual(output['links'], [{'objective_token': 100, 'spawn_token': 12, 'fsp_type': 1}])
        self.assertEqual(output['points'][2]['index'], 2)

    def test_refuses_incomplete_or_cross_team_membership(self):
        self.metadata['maps'][0]['squads'][0]['spawns'] = ['0x0000000b']
        with self.assertRaisesRegex(ValueError, 'team'):
            self.invoke()
        self.metadata['maps'][0]['squads'].pop()
        with self.assertRaisesRegex(ValueError, '32'):
            self.invoke()

    def test_refuses_missing_authored_link_and_unverified_slots(self):
        self.metadata['maps'][0]['forward_spawns'] = []
        with self.assertRaisesRegex(ValueError, 'link'):
            self.invoke()
        self.metadata['maps'][0]['forward_spawns'] = [{'objective_token': '0x00000064', 'spawn_token': '0x0000000c'}]
        self.spawn['maps'][0]['runtime_indices_verified'] = False
        with self.assertRaisesRegex(ValueError, 'binding'):
            self.invoke()
