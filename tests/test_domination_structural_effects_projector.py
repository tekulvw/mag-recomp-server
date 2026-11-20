import runpy
import struct
import unittest
from pathlib import Path


PROJECTOR = runpy.run_path(str(Path(__file__).resolve().parents[1]
                              / 'tools/extraction/project-retail-domination-structural-effects.py'))


def text(value):
    raw = value.encode('ascii') + b'\0'
    return struct.pack('>I', len(raw)) + raw


def field(name, kind, data):
    return b'\1' + text(name) + text(kind) + struct.pack('>I', len(data) + 4) + data


def sequence(kind, fields=b''):
    body = (struct.pack('>II', 3, 0) + bytes.fromhex('deadbeef00000003')
            + text(kind) + fields + b'\0')
    return struct.pack('>II', 1, len(body) + 4) + body


class StructuralSequenceTests(unittest.TestCase):
    def test_reads_bounded_animated_object_and_uses_explicit_defaults(self):
        data = b''.join(
            field(name, 'util::Token', struct.pack('>I', index))
            for index, name in enumerate(('objectName', 'geometry', 'collision'), 1)
        )
        data += field('start', 'float', struct.pack('>f', 0))
        data += field('dur', 'float', struct.pack('>f', -1))
        event, = PROJECTOR['sequence_events'](sequence('AnimObjectCreationEventDef', data))
        self.assertEqual(event['object_token'], '00000001')
        self.assertEqual(event['collision_token'], '00000003')
        self.assertEqual(event['position'], (0, 0, 0))
        self.assertEqual(event['duration_seconds'], -1)

    def test_rejects_unqualified_type_bad_boundary_and_extra_payload(self):
        parse = PROJECTOR['sequence_events']
        raw = sequence('PrintEventDef')
        for bad in (sequence('InventedEventDef'), raw[:-1], raw + b'\0' * 16, raw + b'\1'):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse(bad)

    def test_qualifies_visual_cycles_but_rejects_nested_collision(self):
        qualify = PROJECTOR['qualify_visual_sequence_controls']
        parse = PROJECTOR['sequence_events']
        token = '00000001'
        start = sequence('PlayDependentSequenceEventDef',
                         field('SequenceName', 'util::Token', bytes.fromhex(token)))
        stop = sequence('StopSequencesEventDef',
                        field('SequencesName', 'util::Token', bytes.fromhex('00000002')))
        record = {'resource_version': 3, 'unparsed_resource_header_hex': start.hex()}
        root = {'00000011': {'events': parse(start)}}
        self.assertEqual(qualify(root, {token: record}), set())
        self.assertEqual(root['00000011']['events'][0]['kind'], 'VisualSequenceControl')
        self.assertEqual(qualify({'00000011': {'events': parse(stop)}}, {}), {'00000002'})
        with self.assertRaisesRegex(ValueError, 'Missing original sequence start'):
            qualify({'00000011': {'events': parse(start)}}, {})
        animated_fields = b''.join(
            field(name, 'util::Token', bytes.fromhex(token))
            for name in ('objectName', 'geometry', 'collision'))
        animated_fields += field('start', 'float', struct.pack('>f', 0))
        animated_fields += field('dur', 'float', struct.pack('>f', -1))
        record['unparsed_resource_header_hex'] = sequence(
            'AnimObjectCreationEventDef', animated_fields).hex()
        with self.assertRaisesRegex(ValueError, 'Nested sequence touches collision'):
            qualify({'00000011': {'events': parse(start)}}, {token: record})


if __name__ == '__main__':
    unittest.main()
