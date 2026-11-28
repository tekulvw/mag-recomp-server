"""Invented resources exercise extraction without original data or executable bytes."""
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
import zlib

from test_reflective_zrb import field, nested, obj, text

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools/extraction'))


def vector(*rows):
    return struct.pack('>II', 1, len(rows)) + b''.join(rows)


def integer(name, value):
    return field(name, 'int32', struct.pack('>i', value))


def enum(name, value):
    return field(name, 'enum', bytes.fromhex('deadbeef0000000100000000') + bytes([value]))


class BootstrapExtractionTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('v212_extract', ROOT / 'tools/extraction/extract-retail-v212-bootstrap.py')
        self.api = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = self.api
        spec.loader.exec_module(self.api)
        self.inputs = self.fixture()
        self.spec = self.api.SourceSpec('synthetic-bootstrap', hashlib.sha256(self.inputs['engine']).hexdigest(),
            {key: hashlib.sha256(self.inputs[key]).hexdigest() for key in ('headers', 'classes', 'equipment')},
            (1, 1, 1, 1), 4, 3, 3)

    def fixture(self):
        names = ['synthetic-a', 'synthetic-b', 'synthetic-c']
        tokens = [123] + [zlib.crc32(name.encode()) for name in names]
        banks = [obj('FactionItemList', field('items', 'ztl::vector', vector(
            obj('ItemListData', field('id', 'util::Token', struct.pack('>I', token)))))) for token in tokens]
        items = obj('ItemList', field('factionItemLists', 'ztl::vector', vector(*banks)))
        levels = [obj('ExperienceLevel', integer('Experience', xp), integer('SkillPointsGained', 1),
                      integer('Rank', 0), integer('LoadoutBudget', 0)) for xp in (0, 0, 50)]
        experience = obj('ExperienceTable', field('ExperienceRequiredScale', 'float', struct.pack('>f', 1)),
            field('Levels', 'ztl::vector', vector(*levels)),
            field('Awards', 'object', nested(obj('ExperienceAwards', field('EnemyKilled', 'float', struct.pack('>f', 5))))))
        classes = []
        for faction in range(3):
            kit = obj('CharacterClassInv', *(field(slot, 'util::Token', struct.pack('>I', tokens[faction + 1] if slot == 'Primary' else 0))
                       for slot in self.api.SLOTS if slot != 'BackupMagazine'))
            classes.append(obj('CharacterClass', enum('Class', 21), enum('Faction', faction),
                field('InventoryList', 'ztl::vector', vector(kit, kit, kit))))
        equipment = b''.join(obj('WeaponData', field('id', 'ztl::string', text(name)), integer('magazine_capacity', capacity))
                             for name, capacity in zip(names, (7, 8, 9)))
        return dict(engine=b'invented engine identity only', headers=items + experience,
                    classes=obj('CharacterClassTable', field('Classes', 'ztl::vector', vector(*classes))), equipment=equipment)

    def test_normalized_outputs_bind_inputs_and_apply_explicit_policy(self):
        outputs = self.api.render(self.inputs, self.spec)
        self.assertEqual(len(outputs), 3)
        armory = json.loads(outputs['retail-v212-bootstrap-armory.json'])
        progression = json.loads(outputs['retail-v212-bootstrap-progression.json'])
        manifest = json.loads(outputs['retail-v212-bootstrap-profile.json'])
        self.assertEqual([x['wire_id'] for x in armory['items']], [0, 1074, 2098, 3122])
        self.assertEqual([x['primary_ammo'] for x in armory['items']], [None, 7, 8, 9])
        for faction in armory['factions']:
            self.assertEqual(faction['available_item_bits'], [50])
            self.assertEqual(faction['skill_ids'], [])
            kits = faction['loadouts']
            self.assertEqual(len(kits), 5)
            self.assertTrue(all(row['items'][0] is not None for row in kits[:3]))
            self.assertTrue(all(row['items'][19] is None for row in kits))
            self.assertTrue(all(row['items'] == [None] * 22 for row in kits[3:]))
        self.assertEqual(progression['table']['Levels']['ExperienceLevel2']['ExperienceLevel']['Experience'], 50)
        self.assertIs(type(progression['table']['Awards']['EnemyKilled']), int)
        self.assertEqual(progression['creation_settings']['root_defaults']['AdditionalSkillPoints'], 0)
        for role, entry in manifest['catalogs'].items():
            self.assertEqual(hashlib.sha256(outputs[entry['file']]).hexdigest(), entry['sha256'])
        self.assertEqual(manifest['qualification']['native_acceptance'], False)
        self.assertEqual(manifest['source_sha256'], self.spec.sources)
        self.assertEqual(outputs, self.api.render(self.inputs, self.spec))

    def test_digest_duplicate_identity_and_foreign_kit_fail_closed(self):
        changed = dict(self.inputs, headers=self.inputs['headers'] + b'changed')
        with self.assertRaises(ValueError): self.api.render(changed, self.spec)
        for key, data in [('headers', self.inputs['headers'] + self.inputs['headers']),
                          ('classes', self.inputs['classes'].replace(struct.pack('>I', zlib.crc32(b'synthetic-a')), b'\xff' * 4)),
                          ('equipment', self.inputs['equipment'] + self.inputs['equipment'])]:
            bad = dict(self.inputs, **{key: data})
            hashes = dict(self.spec.sources, **{key: hashlib.sha256(data).hexdigest()})
            spec = self.api.SourceSpec(self.spec.name, self.spec.engine, hashes, (1, 1, 1, 1), 4, 3, 3)
            with self.subTest(key=key), self.assertRaises(ValueError): self.api.render(bad, spec)

    def test_cli_rejects_unqualified_inputs_without_outputs_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            paths = {key: root / (key + '.input') for key in self.inputs}
            for key, path in paths.items(): path.write_bytes(self.inputs[key])
            output = root / 'new-output'
            args = sum(([f'--{key}', str(path)] for key, path in paths.items()), []) + ['--output-dir', str(output)]
            with self.assertRaises(ValueError): self.api.main(args)
            self.assertFalse(output.exists())
            self.api.write_outputs(output, {'synthetic.json': b'{}\n'})
            with self.assertRaises(ValueError): self.api.write_outputs(output, {'synthetic.json': b'other'})
            self.assertEqual((output / 'synthetic.json').read_bytes(), b'{}\n')


if __name__ == '__main__':
    unittest.main()
