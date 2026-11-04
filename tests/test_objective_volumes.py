"""Objective trigger box projection checks; every value below is invented."""
import math
from pathlib import Path
import runpy
import unittest

ROOT = Path(__file__).resolve().parents[1]
EXTRACTION = ROOT / 'tools/extraction'
VOLUMES = runpy.run_path(str(EXTRACTION / 'extract-retail-objective-volumes.py'))
HELPERS = runpy.run_path(str(EXTRACTION / 'retail_map_helpers.py'))


def field(name, value, kind='float', token=None):
    row = {'name': name, 'type': kind, 'value': value, 'raw_hex': '', 'offset': '0x0', 'bytes': 4}
    if token is not None:
        row['token'] = token
    return row


def transform(name, translate, rotate, scale, uniform=1.0):
    rows = [field('Name', name, 'util::Token', name)]
    for axis, value in zip('XYZ', translate):
        rows.append(field('Translate' + axis, value))
    for axis, value in zip('XYZ', rotate):
        rows.append(field('Rotate' + axis, value))
    rows.append(field('Scale', uniform))
    for axis, value in zip('XYZ', scale):
        rows.append(field('Scale' + axis, value))
    return rows


def level(volume_class='TriggerVolumeBoxProperties', scale=(4.0, 2.0, 6.0), duplicate=False):
    """One invented secure-and-hold objective with one linked trigger and box volume."""
    graph = {
        'definitions': [
            {'definition_token': '0x00000a01', 'class_name': 'ObjectiveSecureAndHoldProperties', 'fields': {}, 'offset': 10},
            {'definition_token': '0x00000a02', 'class_name': 'ObjectiveTriggerProperties', 'offset': 20, 'fields': {
                'ObjectiveTriggerVolume': {'token': '0x00000c01'}, 'ObjectiveID': {'token': '0x00000b01'},
                'TriggerType': {'raw_hex': 'deadbeef000000010000000003'}}},
        ],
        'instances': [
            {'name_token': '0x00000b01', 'definition_token': '0x00000a01', 'fields': transform('0x00000b01', (1, 2, 3), (0, 0, 0), (1, 1, 1))},
            {'name_token': '0x00000b02', 'definition_token': '0x00000a02', 'fields': transform('0x00000b02', (9, 8, 7), (0, 0, 0), (1, 1, 1))},
        ],
    }
    volume = {'preceding_token': 0xd1, 'offset': '0x40', 'source_sha256': '0' * 64,
              'fields': transform('0x00000c01', (10.0, 20.0, 30.0), (0.0, 30.0, 0.0), scale, uniform=scale[0])}
    instances = [volume] + ([dict(volume, offset='0x80')] if duplicate else [])
    return graph, instances, {0xd1: volume_class}


def enclosed(box, point, radius=0.0):
    offset = [p - c for p, c in zip(point, box['centre'])]
    return all(abs(sum(o * a for o, a in zip(offset, axis))) <= h + radius
               for axis, h in zip(box['axes'], box['half_extents']))


class ObjectiveVolumeTests(unittest.TestCase):
    def test_box_uses_per_axis_scale_as_full_edge_length(self):
        result = VOLUMES['project_level'](*level(), HELPERS['euler_basis'])
        trigger = result['objectives'][0]['triggers'][0]
        self.assertEqual(result['counts'], {'objectives': 1, 'triggers': 1})
        self.assertEqual(trigger['objective_token'], '0x00000b01')
        self.assertEqual(trigger['trigger_position'], [9.0, 8.0, 7.0])
        box = trigger['box']
        self.assertEqual(box['half_extents'], [2.0, 1.0, 3.0])
        self.assertEqual(box['axes'], [[float(v) for v in row] for row in HELPERS['euler_basis']([0.0, 30.0, 0.0])])

    def test_rotated_box_containment_follows_its_axes(self):
        box = VOLUMES['project_level'](*level(), HELPERS['euler_basis'])['objectives'][0]['triggers'][0]['box']
        x_axis, _, z_axis = box['axes']
        inside = [c + 1.9 * x + 2.9 * z for c, x, z in zip(box['centre'], x_axis, z_axis)]
        outside = [c + 2.1 * x for c, x in zip(box['centre'], x_axis)]
        self.assertTrue(enclosed(box, inside))
        self.assertFalse(enclosed(box, outside))
        self.assertTrue(enclosed(box, outside, radius=0.2))
        # A point inside the old 5 m development sphere can be far outside the box.
        far = [c + 4.5 * z for c, z in zip(box['centre'], box['axes'][1])]
        self.assertLess(math.dist(far, box['centre']), 5.0)
        self.assertFalse(enclosed(box, far))

    def test_rejects_unsupported_volume_class_scale_and_ambiguity(self):
        with self.assertRaisesRegex(ValueError, 'unsupported trigger volume class'):
            VOLUMES['project_level'](*level(volume_class='TriggerVolumeSphereProperties'), HELPERS['euler_basis'])
        with self.assertRaisesRegex(ValueError, 'scale must be positive'):
            VOLUMES['project_level'](*level(scale=(4.0, 0.0, 6.0)), HELPERS['euler_basis'])
        with self.assertRaisesRegex(ValueError, 'missing or ambiguous'):
            VOLUMES['project_level'](*level(duplicate=True), HELPERS['euler_basis'])

    def test_rejects_unknown_objective_and_non_finite_values(self):
        graph, instances, classes = level()
        graph['definitions'][1]['fields']['ObjectiveID'] = {'token': '0x0000ffff'}
        with self.assertRaisesRegex(ValueError, 'unknown objective'):
            VOLUMES['project_level'](graph, instances, classes, HELPERS['euler_basis'])
        graph, instances, classes = level()
        instances[0]['fields'][1]['value'] = float('nan')
        with self.assertRaisesRegex(ValueError, 'non-finite'):
            VOLUMES['project_level'](graph, instances, classes, HELPERS['euler_basis'])


RUNTIME = runpy.run_path(str(EXTRACTION / 'project-retail-objective-runtime.py'))
APPLY = runpy.run_path(str(EXTRACTION / 'apply-objective-trigger-volumes.py'))
UNIT = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]


def volume_catalog(triggers=('0x0000b002',), map_name='invented_map'):
    return {'schema_version': 1, 'format': 'mag_retail_objective_trigger_volumes', 'application_id': 20231,
            'volume_semantics': 'oriented_box_half_scale',
            'maps': [{'map': map_name, 'objectives': [{'objective_token': '0x0000b001', 'triggers': [
                {'trigger_token': t, 'box': {'centre': [1.0, 2.0, 3.0], 'axes': UNIT, 'half_extents': [2.0, 1.0, 0.5]}}
                for t in triggers]}]}]}


def graph():
    return {'map': {'map': 'invented_map'},
            'trigger_links': [{'objective_instance_token': '0x0000b001', 'trigger_instance_token': '0x0000b002'}]}


def rules(trigger_count=1):
    return {'0x0000b001': {'interaction': {'trigger_count': trigger_count, 'maximum_distance_m': 5.0,
                                           'distance_policy': 'development_point_distance'}}}


class TriggerVolumeAttachTests(unittest.TestCase):
    def test_attaches_linked_boxes_and_switches_policy(self):
        body = RUNTIME['attach_trigger_volumes'](graph(), rules(), volume_catalog())['0x0000b001']['interaction']
        self.assertEqual(body['distance_policy'], 'authored_trigger_box')
        self.assertEqual(body['trigger_volumes'], [{'centre': [1.0, 2.0, 3.0], 'axes': UNIT, 'half_extents': [2.0, 1.0, 0.5]}])

    def test_rejects_mismatched_triggers_counts_and_maps(self):
        with self.assertRaisesRegex(ValueError, 'do not match linked triggers'):
            RUNTIME['attach_trigger_volumes'](graph(), rules(), volume_catalog(triggers=('0x0000b003',)))
        with self.assertRaisesRegex(ValueError, 'do not match linked triggers'):
            RUNTIME['attach_trigger_volumes'](graph(), rules(trigger_count=2), volume_catalog())
        with self.assertRaisesRegex(ValueError, 'lacks this map'):
            RUNTIME['attach_trigger_volumes'](graph(), rules(), volume_catalog(map_name='other_map'))
        bad = volume_catalog()
        bad['volume_semantics'] = 'sphere'
        with self.assertRaisesRegex(ValueError, 'identity'):
            RUNTIME['attach_trigger_volumes'](graph(), rules(), bad)

    def test_overlay_requires_the_runtime_catalog_source_metadata(self):
        runtime = {'format': 'mag_retail_objective_catalog', 'schema_version': 1, 'map_name': 'invented_map',
                   'source': {'authored_catalog_sha256': 'a' * 64},
                   'objects': [{'token': '0x0000B001', 'object_index': 0, **rules()['0x0000b001']}]}
        out = APPLY['apply'](runtime, graph(), 'a' * 64, volume_catalog())
        self.assertEqual(out['objects'][0]['interaction']['distance_policy'], 'authored_trigger_box')
        with self.assertRaisesRegex(ValueError, 'not the runtime catalog source'):
            APPLY['apply'](runtime, graph(), 'b' * 64, volume_catalog())

if __name__ == '__main__':
    unittest.main()
