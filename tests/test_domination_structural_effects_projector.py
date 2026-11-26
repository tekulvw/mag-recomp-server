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
    def test_acquisition_only_projects_structural_definitions_for_selected_level(self):
        classes = PROJECTOR['qualified_objective_classes']
        row = {'map': {'mode_enum': 'GameType128'}}
        self.assertEqual(classes({}, row, 'GameType128'),
                         {'ObjectiveDemolitionStructuralProperties'})
        with self.assertRaisesRegex(ValueError, 'Wrong authored mode'):
            classes({}, row, 'GameType256')
        with self.assertRaisesRegex(ValueError, 'Wrong authored mode'):
            classes({'mode_enum': 'GameType256'}, row, 'GameType128')

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

    def test_event_kinds_accept_any_named_class_but_keep_boundaries(self):
        kinds = PROJECTOR['sequence_event_kinds']
        raw = sequence('AnimEventDef', field('objectName', 'util::Token', bytes(4)))
        self.assertEqual(kinds(raw), ['AnimEventDef'])
        with self.assertRaises(ValueError):
            PROJECTOR['sequence_events'](raw)
        for bad in (raw[:-1], raw + b'\0' * 16, sequence('Bad-Name')):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                kinds(bad)


def definition(token, class_name, sequences, indices, owner='00000000'):
    fields = {
        'Sequences': {'raw_hex': (struct.pack('>2I', 1, len(sequences))
                                  + b''.join(bytes.fromhex(t) for t in sequences)).hex()},
        'SequenceOwnerObjective': {'raw_hex': owner},
    }
    for state in PROJECTOR['STATES']:
        fields[state + 'SequenceIndex'] = {'value': indices.get(state, -2)}
    return {'definition_token': token, 'class_name': class_name, 'fields': fields,
            'source_sha256': token[2:] * 8}


class PublishedSequenceTests(unittest.TestCase):
    def records(self):
        closed = sequence('PrintEventDef')
        opening = sequence('AnimEventDef')
        return {token: {'resource_version': 3, 'unparsed_resource_header_hex': raw.hex()}
                for token, raw in (('000000a1', closed), ('000000a2', opening))}

    def graph(self, *definitions):
        return {'definitions': list(definitions),
                'instances': [{'name_token': '0x0000001%d' % index,
                               'definition_token': row['definition_token']}
                              for index, row in enumerate(definitions)]}

    def test_publishes_authored_controller_sequences_and_skips_empty_or_structural(self):
        capture = definition('0x00000c01', 'ObjectiveSecureAndHoldProperties',
                             ['000000a1', '000000a2'], {'Deactivated': 0, 'Completed': 1})
        arrival = definition('0x00000c02', 'ObjectiveArrivalProperties',
                             ['000000a1'], {'Inactive': 0, 'CancelRetaken': -1})
        passive = definition('0x00000c03', 'ObjectiveSecureAndHoldProperties', [],
                             {state: -1 for state in PROJECTOR['STATES']})
        structural = definition('0x00000c04', 'ObjectiveDemolitionStructuralProperties',
                                ['000000a1'], {'Deactivated': 0})
        rows = PROJECTOR['published_sequences'](
            self.graph(capture, arrival, passive, structural), self.records())
        self.assertEqual([row['objective_token'] for row in rows], ['0x00000010', '0x00000011'])
        first, second = rows
        self.assertEqual(first['class'], 'ObjectiveSecureAndHoldProperties')
        self.assertEqual(first['definition_sha256'], '00000c01' * 8)
        self.assertEqual((first['state_sequence_indices']['Deactivated'],
                          first['state_sequence_indices']['Completed'],
                          first['state_sequence_indices']['Activating']), (0, 1, -2))
        self.assertEqual([s['events'] for s in first['sequences']],
                         [['PrintEventDef'], ['AnimEventDef']])
        self.assertEqual(len(first['sequences'][1]['source_sha256']), 64)
        self.assertEqual(second['state_sequence_indices']['CancelRetaken'], -1)

    def test_rejects_linked_owner_bad_index_and_missing_resource(self):
        publish = PROJECTOR['published_sequences']
        cases = (
            (definition('0x00000c01', 'ObjectiveArrivalProperties', ['000000a1'], {},
                        owner='00000001'), 'Linked sequence owner'),
            (definition('0x00000c01', 'ObjectiveArrivalProperties', ['000000a1'],
                        {'Completed': 1}), 'Invalid sequence index'),
            (definition('0x00000c01', 'ObjectiveArrivalProperties', ['000000ff'], {}),
             'Unresolved sequence resource'),
        )
        for row, message in cases:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                publish(self.graph(row), self.records())


if __name__ == '__main__':
    unittest.main()
