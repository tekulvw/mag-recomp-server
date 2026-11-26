"""Sabotage objective collision projection; invented sequences and packs only."""
import runpy
import struct
import unittest
from pathlib import Path

PROJECTOR = runpy.run_path(str(Path(__file__).resolve().parents[1]
                              / 'tools/extraction/project-retail-sabotage-structures.py'))


def text(value):
    raw = value.encode('ascii') + b'\0'
    return struct.pack('>I', len(raw)) + raw


def field(name, kind, data):
    return b'\1' + text(name) + text(kind) + struct.pack('>I', len(data) + 4) + data


def token(name, value):
    return field(name, 'util::Token', bytes.fromhex(value))


def vec(name, xyz):
    return field(name, 'SMath::Vec3', struct.pack('>3f', *xyz) + bytes(4))


def sequence(*events):
    out = struct.pack('>I', len(events))
    for kind, fields in events:
        body = (struct.pack('>II', 3, 0) + bytes.fromhex('deadbeef00000003')
                + text(kind) + fields + b'\0')
        out += struct.pack('>I', len(body) + 4) + body
    return out


def create(collision, position=(0, 0, 0), rotation=(0, 0, 0), start=0.0):
    return ('AnimObjectCreationEventDef',
            token('objectName', '0000000a') + token('geometry', '0000000b')
            + token('collision', collision) + vec('position', position) + vec('rotation', rotation)
            + field('start', 'float', struct.pack('>f', start)) + field('dur', 'float', struct.pack('>f', -1)))


def start(target, parent=False, forget=True):
    return ('PlayDependentSequenceEventDef',
            token('SequenceName', target) + field('useParentTransform', 'bool', bytes([parent]))
            + field('fireAndForget', 'bool', bytes([forget])))


def stop(target):
    return ('StopSequencesEventDef', token('SequencesName', target))


class SequenceActionTests(unittest.TestCase):
    def test_keeps_creations_starts_and_stops_and_drops_presentation(self):
        raw = sequence(('PrintEventDef', b''), create('00000c01', (1, 2, 3), (0, 90, 0), 3.0),
                       start('000000d1'), stop('000000d2'),
                       ('RotateNodeEventDef', token('objectName', '00000001')))
        actions = PROJECTOR['sequence_actions'](raw)
        self.assertEqual([a['kind'] for a in actions], ['create', 'start', 'stop'])
        self.assertEqual(actions[0]['position'], (1, 2, 3))
        self.assertEqual(actions[0]['start_seconds'], 3.0)
        self.assertTrue(actions[1]['fire_and_forget'] and not actions[1]['use_parent_transform'])

    def test_refuses_unknown_events(self):
        with self.assertRaisesRegex(ValueError, 'Unsupported sequence event'):
            PROJECTOR['sequence_actions'](sequence(('InventedEventDef', b'')))

    def test_accepts_only_rotations_about_y(self):
        rotate = PROJECTOR['axis_rotation']
        yaw = rotate((0, 90, 0))
        self.assertAlmostEqual(yaw[0][2], 1.0)
        flipped = rotate((180, 30, -180))
        expected = rotate((0, 150, 0))
        for row, want in zip(flipped, expected):
            for value, other in zip(row, want):
                self.assertAlmostEqual(value, other)
        with self.assertRaisesRegex(ValueError, 'Unqualified event rotation'):
            rotate((10, 0, 0))


class IndexTests(unittest.TestCase):
    """Objective 1's intact sequence starts the mast; its destroyed sequence
    starts the wreck, which stops the mast."""

    def setUp(self):
        self.sequences = {
            '000000a0': sequence(create('00000c01'), start('000000d1')),
            '000000a1': sequence(create('00000c02'), start('000000d2')),
            '000000d1': sequence(create('00000c03', (5, 10, 0))),
            '000000d2': sequence(create('00000c04', (5, 9, 0), start=3.0), stop('000000d1')),
        }
        self.packs = {token: [{'source_sha256': token * 8, 'neutral_sha256': token[::-1] * 8,
                               'parent_token': '00000000', 'collision_tag': 9, 'material_indices': [8]}]
                      for token in ('00000c01', '00000c02', '00000c03', '00000c04')}

    def actions(self, token):
        return PROJECTOR['sequence_actions'](self.sequences[token])

    def objective(self):
        return {'objective_token': '0x00000001', 'class': 'ObjectiveDemolitionStructuralProperties',
                'definition_sha256': '1' * 64,
                'state_sequence_indices': {'Inactive': 0, 'Completed': 1},
                'sequence_tokens': ['000000a0', '000000a1'],
                'sequence_sha256': {'000000a0': '2' * 64, '000000a1': '3' * 64},
                'pose': {'translation': [100, 0, 0], 'rotation': [[0, 0, 1], [0, 1, 0], [-1, 0, 0]]}}

    def test_poses_top_level_packs_on_the_objective_and_dependents_in_the_world(self):
        index = PROJECTOR['build_index']({'map': 'synthetic'}, [self.objective()], self.actions,
                                         self.packs, {})
        intact, destroyed = index['objectives'][0]['sequences']
        self.assertEqual(intact['starts'], ['000000d1'])
        self.assertEqual(intact['surfaces'][0]['pose']['translation'], [100, 0, 0])
        mast = index['dependents']['000000d1']['surfaces'][0]
        self.assertEqual(mast['pose']['translation'], [5, 10, 0])
        wreck = index['dependents']['000000d2']
        self.assertEqual(wreck['stops'], ['000000d1'])
        self.assertEqual(wreck['surfaces'][0]['start_seconds'], 3.0)
        self.assertEqual(destroyed['surfaces'][0]['neutral_sha256'],
                         self.packs['00000c02'][0]['neutral_sha256'])

    def test_refuses_dependents_that_need_the_parent_transform(self):
        self.sequences['000000a0'] = sequence(create('00000c01'), start('000000d1', parent=True))
        with self.assertRaisesRegex(ValueError, 'parent transform'):
            PROJECTOR['build_index']({'map': 'synthetic'}, [self.objective()], self.actions,
                                     self.packs, {})

    def test_standing_packs_come_from_the_animated_root_pack_index(self):
        rows = PROJECTOR['standing_surfaces'](
            {'schema': 'mag_animated_root_packs_v1', 'map': 'synthetic',
             'packs': [{'instance_name': '0x0000abcd', 'resource_token': '00000c09',
                        'packfile_sha256': '4' * 64, 'neutral_sha256': '5' * 64,
                        'collision_tag': 29, 'material_indices': [255]}]}, 'synthetic')
        self.assertEqual(rows[0]['object_token'], '0000abcd')
        self.assertEqual(rows[0]['pose']['translation'], [0.0, 0.0, 0.0])
        with self.assertRaisesRegex(ValueError, 'Wrong animated root pack index'):
            PROJECTOR['standing_surfaces']({'schema': 'mag_animated_root_packs_v1', 'map': 'other',
                                            'packs': []}, 'synthetic')


if __name__ == '__main__':
    unittest.main()
