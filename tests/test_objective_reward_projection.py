import copy
import hashlib
import io
import json
from contextlib import redirect_stdout
from pathlib import Path
import runpy
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
PROJECTOR = ROOT / 'tools/extraction/project-retail-objective-rewards.py'
RETAIL_SHA = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'


def encode(document):
    return (json.dumps(document, indent=2) + '\n').encode()


def enum(value):
    return {'type': 'enum', 'raw_hex': f'deadbeef0000000100{value:08x}'}


class ObjectiveRewardProjectionTests(unittest.TestCase):
    def setUp(self):
        self.api = runpy.run_path(str(PROJECTOR))
        definitions = [
            {'definition_token': '0x0000000a', 'class_name': 'ObjectiveSecureAndHoldProperties',
             'fields': {'Category': enum(0), 'SubType': enum(255)}},
            {'definition_token': '0x0000000b', 'class_name': 'ObjectiveDemolitionStructuralProperties',
             'fields': {'Category': enum(4), 'SubType': enum(1)}},
            {'definition_token': '0x0000000c', 'class_name': 'ObjectiveArrivalProperties',
             'fields': {'Category': enum(0), 'SubType': enum(12)}},
            {'definition_token': '0x0000000d', 'class_name': 'ObjectiveTriggerProperties', 'fields': {}},
        ]
        instances = [
            {'name_token': f'0x000000f{key}', 'definition_token': row['definition_token'], 'fields': []}
            for key, row in zip('abcd', definitions)
        ]
        instances[1]['fields'] = [{'name': 'SubType', **enum(0)}]
        self.selected = {'map': {'map': 'synthetic_map', 'mode_enum': 'GameType128'},
                         'objectives': {'definitions': definitions, 'instances': instances}}
        self.metadata = {'schema_version': 1, 'application_id': 20231, 'maps': [self.selected]}
        self.catalog = {
            'format': 'mag_retail_objective_catalog', 'schema_version': 1, 'application_id': 20231,
            'protocol': 1740, 'game_type': 1, 'map_name': 'synthetic_map',
            'entity_family': 12, 'network_type': 12,
            'source': {'source_image_sha256': RETAIL_SHA, 'capture_qualification': 'native_source_gameplay'},
            'objects': [
                {'token': '0x000000FB', 'object_index': 109, 'initial_state': 4, 'native_class': 'structural',
                 'demolition': {'position': [12, 13, 14]}},
                {'token': '0x000000fa', 'object_index': 7, 'initial_state': 0,
                 'scoring': {'owner_team': 0}, 'opaque': ['keep', 0]},
                {'token': '0x000000fc', 'object_index': 73, 'initial_state': 2},
            ],
            'frago': {'vehicle_range': 4.25, 'objectives': [{'object_index': 7, 'radius': 0}]},
            'squads': [{'team': 0, 'squad_index': 0, 'initial_objective_index': 73}],
        }

    def inputs(self):
        metadata = encode(self.metadata)
        self.catalog['source']['authored_catalog_sha256'] = hashlib.sha256(metadata).hexdigest()
        return encode(self.catalog), metadata

    def project(self):
        return self.api['project'](*self.inputs())

    def test_layers_only_authored_classifications_and_preserves_zero_and_override(self):
        result = self.project()
        expected = copy.deepcopy(self.catalog)
        expected['objects'][0]['reward'] = {'category': 4, 'subtype': 0, 'demolition': True}
        expected['objects'][1]['reward'] = {'category': 0, 'subtype': 255, 'demolition': False}
        expected['objects'][2]['reward'] = {'category': 0, 'subtype': 12, 'demolition': False}
        self.assertEqual(result, expected)
        self.catalog = result
        self.assertEqual(self.project(), result)

    def test_accepts_domination_metadata_without_retail_application_field(self):
        self.catalog['game_type'] = 2
        self.catalog['objects'].pop()
        self.selected['map']['mode_enum'] = 'GameType256'
        self.selected['objectives']['definitions'].pop(2)
        self.selected['objectives']['instances'].pop(2)
        self.metadata.pop('application_id')
        self.metadata.update(format='mag_retail_domination_map_metadata', mode_enum='GameType256')
        result = self.project()
        self.assertEqual([obj['reward']['category'] for obj in result['objects']], [4, 0])

    def test_rejects_missing_and_malformed_authored_classification(self):
        fields = self.selected['objectives']['definitions'][0]['fields']
        with self.subTest('missing enum'):
            fields.pop('Category')
            with self.assertRaisesRegex(ValueError, 'Category'):
                self.project()
        for prop in ({**enum(0), 'type': 'uint32'},
                     {'type': 'enum', 'raw_hex': '00'},
                     {'type': 'enum', 'raw_hex': '00' * 13},
                     {'type': 'enum', 'raw_hex': 'xyz'}, enum(5)):
            with self.subTest(prop=prop), self.assertRaisesRegex(ValueError, 'Category'):
                fields['Category'] = prop
                self.project()

    def test_rejects_ambiguous_or_malformed_instance_override(self):
        fields = self.selected['objectives']['instances'][1]['fields']
        fields.append({'name': 'SubType', **enum(1)})
        with self.assertRaisesRegex(ValueError, 'Duplicate.*SubType'):
            self.project()
        fields.pop()
        fields[0].update(enum(256))
        with self.assertRaisesRegex(ValueError, 'SubType'):
            self.project()

    def test_rejects_conflicting_existing_rewards(self):
        structural, _, arrival = self.catalog['objects']
        for conflict in (None, {'category': 4, 'subtype': 1, 'demolition': True},
                         {'category': 4, 'subtype': 0, 'demolition': 1},
                         {'category': 4, 'subtype': 0, 'demolition': True, 'bonus': 1}):
            with self.subTest(conflict=conflict), self.assertRaisesRegex(ValueError, 'Conflicting.*reward'):
                structural['reward'] = conflict
                self.project()
        structural.pop('reward')
        arrival['reward'] = {'category': 4, 'subtype': 0, 'demolition': False}
        with self.assertRaisesRegex(ValueError, 'Conflicting.*reward'):
            self.project()

    def test_arrival_stays_untyped_in_the_runtime_and_requires_its_authored_classification(self):
        arrival = self.catalog['objects'][2]
        arrival['native_class'] = 'secure_hold'
        with self.assertRaisesRegex(ValueError, 'runtime class'):
            self.project()
        arrival.pop('native_class')
        del self.selected['objectives']['definitions'][2]['fields']['SubType']
        with self.assertRaisesRegex(ValueError, 'SubType'):
            self.project()

    def test_original_reward_classifier_defaults_to_secure_and_structural_classes(self):
        self.assertEqual(set(self.api['reward_rules'](self.selected)), {'0x000000fa', '0x000000fb'})

    def test_rejects_wrong_source_or_catalog_identity(self):
        objective, metadata = self.inputs()
        with self.assertRaisesRegex(ValueError, 'authored.*SHA256'):
            self.api['project'](objective, metadata + b' ')
        for scope in ('native_initial_load', 'bounded_concurrent_token_slot_capture'):
            with self.subTest(scope=scope), self.assertRaisesRegex(ValueError, 'qualification'):
                self.catalog['source']['capture_qualification'] = scope
                self.project()
        self.catalog['source']['capture_qualification'] = 'native_source_gameplay'
        self.catalog['source']['source_image_sha256'] = 'd' * 64
        with self.assertRaisesRegex(ValueError, 'source image'):
            self.project()
        self.catalog['source']['source_image_sha256'] = RETAIL_SHA
        for field, value in [('application_id', 20230), ('protocol', 1741), ('entity_family', 13),
                             ('network_type', 13), ('schema_version', True), ('game_type', 0)]:
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'retail Acquisition or Domination'):
                original, self.catalog[field] = self.catalog[field], value
                try:
                    self.project()
                finally:
                    self.catalog[field] = original

    def test_rejects_wrong_map_mode_and_unqualified_objective_class(self):
        self.selected['map']['mode_enum'] = 'GameType256'
        with self.assertRaisesRegex(ValueError, 'authored map mode'):
            self.project()
        self.selected['map']['mode_enum'] = 'GameType128'
        self.metadata['maps'].append(self.selected)
        with self.assertRaisesRegex(ValueError, 'one authored map'):
            self.project()
        self.metadata['maps'].pop()
        self.catalog['objects'][0]['native_class'] = 'secure_hold'
        with self.assertRaisesRegex(ValueError, 'runtime class'):
            self.project()
        self.catalog['objects'][0]['native_class'] = 'structural'
        self.selected['objectives']['definitions'][0]['class_name'] = 'ObjectiveOtherProperties'
        with self.assertRaisesRegex(ValueError, 'Unsupported authored objective class'):
            self.project()

    def test_rejects_duplicate_and_mismatched_token_inventories(self):
        objects = self.catalog['objects']
        original = copy.deepcopy(objects[0])
        for edit, message in [({'token': '0x000000FA'}, 'Duplicate objective token'),
                              ({'object_index': 7}, 'Duplicate objective slot'),
                              ({'object_index': 128}, 'objective slot inventory'),
                              ({'token': '0x000000fd'}, 'objective token inventory')]:
            with self.subTest(edit=edit), self.assertRaisesRegex(ValueError, message):
                objects[0] = {**original, **edit}
                self.project()
        objects[0] = original
        for key, message in [('instances', 'Duplicate authored instance token'),
                             ('definitions', 'Duplicate authored definition token')]:
            rows = self.selected['objectives'][key]
            rows.append(copy.deepcopy(rows[0]))
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, message):
                self.project()
            rows.pop()
        objects.pop()
        with self.assertRaisesRegex(ValueError, 'objective token inventory'):
            self.project()

    def test_cli_checks_exact_bytes_and_refuses_to_overwrite(self):
        main = self.api['main']
        image = b'synthetic image identity for the projection tool test'
        synthetic_sha = hashlib.sha256(image).hexdigest()
        self.catalog['source']['source_image_sha256'] = synthetic_sha
        objective, metadata = self.inputs()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp).resolve()
            metadata_path = path / 'metadata.json'
            objective_path = path / 'objectives.json'
            image_path = path / 'native-image'
            output = path / 'output.json'
            for dest, raw in [(metadata_path, metadata), (objective_path, objective), (image_path, image)]:
                dest.write_bytes(raw)
            args = [str(PROJECTOR), '--metadata', str(metadata_path), '--objectives', str(objective_path),
                    '--native-elf', str(image_path), '--output', str(output)]

            def invoke(argv):
                with patch.object(sys, 'argv', argv), redirect_stdout(io.StringIO()):
                    main()

            with self.assertRaises(SystemExit), patch.object(sys, 'stderr', io.StringIO()) as stderr:
                invoke(args)
            self.assertIn('Wrong original retail ELF', stderr.getvalue())
            self.assertFalse(output.exists())

            with patch.dict(main.__globals__, {'RETAIL_SHA': synthetic_sha}):
                invoke(args)
                expected = output.read_bytes()
                self.assertEqual(json.loads(expected)['objects'][0]['reward']['subtype'], 0)
                invoke(args + ['--check'])
                with self.assertRaises(SystemExit), patch.object(sys, 'stderr', io.StringIO()):
                    invoke(args)
                self.assertEqual(output.read_bytes(), expected)

                mismatch = expected.replace(b'4.25', b'5.25')
                self.assertNotEqual(mismatch, expected)
                output.write_bytes(mismatch)
                with self.assertRaises(SystemExit), patch.object(sys, 'stderr', io.StringIO()) as stderr:
                    invoke(args + ['--check'])
                self.assertIn('differs from reproduction', stderr.getvalue())
                self.assertEqual(output.read_bytes(), mismatch)
                with self.assertRaises(SystemExit), patch.object(sys, 'stderr', io.StringIO()):
                    invoke(args[:-1] + [str(objective_path)])
                self.assertEqual(objective_path.read_bytes(), objective)

                checkout = path / 'checkout'
                checkout.mkdir()
                public_output = checkout / 'output.json'
                with patch.dict(main.__globals__, {'TOOLS': checkout / 'tools' / 'extraction'}):
                    with self.assertRaises(SystemExit), patch.object(sys, 'stderr', io.StringIO()) as stderr:
                        invoke(args[:-1] + [str(public_output)])
                self.assertIn('outside the public checkout', stderr.getvalue())
                self.assertFalse(public_output.exists())


if __name__ == '__main__':
    unittest.main()
