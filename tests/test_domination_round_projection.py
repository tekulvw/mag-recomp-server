"""Synthetic score rules and source joins for the Domination round setup."""
import json
from pathlib import Path
import runpy
import unittest


class RoundProjectionTests(unittest.TestCase):
    def setUp(self):
        path = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-domination-round.py'
        self.assertTrue(path.is_file(), 'round projector is missing')
        self.project = runpy.run_path(str(path))['project_rules']
        self.source = {'sha256': 'a' * 64, 'rules_json_pointer': '/rules/example'}
        self.spawn = {
            'source': json.dumps({'game_rules_source': self.source}),
            'match_profile': {'game_type': 2, 'maximum_points': [700, 600]},
            'maps': [{'name': 'invented'}],
        }
        self.objectives = {'game_type': 2, 'map_name': 'invented', 'source': {
            'authored_catalog_sha256': 'b' * 64,
        }}
        self.rules = {'StartingPointsTeam0': 700, 'StartingPointsTeam1': 600,
                      'MajorVictoryPercent': 43, 'EndGameDelay': 1700}

    def test_carries_rules_and_exact_source_hashes(self):
        result = self.project(self.rules, self.source, self.spawn, self.objectives, 'c' * 64)
        self.assertEqual(result['starting_points'], [700, 600])
        self.assertEqual(result['major_victory_percent'], 43)
        self.assertEqual(result['end_game_delay_ms'], 1700)
        self.assertEqual(result['spawn_catalog_sha256'], 'c' * 64)
        self.assertEqual(result['game_rules_json_pointer'], '/rules/example')

    def test_rejects_cross_map_rules_and_noninteger_score(self):
        self.objectives['map_name'] = 'foreign'
        with self.assertRaisesRegex(ValueError, 'map'):
            self.project(self.rules, self.source, self.spawn, self.objectives, 'c' * 64)
        self.objectives['map_name'] = 'invented'
        self.rules['StartingPointsTeam0'] = 700.5
        with self.assertRaisesRegex(ValueError, 'integer'):
            self.project(self.rules, self.source, self.spawn, self.objectives, 'c' * 64)
