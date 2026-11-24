"""Sabotage map generalisation checks; invented archives and values only."""
import hashlib
import importlib.util
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools/collision'))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


OBJECTIVES = load('sabotage_objectives', ROOT / 'tools/extraction/project-retail-sabotage-objectives.py')
SPAWNS = load('sabotage_spawns', ROOT / 'tools/extraction/build-retail-spawn-catalog.py')
import retail_compiled  # noqa: E402

SABOTAGE = ('mag_cosmodrome_final', 'mag_kennicott_final', 'mag_remote_comms_final')


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


def settings(map_name, owner):
    return ('''GameCreationPropertiesTable={m_gameCreationTypes={Invented={GameCreationType={
        m_gameType="GameType64",m_minPlayers=2,m_maxPlayers=64,
        maps={GameLevelData={m_levelName="%s",m_displayName="Invented",
        m_owningFaction="%s"}}}}}}''' % (map_name, owner)).encode()


class SabotageObjectiveProfileTests(unittest.TestCase):
    def test_every_supported_map_pins_identity_and_cardinality(self):
        self.assertEqual(tuple(OBJECTIVES.SABOTAGE_MAPS), SABOTAGE)
        for name in SABOTAGE:
            with self.subTest(map=name):
                profile = OBJECTIVES.map_profile(name)
                self.assertEqual(profile['level'], f'PS3_GAME/USRDIR/run/mag/ps3/levels/{name}.psarc')
                self.assertEqual(profile['clv'], f'/run/mag/ps3/levels/{name}/{name}.clv')
                for key in ('level_sha256', 'clv_sha256'):
                    self.assertRegex(profile[key], r'^[0-9a-f]{64}$')
                for key in ('objectives', 'records', 'controls', 'triggers', 'forward_spawns'):
                    self.assertIsNotNone(profile[key], key)

    def test_existing_module_identity_still_selects_the_original_map(self):
        self.assertEqual(OBJECTIVES.MAP, 'mag_cosmodrome_final')
        profile = OBJECTIVES.map_profile(OBJECTIVES.MAP)
        self.assertEqual((OBJECTIVES.LEVEL, OBJECTIVES.LEVEL_SHA, OBJECTIVES.CLV, OBJECTIVES.CLV_SHA),
                         (profile['level'], profile['level_sha256'], profile['clv'], profile['clv_sha256']))

    def test_three_defenders_give_all_six_ordered_faction_pairs(self):
        factions = {'FactionTypeGreen': 'Valor', 'FactionTypeBlack': 'Raven', 'FactionTypeRed': 'SVER'}
        defenders = [factions[OBJECTIVES.SABOTAGE_MAPS[name]['owner_faction_enum']] for name in SABOTAGE]
        self.assertEqual(sorted(defenders), sorted(factions.values()))
        pairs = {(attacker, defender) for defender in defenders
                 for attacker in factions.values() if attacker != defender}
        self.assertEqual(len(pairs), 6)

    def test_unknown_map_and_changed_counts_are_refused(self):
        with self.assertRaisesRegex(ValueError, 'Unsupported retail Sabotage map'):
            OBJECTIVES.map_profile('invented_map')
        profile = OBJECTIVES.map_profile('mag_kennicott_final')
        OBJECTIVES.cardinality(profile, 'records', profile['records'], 'unused')
        with self.assertRaisesRegex(ValueError, 'Unexpected objective cardinality'):
            OBJECTIVES.cardinality(profile, 'objectives', profile['objectives'] + 1,
                                   'Unexpected objective cardinality')

    def test_cli_rejects_unknown_map_before_reading_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(
                [sys.executable, str(ROOT / 'tools/extraction/project-retail-sabotage-objectives.py'),
                 '--disc', directory, '--game-archive', str(Path(directory) / 'absent.psarc'),
                 '--map', 'invented_map', '--output', str(Path(directory) / 'out.json')],
                capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn('invalid choice', result.stderr)
            self.assertFalse((Path(directory) / 'out.json').exists())

    def test_selected_map_checks_its_configured_defender_and_level_identity(self):
        for name, owner, other in (('mag_kennicott_final', 'FactionTypeGreen', 'FactionTypeRed'),
                                   ('mag_remote_comms_final', 'FactionTypeBlack', 'FactionTypeGreen')):
            with tempfile.TemporaryDirectory() as directory:
                disc = Path(directory)
                level = disc / OBJECTIVES.map_profile(name)['level']
                level.parent.mkdir(parents=True)
                level.write_bytes(archive({OBJECTIVES.map_profile(name)['clv']: b'invented clv'}))
                for configured, message in ((other, 'Unexpected configured Sabotage map identity'),
                                            (owner, 'Original resource hash mismatch')):
                    with self.subTest(map=name, configured=configured):
                        game = archive({'/run/mag/ps3/settings/gameproperties.zpr': settings(name, configured)})
                        game_path = disc / 'game.psarc'
                        game_path.write_bytes(game)
                        with patch.object(OBJECTIVES, 'GAME_SHA', hashlib.sha256(game).hexdigest()):
                            with self.assertRaisesRegex(ValueError, message):
                                OBJECTIVES.project(disc, game_path, name)


class SabotageSpawnProfileTests(unittest.TestCase):
    def invented(self, name, definitions, instances, owner):
        points, groups = [], []
        for team in (0, 1):
            for squad in ('Alpha', 'Bravo', 'Charlie', 'Delta'):
                rows = []
                for category in (0, 1):
                    i = len(points)
                    row = dict(catalog_id=f'{name}:{i:03d}', name_token=hex(700 + i),
                               spawn_type_name='SpawnPointBasic', spawn_type=0, category=category,
                               user_class=0, active=True, position_xyz=[float(i), 1., 2.],
                               team=team, faction=owner if team else None,
                               squad_links=[dict(team=team, platoon_id=0, squad=squad, slot=category)])
                    rows.append(row)
                    points.append(row)
                groups.append(dict(team=team, platoon_id=0, squad=squad, points=rows))
        return (dict(map=dict(map=name, mode_enum='GameType64', owner_faction=owner), points=points,
                     source=dict(archive_sha256='a' * 64, clv_sha256='b' * 64),
                     objectives=dict(definitions=[{}] * definitions, instances=[{}] * instances)),
                groups, 'c' * 64, dict(maximum_points=[10, 9], spawn_wave_ms=[1000, 1000],
                                      initial_spawn_delay_ms=[2000, 3000]), {})

    def build(self, data, attacker):
        with patch.object(SPAWNS, 'extract', return_value=data):
            return SPAWNS.build(Path('/unused'), Path('/unused'), data[0]['map']['map'], 1, 0, 0,
                                attacking_faction=attacker)

    def test_new_maps_pin_objective_cardinality_and_require_an_attacker(self):
        self.assertEqual(tuple(SPAWNS.SABOTAGE_OBJECTIVES), SABOTAGE)
        for name, owner, attackers in (('mag_kennicott_final', 'Valor', ('Raven', 'SVER')),
                                       ('mag_remote_comms_final', 'Raven', ('Valor', 'SVER'))):
            definitions, instances = SPAWNS.SABOTAGE_OBJECTIVES[name]
            data = self.invented(name, definitions, instances, owner)
            for attacker in attackers:
                with self.subTest(map=name, attacker=attacker):
                    result = self.build(data, attacker)
                    self.assertEqual(result['match_profile']['attacking_faction'], SPAWNS.FACTIONS[attacker])
                    self.assertEqual(result['maps'][0]['owner_faction'], SPAWNS.FACTIONS[owner])
            with self.subTest(map=name, attacker=None):
                with self.assertRaisesRegex(SystemExit, 'attacking faction'):
                    self.build(data, None)
            with self.subTest(map=name, changed=True):
                changed = self.invented(name, definitions, instances + 1, owner)
                with self.assertRaisesRegex(SystemExit, 'cardinality changed'):
                    self.build(changed, attackers[0])

    def test_unlisted_sabotage_map_keeps_unpinned_cardinality(self):
        result = self.build(self.invented('invented_map', 2, 3, 'Valor'), 'SVER')
        self.assertEqual(result['match_profile']['attacking_faction'], 2)


class CompiledTerrainTokenTests(unittest.TestCase):
    @staticmethod
    def manifest(*tokens):
        packs = [dict(resource_binding=dict(resource_name_token=token, physics_type_raw=kind))
                 for token, kind in tokens]
        return dict(resource_members=[dict(packfiles=packs), dict(packfiles=[dict(source_sha256='d' * 64)])])

    def test_single_bound_terrain_resource_is_selected(self):
        manifest = self.manifest(('0badf00d', 1), ('0badf00d', 1), ('12345678', 0))
        self.assertEqual(retail_compiled.manifest_terrain_token(manifest), '0badf00d')

    def test_absent_or_ambiguous_terrain_is_refused(self):
        for tokens in ((('12345678', 0),), (('0badf00d', 1), ('cafef00d', 1))):
            with self.subTest(tokens=tokens):
                with self.assertRaisesRegex(ValueError, 'exactly one type1 terrain'):
                    retail_compiled.manifest_terrain_token(self.manifest(*tokens))


if __name__ == '__main__':
    unittest.main()
