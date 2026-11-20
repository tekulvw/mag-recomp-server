import copy
from pathlib import Path
import runpy
import unittest


API = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tools/extraction'
                         / 'project-retail-domination-gameplay-objectives.py'))


class DominationGameplayObjectiveProjection(unittest.TestCase):
    def setUp(self):
        properties = {
            'Active': True, 'OwnerTeam': 1, 'CanRepair': True, 'CanDeactivate': False,
            'ActivationScoreValue': 9, 'DeactivationScoreValue': 2, 'TimeToArmMs': 17,
            'TimeToDetonateMs': 95, 'TimeToDefuseMs': 23, 'TimeToRepairMs': 221, 'MinTimeToRepairMs': 39,
        }
        self.definition = {
            'class_name': API['STRUCTURAL'], 'source_sha256': 'b' * 64,
            'fields': {name: {'value': value} for name, value in properties.items()},
        }
        self.definition['fields']['InitialActiveState'] = {'raw_hex': 'synthetic inactive enum'}
        self.instance = {'fields': [dict(name='Translate' + axis, value=i + 5) for i, axis in enumerate('XYZ')]}
        self.initial = {'token': '0x00000002', 'object_index': 3, 'initial_state': 0}
        self.intent = dict(property_class=API['STRUCTURAL'], client_network_index=3,
                           property_table_sha256=self.definition['source_sha256'],
                           property_active=self.definition['fields']['Active'],
                           property_initial_active_state=self.definition['fields']['InitialActiveState'],
                           current_network_state_after_mission_start=None, sim_network_index=None)
        self.boxes = [
            {'centre': [4, 5, 6], 'axes': [[1, 0, 0], [0, 1, 0], [0, 0, 1]], 'half_extents': [2, 3, 4]},
            {'centre': [9, 3, 5], 'axes': [[1, 0, 0], [0, 1, 0], [0, 0, 1]], 'half_extents': [3, 4, 5]},
        ]

    def bind(self, boxes):
        return API['bind_one'](self.initial, self.definition, self.instance, self.intent, boxes)

    def test_keeps_inactive_constructor_state_and_uses_proved_box_order(self):
        result = self.bind(list(reversed(self.boxes)))
        self.assertEqual(result['initial_state'], 0)
        self.assertEqual(result['native_class'], 'structural')
        self.assertEqual(result['demolition']['trigger_volumes'], list(reversed(self.boxes)))
        self.assertEqual(result['demolition']['detonate_ms'], 95)
        self.assertFalse(result['demolition']['can_defuse'])
        self.assertEqual(result['repair']['minimum_repair_ms'], 39)
        self.assertTrue(result['repair']['can_repair'])

    def test_unlinked_class_keeps_repair_but_never_invents_an_interaction_volume(self):
        result = self.bind([])
        self.assertIn('repair', result)
        self.assertNotIn('demolition', result)
        self.assertNotIn('interaction', result)

    def test_rejects_rebound_startup_and_unproved_instance_override(self):
        original = copy.deepcopy(self.intent)
        self.intent['client_network_index'] += 1
        with self.assertRaisesRegex(ValueError, 'Objective class/startup binding mismatch'):
            self.bind(self.boxes)
        self.intent = original
        self.instance['fields'].append(dict(name='TimeToRepairMs', value=400))
        with self.assertRaisesRegex(ValueError, 'Unexpected instance gameplay property override'):
            self.bind(self.boxes)
