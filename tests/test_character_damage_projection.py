"""Incoming damage policy must carry the selected original mode and exact profile."""
import json
from pathlib import Path
import runpy
import unittest


class CharacterDamageProjectionTests(unittest.TestCase):
    def setUp(self):
        path = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-character-damage-rules.py'
        self.assertTrue(path.is_file())
        self.project = runpy.run_path(str(path))['project_rules']
        self.spawn = {'maps': [{'name': 'invented'}], 'match_profile': {'game_type': 2},
                      'source': json.dumps({'game_rules_source': {'sha256': 'b'*64,
                          'rules_json_pointer': '/synthetic/rules'}})}
        self.source = {'sha256': 'b'*64, 'rules_json_pointer': '/synthetic/rules'}

    def test_preserves_fraction_and_exact_selected_source(self):
        output = self.project({'FriendlyFireDamageMultiplier': 0.375}, self.source,
                              self.spawn, 'a'*64, 'c'*64)
        self.assertEqual(output['friendly_fire_multiplier'], 0.375)
        self.assertEqual(output['game_type'], 2)
        self.assertEqual(output['provenance']['spawn_catalog_sha256'], 'a'*64)
        self.assertEqual(output['provenance']['json_pointer'],
                         '/synthetic/rules/FriendlyFireDamageMultiplier')

    def test_rejects_foreign_rules_and_nonfinite_or_boolean_values(self):
        self.source['sha256'] = 'd'*64
        with self.assertRaisesRegex(ValueError, 'source'):
            self.project({'FriendlyFireDamageMultiplier': 0.375}, self.source, self.spawn, 'a'*64, 'c'*64)
        self.source['sha256'] = 'b'*64
        for value in (True, float('nan'), float('inf'), -1):
            with self.assertRaises(ValueError):
                self.project({'FriendlyFireDamageMultiplier': value}, self.source, self.spawn, 'a'*64, 'c'*64)
