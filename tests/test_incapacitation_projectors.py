"""SYNTHETIC incapacitation projector contracts; every input is newly invented.

A local module instance is repinned only to invented archive bytes. The
production command line has no hash-override switch.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / 'tools/extraction'
NAMES = ('extract-retail-incapacitation-rules', 'project-retail-combat-incapacitation')
POINTER = '/parameters/CharacterCollisionData/CharacterStyles/CharacterStyle0/CharacterStyle/m_bodyData'


def load(name):
    spec = importlib.util.spec_from_file_location('synthetic_' + name.replace('-', '_'), TOOLS / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def archive_bytes(resources):
    payloads = [('\n'.join(resources) + '\n').encode(), *resources.values()]
    count = len(payloads)
    toc = 32 + 30 * count + count
    raw = bytearray(struct.pack('>4sI4sIIIII', b'PSAR', 0x10003, b'zlib', toc, 30, count, 256, 0))
    offset = toc
    for i, value in enumerate(payloads):
        raw.extend(bytes(16) + struct.pack('>I', i) + len(value).to_bytes(5, 'big') + offset.to_bytes(5, 'big'))
        offset += len(value)
    raw.extend(bytes(len(value) for value in payloads))
    raw.extend(b''.join(payloads))
    return bytes(raw)


def invented_items():
    bodies = {f'CollisionBodyInfo{i}': {'CollisionBodyInfo': {
        'armorArea': i % 4, 'hitpointMult': 1 + i / 8, 'incapMult': i / 16}} for i in range(16)}
    return json.dumps({'resources': {'collision_info.lua': {'parameters': {'CharacterCollisionData': {
        'CharacterStyles': {'CharacterStyle0': {'CharacterStyle': {'m_bodyData': bodies}}}}}}}}).encode()


def invented_combat(items_raw):
    return json.dumps({
        'schema_version': 4, 'provenance': {'catalog_sha256': sha(items_raw), 'kept': 'invented'},
        'body': [{'location': i, 'armor_area': i % 4, 'multiplier': 1 + i / 8,
                  'source': {'file': 'collision_info.lua',
                             'pointer': f'{POINTER}/CollisionBodyInfo{i}/CollisionBodyInfo'}}
                 for i in reversed(range(16))],
        'bullets': {'7': {'invented': True}}}).encode()


class IncapacitationProjectorTests(unittest.TestCase):
    def test_commands_help_without_external_inputs(self):
        for name in NAMES:
            with self.subTest(name=name):
                result = subprocess.run([sys.executable, '-B', str(TOOLS / (name + '.py')), '--help'],
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn('--output', result.stdout)

    def test_rules_read_two_authored_scalars_and_bind_the_archive(self):
        tool = load('extract-retail-incapacitation-rules')
        member = (b'Invented = {\n  Other = "3",\n  fIncapacitatedDamageThreshold = "61.5",\n'
                  b'  fIncapacitatedTime = "9", -- invented comment\n}\n')
        arc = archive_bytes({tool.MEMBER: member})
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / 'invented-archive'
            archive.write_bytes(arc)
            tool.ARCHIVE_SHA256 = sha(arc)
            rules = tool.produce(archive)
            self.assertEqual(rules['schema_version'], 1)
            self.assertEqual(rules['incapacitated_damage_threshold_hp'], 61.5)
            self.assertEqual(rules['incapacitated_duration_seconds'], 9.0)
            self.assertEqual(rules['excluded_character_states'], [])
            self.assertEqual(rules['provenance']['member_sha256'], sha(member))
            self.assertEqual(rules['provenance']['json_pointers']['incapacitated_duration_seconds'],
                             '/Invented/fIncapacitatedTime')
            archive.write_bytes(arc + b'x')
            with self.assertRaisesRegex(ValueError, 'SHA256 mismatch'):
                tool.produce(archive)
            other = archive_bytes({'/run/mag/ps3/scripts/other.zpr': member})
            archive.write_bytes(other)
            tool.ARCHIVE_SHA256 = sha(other)
            with self.assertRaisesRegex(ValueError, 'Missing character gameplay member'):
                tool.produce(archive)

    def test_rules_reject_missing_duplicate_nonpositive_and_nonnumeric_scalars(self):
        tool = load('extract-retail-incapacitation-rules')
        for text in [b'A = { fIncapacitatedDamageThreshold = "50" }',
                     b'A = { fIncapacitatedDamageThreshold = "50", fIncapacitatedTime = "0" }',
                     b'A = { fIncapacitatedDamageThreshold = "-1", fIncapacitatedTime = "4" }',
                     b'A = { fIncapacitatedDamageThreshold = "x", fIncapacitatedTime = "4" }',
                     b'A = { fIncapacitatedDamageThreshold = "5", fIncapacitatedTime = "4" }\n'
                     b'B = { fIncapacitatedTime = "4" }']:
            with self.subTest(text=text), self.assertRaisesRegex(ValueError, 'Require one finite positive'):
                tool.project(text)

    def test_combat_gains_each_authored_row_multiplier_and_keeps_everything_else(self):
        tool = load('project-retail-combat-incapacitation')
        items = invented_items()
        combat = invented_combat(items)
        output = tool.project(combat, items)
        for row in output['body']:
            self.assertEqual(row['incapacitated_multiplier'], row['location'] / 16)
        source = output['provenance'].pop('incapacitated_multiplier_source')
        self.assertEqual(source['item_catalog_sha256'], sha(items))
        self.assertEqual(source['input_combat_sha256'], sha(combat))
        for row in output['body']:
            del row['incapacitated_multiplier']
        self.assertEqual(output, json.loads(combat))
        again = tool.project(tool.encode(tool.project(combat, items)), items)
        self.assertEqual([row['incapacitated_multiplier'] for row in again['body']],
                         [i / 16 for i in reversed(range(16))])

    def test_combat_rejects_unbound_or_inconsistent_inputs(self):
        tool = load('project-retail-combat-incapacitation')
        items = invented_items()
        combat = json.loads(invented_combat(items))
        cases = {}
        cases['item_catalog_does_not_match_combat_provenance'] = (combat | {'provenance': {'catalog_sha256': 'x'}}, items)
        short = json.loads(json.dumps(combat))
        short['body'].pop()
        cases['requires_16_body_rows'] = (short, items)
        moved = json.loads(json.dumps(combat))
        moved['body'][0]['source']['pointer'] = 'invented'
        cases['body_row_source_mismatch'] = (moved, items)
        living = json.loads(json.dumps(combat))
        living['body'][0]['multiplier'] = 99
        cases['living_body_row_differs_from_authored_entry'] = (living, items)
        conflicting = json.loads(json.dumps(combat))
        conflicting['body'][0]['incapacitated_multiplier'] = 99
        cases['existing_incapacitated_multiplier_differs'] = (conflicting, items)
        negative = json.loads(items)
        body = negative['resources']['collision_info.lua']['parameters']['CharacterCollisionData'][
            'CharacterStyles']['CharacterStyle0']['CharacterStyle']['m_bodyData']
        body['CollisionBodyInfo3']['CollisionBodyInfo']['incapMult'] = -1
        negative_raw = json.dumps(negative).encode()
        rebound = json.loads(json.dumps(combat))
        rebound['provenance']['catalog_sha256'] = sha(negative_raw)
        cases['invalid_authored_incapacitated_multiplier'] = (rebound, negative_raw)
        for message, (value, item_raw) in cases.items():
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                tool.project(json.dumps(value).encode(), item_raw)

    def test_combat_cli_refuses_to_overwrite_and_checks_existing_output(self):
        items = invented_items()
        combat = invented_combat(items)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'items.json').write_bytes(items)
            (root / 'combat.json').write_bytes(combat)
            command = [sys.executable, '-B', str(TOOLS / 'project-retail-combat-incapacitation.py'),
                       '--combat', str(root / 'combat.json'), '--item-catalog', str(root / 'items.json'),
                       '--output', str(root / 'out.json')]
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 0)
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 1)
            self.assertEqual(subprocess.run(command + ['--check'], capture_output=True).returncode, 0)
            (root / 'out.json').write_text('{}')
            self.assertEqual(subprocess.run(command + ['--check'], capture_output=True).returncode, 1)


if __name__ == '__main__':
    unittest.main()
