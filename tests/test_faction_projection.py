"""All-faction projector contracts use invented maps, positions and tokens only."""
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    'faction_projector', ROOT / 'tools/extraction/build-retail-spawn-catalog.py')
API = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(API)


def invented(mode='GameType64', owner='Valor', name='invented_map'):
    points, groups = [], []
    for team in (0, 1):
        for platoon in range(2 if mode == 'GameType128' else 1):
            for squad in ('Alpha', 'Bravo', 'Charlie', 'Delta'):
                rows = []
                for category in (0, 1, 2):
                    i = len(points)
                    row = dict(catalog_id=f'{name}:{i:03d}', name_token=hex(900 + i),
                               spawn_type_name='SpawnPointBasic' if category < 2 else 'FutureKind',
                               spawn_type=0 if category < 2 else 111, category=category,
                               user_class=0, active=True, position_xyz=[float(i), 2., 3.],
                               team=team, faction=owner if team or mode == 'GameTypeFactionExercise' else None,
                               squad_links=[dict(team=team, platoon_id=platoon, squad=squad, slot=category)])
                    rows.append(row)
                    points.append(row)
                groups.append(dict(team=team, platoon_id=platoon, squad=squad, points=rows))
    return (dict(map=dict(map=name, mode_enum=mode, owner_faction=owner), points=points,
                 source=dict(archive_sha256='a' * 64, clv_sha256='b' * 64),
                 objectives=dict(definitions=[{}] * 2, instances=[{}] * 3)),
            groups, 'c' * 64, dict(maximum_points=[100, 90], spawn_wave_ms=[11000, 12000],
                                  initial_spawn_delay_ms=[13000, 14000]), {})


def archive(resources):
    payloads = [('\n'.join(resources) + '\n').encode(), *resources.values()]
    count = len(payloads)
    toc = 32 + 30 * count + 2 * count
    raw = bytearray(struct.pack('>4sI4sIIIII', b'PSAR', 0x10003, b'zlib', toc, 30, count, 65536, 0))
    offset = toc
    for i, value in enumerate(payloads):
        raw.extend(bytes(16) + struct.pack('>I', i) + len(value).to_bytes(5, 'big') + offset.to_bytes(5, 'big'))
        offset += len(value)
    for value in payloads:
        raw.extend(len(value).to_bytes(2, 'big'))
    raw.extend(b''.join(payloads))
    return bytes(raw)


class FactionProjectionTests(unittest.TestCase):
    def project(self, data, attacker=None):
        with patch.object(API, 'extract', return_value=data):
            return API.build(Path('/unused'), Path('/unused'), data[0]['map']['map'],
                             1, 0, 0, attacking_faction=attacker)

    def test_each_faction_defends_its_map_against_each_other_faction_in_both_modes(self):
        # A hard-coded SVER map guard or attacker default must not select the sides.
        for mode, game_type, squad_capacity, count in (
                ('GameType64', 0, 8, 24), ('GameType128', 1, 4, 48)):
            for owner, defender in (('Valor', 0), ('Raven', 1), ('SVER', 2)):
                for attacker, attacker_id in (('Valor', 0), ('Raven', 1), ('SVER', 2)):
                    if attacker == owner:
                        continue
                    with self.subTest(mode=mode, owner=owner, attacker=attacker):
                        result = self.project(invented(mode, owner), attacker)
                        profile, pool = result['match_profile'], result['maps'][0]
                        self.assertEqual(profile['attacking_faction'], attacker_id)
                        self.assertEqual(pool['owner_faction'], defender)
                        self.assertEqual(profile['game_type'], game_type)
                        self.assertEqual((profile['team_capacity'], profile['squad_capacity']), (32, squad_capacity))
                        self.assertEqual(profile['initial_spawn_delay_ms'], [13000, 14000])
                        self.assertEqual(pool['spawn_count'], count)
                        self.assertEqual(len(pool['locations']), count)
                        self.assertFalse(pool['runtime_indices_verified'])
                        self.assertFalse(profile['has_objectives'])
                        self.assertFalse(profile['has_bunkers'])
                        for squad in pool['squads']:
                            self.assertEqual(squad['faction'], defender if squad['team'] else None)
                            self.assertEqual(len(squad['available']), 2)
                            self.assertIn(squad['default_spawn'], squad['available'])
                            for index in squad['available']:
                                self.assertNotEqual(pool['locations'][index]['kind'], 'unsupported')
                        self.assertEqual(len(pool['squads']), 16 if game_type == 1 else 8)
                        source = json.loads(result['source'])
                        self.assertEqual(source['authored_objectives']['definitions'], 2)
                        self.assertEqual(source['authored_objectives']['direct_instances'], 3)
                        self.assertEqual(len(source['unsupported_spawn_links']), count // 3)

    def test_new_opposing_profiles_require_explicit_distinct_attacker(self):
        for mode in ('GameType64', 'GameType128'):
            for owner in ('Valor', 'Raven', 'SVER'):
                for attacker in (None, owner, 'invalid'):
                    with self.subTest(mode=mode, owner=owner, attacker=attacker):
                        with self.assertRaisesRegex(SystemExit, 'attacking faction'):
                            self.project(invented(mode, owner), attacker)

    def test_legacy_syr_darya_default_stays_valor_but_can_explicitly_select_raven(self):
        data = invented(owner='SVER', name='mag_cosmodrome_final')
        # The existing source-bound Syr Darya objective check remains supported.
        data[0]['objectives'] = dict(definitions=[{}] * 13, instances=[{}] * 13)
        for row in data[0]['points']:
            if row['category'] == 2:
                row.update(spawn_type_name='SpawnPointLALODropZone', spawn_type=5, active=False)
        self.assertEqual(self.project(data)['match_profile']['attacking_faction'], 0)
        self.assertEqual(self.project(data, 'Raven')['match_profile']['attacking_faction'], 1)
        with self.assertRaisesRegex(SystemExit, 'attacking faction'):
            self.project(data, 'SVER')

    def test_suppression_retains_owner_faction_and_rejects_explicit_attacker(self):
        data = invented('GameTypeFactionExercise', 'SVER')
        for row in data[0]['points']:
            if row['category'] == 2:
                row.update(spawn_type_name='SpawnPointBasic', spawn_type=0, category=1, active=False)
        result = self.project(data)
        self.assertEqual(result['match_profile']['attacking_faction'], 2)
        self.assertEqual(result['match_profile']['game_type'], 3)
        with self.assertRaises(SystemExit):
            self.project(data, 'Valor')

    def test_non_player_basic_spawn_remains_unavailable_in_sabotage(self):
        data = invented()
        row = data[0]['points'][2]
        row.update(spawn_type_name='SpawnPointBasic', spawn_type=0, category=1, user_class=1)
        result = self.project(data, 'Raven')
        self.assertEqual(result['maps'][0]['locations'][2]['kind'], 'unsupported')
        self.assertNotIn(2, result['maps'][0]['squads'][0]['available'])

    def test_extraction_accepts_configured_modes_without_map_name_allowlist(self):
        # Run the real archive/settings readers on invented archives. Only the
        # external spawn-record parser is replaced; original assets are absent.
        for mode in ('GameType64', 'GameType128'):
            raw = ('''GameCreationPropertiesTable={m_gameCreationTypes={Invented={GameCreationType={
                m_gameType="%s",m_minPlayers=2,m_maxPlayers=64,
                m_gameRules={StartingPointsTeam0=10,StartingPointsTeam1=10,
                SpawnIntervalTeam0=10,SpawnIntervalTeam1=10,
                SpawnInitialDelayTeam0=10,SpawnInitialDelayTeam1=10},
                maps={GameLevelData={m_levelName="invented_map",m_displayName="Invented",
                m_owningFaction="FactionTypeBlack"}}}}}}''' % mode).encode()
            game = archive({'/run/mag/ps3/settings/gameproperties.zpr': raw})
            with tempfile.TemporaryDirectory() as directory:
                disc = Path(directory)
                game_path = disc / 'game.psarc'
                game_path.write_bytes(game)
                level = disc / 'PS3_GAME/USRDIR/run/mag/ps3/levels/invented_map.psarc'
                level.parent.mkdir(parents=True)
                level.write_bytes(archive({'/invented.clv': b'invented empty clv'}))
                x = API.extractor()
                points = invented(mode, 'Raven')[0]
                points['source']['clv_member'] = '/invented.clv'
                with patch.object(API, 'GAME_ARCHIVE_SHA256', hashlib.sha256(game).hexdigest()), \
                     patch.object(API, 'SETTINGS_SHA256', hashlib.sha256(raw).hexdigest()), \
                     patch.object(API, 'extractor', return_value=x), \
                     patch.object(x, 'extract_map', return_value=points):
                    result = API.extract(disc, game_path, 'invented_map')
                self.assertEqual(result[0]['map']['owner_faction'], 'Raven')
                self.assertEqual(len(result[1]), 16 if mode == 'GameType128' else 8)
                self.assertEqual(result[0]['objectives']['definitions'], [])


if __name__ == '__main__':
    unittest.main()
