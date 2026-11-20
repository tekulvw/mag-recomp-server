import json
from pathlib import Path
import runpy
import unittest


TOOLS = Path(__file__).resolve().parents[1] / 'tools' / 'extraction'
PROJECT = runpy.run_path(str(TOOLS / 'project-retail-domination-ground-lifecycle.py'))
RULES = {'sha256': 'source', 'rules_json_pointer': '/original/literal'}


def fixture():
    points = [{'index': index + 23, 'token': index + 900} for index in range(16)]
    spawns = dict(
        format='mag_spawn_catalog', active={'map_name': 'test_map'},
        match_profile={'game_type': 2, 'spawn_wave_ms': [2000, 3000], 'attacking_faction': 2},
        maps=[dict(name='test_map', owner_faction=1, runtime_indices_verified=True,
                   locations=points)],
        source=json.dumps(dict(map='test_map', game_archive_sha256='archive',
                               game_rules_source=RULES, domination_ground_sources_sha256='ground')))
    raw = json.dumps(spawns).encode()
    ground = dict(
        format='mag_ground_vehicles',
        source=dict(creation_scope='native_domination_vehicle_source_points',
                    spawn_catalog_sha256=PROJECT['sha'](raw), native_elf_sha256='elf',
                    native_binding_sha256='ground'),
        vehicles=[], vehicle_sources=[])
    for index, point in enumerate(points):
        team = int(index >= 8)
        point_fields = dict(spawn_point_index=point['index'], spawn_point_token=point['token'],
                            vehicle_type_token=102 + team, team=team)
        ground['vehicles'].append(dict(index=index, faction=2 - team,
                                       health_quarter_points=200 + team, **point_fields))
        ground['vehicle_sources'].append(dict(
            vehicle_index=index, spawn_type=(3, 7)[team], user_class=team,
            source_family=35, authored_active=True, **point_fields))
    return raw, ground


class DominationGroundLifecycleProjectorTests(unittest.TestCase):
    def test_joins_native_indices_and_vehicle_variant_health(self):
        raw, ground = fixture()
        identity, source_types, health = PROJECT['project_profile'](
            raw, json.dumps(ground), 'elf', 'archive', RULES, [2000, 3000])
        self.assertEqual(identity, ('test_map', 2, 1))
        self.assertEqual(list(source_types.values()), [3] * 8 + [7] * 8)
        self.assertEqual(health, {(102, 2): 200, (103, 1): 201})

    def test_rejects_same_slot_with_different_point_token(self):
        raw, ground = fixture()
        ground['vehicles'][0]['spawn_point_token'] += 1
        with self.assertRaisesRegex(ValueError, 'differs from its bound source point'):
            PROJECT['project_profile'](
                raw, json.dumps(ground), 'elf', 'archive', RULES, [2000, 3000])

    def test_rejects_profile_with_a_different_wave_interval(self):
        raw, ground = fixture()
        with self.assertRaisesRegex(ValueError, 'original wave settings'):
            PROJECT['project_profile'](
                raw, json.dumps(ground), 'elf', 'archive', RULES, [2000, 3001])
