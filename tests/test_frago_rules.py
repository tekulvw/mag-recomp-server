import hashlib
import json
from pathlib import Path
import runpy
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PROJECTOR = ROOT / 'tools/extraction/project-retail-frago-rules.py'
API = runpy.run_path(str(PROJECTOR))
RETAIL_SHA = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'


def encode(document):
    return json.dumps(document).encode()


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def authored_float(value):
    return {'type': 'float', 'raw_hex': struct.pack('>f', value).hex(), 'value': value}


def placed(name, definition, position):
    return {'name_token': name, 'definition_token': definition, 'fields': [
        {'name': f'Translate{axis}', **authored_float(value)}
        for axis, value in zip('XYZ', position)
    ]}


def elf64_be(virtual_address):
    ident = b'\x7fELF\x02\x02\x01' + bytes(9)
    header = struct.pack('>16sHHIQQQIHHHHHH', ident, 2, 21, 1, 0, 64, 0, 0, 64, 56, 1, 0, 0, 0)
    load = struct.pack('>IIQQQQQQ', 1, 4, 128, virtual_address, 0, 8, 16, 1)
    return header + load + bytes(8) + b'\x11\x22\x33\x44\x41\x48\x00\x00'


class FragoProjectionTests(unittest.TestCase):
    def setUp(self):
        self.digests = {'game.psarc': 'a' * 64, 'experience.zpr': 'b' * 64}
        defs = [
            {'definition_token': '0x0000000a', 'class_name': 'ObjectiveSecureAndHoldProperties',
             'fields': {'FRAGOBonusRange': authored_float(9.75)}},
            {'definition_token': '0x0000000b', 'class_name': 'ObjectiveDemolitionStructuralProperties',
             'fields': {'FRAGOBonusRange': authored_float(0)}},
            {'definition_token': '0x0000000c', 'class_name': 'ObjectiveArrivalProperties', 'fields': {}},
            {'definition_token': '0x0000000d', 'class_name': 'ObjectiveTriggerProperties', 'fields': {}},
        ]
        instances = [
            placed('0x000000fa', '0x0000000a', [100, 101, 102]),
            placed('0x000000fb', '0x0000000b', [3, 4, 5]),
            placed('0x000000fc', '0x0000000c', [6, 7, 8]),
            placed('0x000000fd', '0x0000000d', [9, 10, 11]),
        ]
        self.selected = {'map': {'map': 'test_map', 'mode_enum': 'GameType128'},
                         'objectives': {'definitions': defs, 'instances': instances}}
        self.authored = {'schema_version': 1, 'application_id': 20231,
                         'source_archive_sha256': self.digests['game.psarc'], 'maps': [self.selected]}
        self.catalog = {
            'format': 'mag_retail_objective_catalog', 'schema_version': 1,
            'application_id': 20231, 'protocol': 0x6cc, 'game_type': 1,
            'map_name': 'test_map', 'entity_family': 12, 'network_type': 12,
            'source': {'source_image_sha256': RETAIL_SHA, 'startup_proof_sha256': 'c' * 64},
            'objects': [
                {'token': '0x000000fb', 'object_index': 109, 'initial_state': 4},
                {'token': '0x000000fa', 'object_index': 7, 'initial_state': 0,
                 'interaction': {'position': [-1.25, 0, 16777217]}, 'opaque_rule': {'keep': 'all fields'}},
                {'token': '0x000000fc', 'object_index': 73, 'initial_state': 2,
                 'demolition': {'position': [12, 13, 14]}},
            ],
            'squads': [{'team': 0, 'squad_index': 0, 'initial_objective_index': 73}],
            'control': {'bunker_checks': ['existing']},
        }
        self.progression = {
            'application_id': 20231, 'schema_version': 1, 'source_sha256': self.digests,
            'table': {'FRAGOBonus_VehicleRange': 3.14159265},
        }

    def project(self, *, bind_authored=True, default_radius=11.5):
        authored = encode(self.authored)
        if bind_authored:
            self.catalog['source']['authored_catalog_sha256'] = sha(authored)
        return API['project'](encode(self.catalog), authored, encode(self.progression), default_radius)

    def test_projection_preserves_catalog_and_uses_runtime_anchors_and_authored_radii(self):
        result = self.project()
        rules = result.pop('frago')
        self.assertEqual(result, self.catalog)
        self.assertEqual(rules, {
            'schema_version': 1,
            'source': {'authored_catalog_sha256': sha(encode(self.authored)),
                       'progression_catalog_sha256': sha(encode(self.progression)),
                       'source_image_sha256': RETAIL_SHA},
            'vehicle_range': 3.1415927410125732,
            'objectives': [
                {'object_index': 7, 'position': [-1.25, 0.0, 16777216.0], 'radius': 9.75},
                {'object_index': 73, 'position': [12.0, 13.0, 14.0], 'radius': 11.5},
                {'object_index': 109, 'position': [3.0, 4.0, 5.0], 'radius': 0.0},
            ],
        })

    def test_zero_vehicle_range_is_preserved(self):
        self.progression['table']['FRAGOBonus_VehicleRange'] = 0
        self.assertEqual(self.project()['frago']['vehicle_range'], 0)

    def test_rounds_finite_vehicle_range_to_max_f32(self):
        self.progression['table']['FRAGOBonus_VehicleRange'] = 3.40282347e38
        self.assertEqual(self.project()['frago']['vehicle_range'], 3.4028234663852886e38)

    def test_uses_verified_default_only_when_radius_field_is_absent(self):
        self.selected['objectives']['definitions'][2]['fields']['FRAGOBonusRange'] = authored_float(0)
        result = self.project(default_radius=lambda: self.fail('Read an unused native default'))
        self.assertEqual(result['frago']['objectives'][1]['radius'], 0)

    def test_can_reproject_and_replaces_only_the_frago_section(self):
        self.catalog['frago'] = {'old': 'data'}
        first = self.project()
        self.catalog = first
        self.assertEqual(self.project(), first)

    def test_rejects_different_authored_bytes(self):
        self.catalog['source']['authored_catalog_sha256'] = 'd' * 64
        with self.assertRaisesRegex(ValueError, 'authored.*SHA256'):
            self.project(bind_authored=False)

    def test_rejects_different_objective_source_image(self):
        self.catalog['source']['source_image_sha256'] = 'd' * 64
        with self.assertRaisesRegex(ValueError, 'source image'):
            self.project()

    def test_rejects_progression_from_another_archive(self):
        self.progression['source_sha256']['game.psarc'] = 'e' * 64
        with self.assertRaisesRegex(ValueError, 'progression.*archive'):
            self.project()

    def test_rejects_non_acquisition_or_domination_catalog(self):
        self.catalog['game_type'] = 0
        with self.assertRaisesRegex(ValueError, 'Acquisition or Domination'):
            self.project()

    def test_rejects_authored_mode_mismatch(self):
        self.catalog['game_type'] = 2
        with self.assertRaisesRegex(ValueError, 'authored map mode'):
            self.project()

    def test_rejects_ambiguous_map_selection(self):
        self.authored['maps'].append(self.selected)
        with self.assertRaisesRegex(ValueError, 'one authored map'):
            self.project()

    def test_rejects_unbound_target_and_trigger_substitution(self):
        self.catalog['objects'][0]['token'] = '0x000000fd'
        with self.assertRaisesRegex(ValueError, 'objective token inventory'):
            self.project()

    def test_rejects_missing_target(self):
        self.catalog['objects'].pop()
        with self.assertRaisesRegex(ValueError, 'objective token inventory'):
            self.project()

    def test_rejects_duplicate_bindings(self):
        self.catalog['objects'][0]['object_index'] = 7
        with self.assertRaisesRegex(ValueError, 'Duplicate objective slot'):
            self.project()
        self.catalog['objects'][0]['object_index'] = 109
        self.catalog['objects'][0]['token'] = '0x000000FA'
        with self.assertRaisesRegex(ValueError, 'Duplicate objective token'):
            self.project()

    def test_rejects_slot_outside_native_objective_pool(self):
        self.catalog['objects'][0]['object_index'] = 128
        with self.assertRaisesRegex(ValueError, 'objective slot'):
            self.project()

    def test_rejects_duplicate_authored_definition_and_instance(self):
        definitions = self.selected['objectives']['definitions']
        definitions.append(definitions[0])
        with self.assertRaisesRegex(ValueError, 'Duplicate authored definition token'):
            self.project()
        definitions.pop()
        instances = self.selected['objectives']['instances']
        instances.append(instances[0])
        with self.assertRaisesRegex(ValueError, 'Duplicate authored instance token'):
            self.project()

    def test_rejects_missing_and_unplaced_definition(self):
        instance = self.selected['objectives']['instances'][0]
        instance['definition_token'] = '0x00000000'
        with self.assertRaisesRegex(ValueError, 'definition token'):
            self.project()
        instance['definition_token'] = '0x0000000b'
        with self.assertRaisesRegex(ValueError, 'Unplaced authored objective definition'):
            self.project()

    def test_rejects_instance_radius_override(self):
        self.selected['objectives']['instances'][0]['fields'].append({
            'name': 'FRAGOBonusRange', **authored_float(0),
        })
        with self.assertRaisesRegex(ValueError, 'instance FRAGOBonusRange'):
            self.project()

    def test_rejects_conflicting_runtime_positions(self):
        self.catalog['objects'][1]['demolition'] = {'position': [3, 4, 5]}
        with self.assertRaisesRegex(ValueError, 'Ambiguous runtime position'):
            self.project()

    def test_rejects_missing_and_duplicate_fallback_coordinates(self):
        fields = self.selected['objectives']['instances'][1]['fields']
        fields.pop()
        with self.assertRaisesRegex(ValueError, 'Incomplete authored translation'):
            self.project()
        fields.append(fields[0])
        with self.assertRaisesRegex(ValueError, 'Duplicate authored instance field'):
            self.project()

    def test_rejects_malformed_numbers(self):
        data = self.progression['table']
        for value in (-1, float('nan'), float('inf'), 1e100, 10**1000, True, '7'):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'vehicle range'):
                data['FRAGOBonus_VehicleRange'] = value
                self.project()

    def test_rejects_inconsistent_authored_radius_value_and_raw_float(self):
        radius = self.selected['objectives']['definitions'][0]['fields']['FRAGOBonusRange']
        radius['value'] = 10
        with self.assertRaisesRegex(ValueError, 'FRAGOBonusRange.*encoding'):
            self.project()

    def test_rejects_negative_default(self):
        with self.assertRaisesRegex(ValueError, 'native objective radius'):
            self.project(default_radius=-0.25)

    def test_rejects_nonfinite_runtime_position(self):
        self.catalog['objects'][1]['interaction']['position'][0] = float('inf')
        with self.assertRaisesRegex(ValueError, 'runtime position'):
            self.project()


class ElfDefaultTests(unittest.TestCase):
    def test_reads_big_endian_float_in_file_backed_load_segment(self):
        self.assertEqual(API['elf_load_f32'](elf64_be(0x3000), 0x3004), 12.5)

    def test_rejects_bss_or_unmapped_defaults(self):
        for address in (0x2fff, 0x3005, 0x3008):
            with self.subTest(address=address), self.assertRaisesRegex(ValueError, 'file-backed'):
                API['elf_load_f32'](elf64_be(0x3000), address)

    def test_rejects_truncated_segment(self):
        with self.assertRaisesRegex(ValueError, 'ELF load segment'):
            API['elf_load_f32'](elf64_be(0x3000)[:-1], 0x3004)


class ProjectorCliTests(unittest.TestCase):
    def run_cli(self, objective, authored, progression, elf, output, *args):
        return subprocess.run(
            [sys.executable, str(PROJECTOR), '--objective-catalog', str(objective),
             '--authored-catalog', str(authored), '--progression-catalog', str(progression),
             '--retail-elf', str(elf), '--output', str(output), *args],
            capture_output=True, text=True,
        )

    def test_rejects_wrong_elf_even_when_native_default_is_at_expected_address(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            elf, output = root / 'test.elf', root / 'output.json'
            elf.write_bytes(elf64_be(0x1b3be64))
            result = self.run_cli(root / 'objects.json', root / 'authored.json', root / 'xp.json', elf, output)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('original retail ELF', result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_input_path_as_output_including_check_mode(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            objective = root / 'objects.json'
            objective.write_bytes(b'original')
            result = self.run_cli(objective, root / 'authored.json', root / 'xp.json',
                                  root / 'test.elf', objective, '--check')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('outside the public checkout and inputs', result.stderr)
            self.assertEqual(objective.read_bytes(), b'original')

    def test_rejects_output_inside_public_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.run_cli(root / 'objects.json', root / 'authored.json', root / 'xp.json',
                                  root / 'test.elf', PROJECTOR, '--check')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('outside the public checkout and inputs', result.stderr)


if __name__ == '__main__':
    unittest.main()
