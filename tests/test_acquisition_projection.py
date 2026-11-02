"""Invented Acquisition graph; no original map coordinates, tokens or records."""
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    'acquisition_projector', ROOT / 'tools/extraction/build-retail-spawn-catalog.py')
API = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(API)


def invented():
    points, groups = [], []

    def point(kind, category, team, active=True, user=0):
        i = len(points)
        row = dict(catalog_id=f'mag_voz_final:{i:03d}', name_token=hex(100 + i),
                   spawn_type_name=kind, spawn_type={'SpawnPointBasic': 0,
                       'SpawnPointAPC': 3, 'SpawnPointHelo': 4,
                       'SpawnPointFieldHospital': 8, 'SpawnPointMotorPool': 7}[kind],
                   category=category, user_class=user, active=active,
                   position_xyz=[100. + i, 10., 100.], team=team,
                   faction=None if team == 0 else 'SVER', squad_links=[])
        points.append(row)
        return row

    for team in (0, 1):
        for platoon in (0, 1):
            for squad in ('Alpha', 'Bravo', 'Charlie', 'Delta'):
                insertion = 'SpawnPointAPC' if team == 0 and squad in ('Bravo', 'Charlie') else 'SpawnPointBasic'
                rows = [point(insertion, 0, team), point('SpawnPointBasic', 1, team),
                        point('SpawnPointHelo' if team == 0 else 'SpawnPointFieldHospital',
                              2, team, active=team == 1)]
                for slot, row in enumerate(rows):
                    row['squad_links'].append(dict(team=team, platoon_id=platoon,
                                                   squad=squad, slot=slot))
                groups.append(dict(team=team, platoon_id=platoon, squad=squad, points=rows))
    # Unlinked authored points remain in the pool, including vehicle-user points.
    for i in range(24):
        point('SpawnPointMotorPool' if i % 2 else 'SpawnPointBasic', 0, i % 2,
              user=i % 2)
    row = dict(map=dict(map='mag_voz_final', owner_faction='SVER', mode_enum='GameType128'),
               points=points, source=dict(archive_sha256='a' * 64, clv_sha256='b' * 64),
               objectives=dict(definitions=[{}], instances=[{}]))
    rules = dict(maximum_points=[1200, 1000], spawn_wave_ms=[20000, 20000],
                 initial_spawn_delay_ms=[35000, 20000])
    return row, groups, 'c' * 64, rules, dict(authored_maximum_players=128,
                                             authored_minimum_players=102)


class AcquisitionProjectionTests(unittest.TestCase):
    def project(self, data=None, attacker='Valor', active=(1, 1, 3)):
        with patch.object(API, 'extract', return_value=data or invented()):
            return API.build(Path('/unused'), Path('/unused'), 'mag_voz_final',
                             *active, attacking_faction=attacker)

    def test_all_records_and_two_platoons_retained_without_selectable_vehicles(self):
        result = self.project()
        pool = result['maps'][0]
        self.assertEqual(pool['spawn_count'], 72)
        self.assertEqual(len(pool['locations']), 72)
        self.assertFalse(pool['runtime_indices_verified'])
        self.assertEqual([s['index'] for s in pool['squads']], list(range(8)) + list(range(16, 24)))
        original = invented()[0]['points']
        unsupported = {i for i, p in enumerate(original)
                       if p['spawn_type_name'] != 'SpawnPointBasic' or p['user_class'] != 0}
        for i in unsupported:
            self.assertEqual(pool['locations'][i]['kind'], 'unsupported')
            self.assertEqual(pool['locations'][i]['active'], original[i]['active'])
        for squad in pool['squads']:
            self.assertFalse(unsupported.intersection(squad['available']))
            self.assertIn(squad['default_spawn'], squad['available'])
            self.assertEqual(squad['platoon'], (squad['index'] % 16) // 4)
        profile = result['match_profile']
        self.assertEqual((profile['game_type'], profile['attacking_faction']), (1, 0))
        self.assertEqual((profile['team_capacity'], profile['squad_capacity']), (32, 4))
        self.assertEqual(profile['initial_spawn_delay_ms'], [35000, 20000])
        self.assertFalse(profile['has_bunkers'])
        self.assertFalse(profile['has_objectives'])
        self.assertEqual(result['active']['squad_index'], 23)
        source = json.loads(result['source'])
        self.assertEqual(len(source['default_spawn_fallbacks']), 4)
        self.assertEqual(len(source['unsupported_spawn_links']), len(unsupported))
        self.assertLess(len(result['source'].encode()), 8192)

    def test_attacker_must_be_explicit_and_distinct(self):
        for attacker in (None, 'SVER', 'invalid'):
            with self.subTest(attacker=attacker):
                with self.assertRaisesRegex(SystemExit, 'attacking faction'):
                    self.project(attacker=attacker)
        self.assertEqual(self.project(attacker='Raven')['match_profile']['attacking_faction'], 1)

    def test_missing_safe_vehicle_fallback_refuses_projection(self):
        data = invented()
        group = data[1][1]
        group['points'][1]['active'] = False
        with self.assertRaisesRegex(SystemExit, 'supported active.*default'):
            self.project(data)

    def test_unknown_kind_is_retained_unavailable_instead_of_becoming_basic(self):
        data = invented()
        row = data[0]['points'][-1]
        row['spawn_type_name'] = 'FutureVehicleKind'
        row['spawn_type'] = 111
        result = self.project(data)
        self.assertEqual(result['maps'][0]['locations'][-1]['kind'], 'unsupported')

    def test_acquisition_rules_keep_authored_capacity_separate_from_local_policy(self):
        raw = b'''GameCreationPropertiesTable={m_gameCreationTypes={Invented={GameCreationType={
          m_gameType="GameType128",m_minPlayers=90,m_maxPlayers=128,m_gameRules={
          StartingPointsTeam0=900,StartingPointsTeam1=800,
          SpawnIntervalTeam0=18000,SpawnIntervalTeam1=19000,
          SpawnInitialDelayTeam0=32000,SpawnInitialDelayTeam1=21000}}}}}'''
        with patch.object(API, 'SETTINGS_SHA256', hashlib.sha256(raw).hexdigest()):
            profile, source = API.original_rules(raw, 'GameType128')
        self.assertEqual(profile, dict(maximum_points=[900, 800], spawn_wave_ms=[18000, 19000],
                                       initial_spawn_delay_ms=[32000, 21000]))
        self.assertEqual(source['authored_maximum_players'], 128)
        self.assertEqual(source['authored_minimum_players'], 90)
        with self.assertRaisesRegex(SystemExit, 'Wrong retail'):
            API.original_rules(raw, 'GameType128')

    def test_invalid_active_squad_and_ambiguous_default_fail_closed(self):
        with self.assertRaisesRegex(SystemExit, 'Active squad'):
            self.project(active=(1, 2, 0))
        data = invented()
        group = data[1][1]
        extra = dict(group['points'][1], catalog_id='mag_voz_final:072', name_token=hex(200))
        data[0]['points'].append(extra)
        group['points'].append(extra)
        with self.assertRaisesRegex(SystemExit, 'supported active.*default'):
            self.project(data)


if __name__ == '__main__':
    unittest.main()
