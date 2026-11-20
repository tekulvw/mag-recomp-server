"""Synthetic inputs for the Domination metadata join."""
from pathlib import Path
import runpy
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[1] / 'tools/extraction'


def sources():
    source = {'archive_sha256': 'a' * 64, 'clv_sha256': 'b' * 64,
              'clv_member': '/levels/invented.clv'}
    points = [
        {'name_token': '0x00000001', 'team': 1, 'squad_links': [
            {'team': 1, 'platoon_id': 3, 'squad': 'Delta', 'slot': 0}]},
        {'name_token': '0x00000002', 'team': 0, 'squad_links': []},
    ]
    definition = {'definition_token': '0x00000003', 'runtime_pool_id': None,
                  'class_name': 'ObjectiveDemolitionStructuralProperties', 'fields': {
                      'ForwardSpawnPoint': {'token': '0x00000001'},
                      'OwnerTeam': {'value': 1},
                  }}
    instance = {'name_token': '0x00000004', 'definition_token': '0x00000003',
                'runtime_pool_id': None, 'fields': []}
    row = {'map': {'map': 'invented', 'mode_enum': 'GameType256', 'owner_faction': 'Valor'},
           'source': source, 'points': points, 'runtime_pool_indices_verified': False,
           'unresolved_squad_references': [], 'bunkers': [],
           'objectives': {'definitions': [definition], 'instances': [instance]}}
    triggers = {'map': 'invented', 'source': dict(source), 'objectives': []}
    return row, triggers


class DominationMapProjectionTests(unittest.TestCase):
    def setUp(self):
        path = TOOLS / 'extract-retail-domination.py'
        self.assertTrue(path.is_file(), 'Domination extraction entry point is missing')
        self.project = runpy.run_path(str(path))['project_map']

    def test_joins_forward_spawn_by_token_and_preserves_fourth_platoon(self):
        row, triggers = sources()
        result = self.project(row, triggers)
        self.assertEqual(result['forward_spawns'], [
            {'objective_token': '0x00000004', 'spawn_token': '0x00000001'}])
        self.assertEqual(result['squads'][0]['platoon'], 3)
        self.assertEqual(result['squads'][0]['spawns'], ['0x00000001'])
        self.assertFalse(result['runtime_pool_indices_verified'])

    def test_rejects_trigger_geometry_from_a_different_clv(self):
        row, triggers = sources()
        triggers['source']['clv_sha256'] = 'c' * 64
        with self.assertRaisesRegex(ValueError, 'source mismatch'):
            self.project(row, triggers)

    def test_rejects_missing_forward_spawn_and_duplicate_squad_slots(self):
        row, triggers = sources()
        row['objectives']['definitions'][0]['fields']['ForwardSpawnPoint']['token'] = '0x00000099'
        with self.assertRaisesRegex(ValueError, 'forward spawn'):
            self.project(row, triggers)
        row, triggers = sources()
        row['points'][0]['squad_links'] *= 2
        with self.assertRaisesRegex(ValueError, 'squad slot'):
            self.project(row, triggers)

    def test_authored_rules_are_read_from_mode_table(self):
        raw = b'''GameCreationPropertiesTable={m_gameCreationTypes={Invented={GameCreationType={
            m_gameType="GameType256",m_minPlayers=101,m_maxPlayers=203,m_gameRules={
            StartingPointsTeam0=777,StartingPointsTeam1=678,
            SpawnIntervalTeam0=1000,SpawnIntervalTeam1=2000,
            SpawnInitialDelayTeam0=300,SpawnInitialDelayTeam1=400,
            PointsLostPerDeathTeam0=6,PointsLostPerDeathTeam1=7}}}}}'''
        builder = runpy.run_path(str(TOOLS / 'build-retail-spawn-catalog.py'))
        original_rules = builder['original_rules']
        with patch.dict(original_rules.__globals__, GAME_ARCHIVE_SHA256='unused',
                        SETTINGS_SHA256=builder['sha'](raw)):
            rules, source = original_rules(raw, 'GameType256')
        self.assertEqual(rules, {'maximum_points': [777, 678], 'spawn_wave_ms': [1000, 2000],
                                'initial_spawn_delay_ms': [300, 400], 'points_lost_per_death': [6, 7]})
        self.assertEqual(source['authored_maximum_players'], 203)
