"""Synthetic Acquisition LALO drop-zone projection checks. Invented records only."""
import copy
import importlib.util
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / 'tools/extraction/project-retail-acquisition-lalo.py'
SPEC = importlib.util.spec_from_file_location('acquisition_lalo', TOOL)
API = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(API)

MAP = 'invented_acquisition'


def drop_zone(index, rotation, link):
    return {'name_token': f'0x{0x200 + index:08x}', 'spawn_type': 5, 'active': False,
            'user_class': 0, 'team': 0, 'category': 2, 'radius': 1.0,
            'snap_to_terrain': False, 'position_xyz': [100.0 + index, 120.0, 200.0],
            'rotation_xyz': rotation, 'squad_links': [link]}


def invented():
    basic = {'name_token': '0x00000200', 'spawn_type': 0, 'active': True, 'user_class': 0,
             'team': 0, 'category': 0, 'radius': 3.0, 'snap_to_terrain': False,
             'position_xyz': [100.0, 80.0, 200.0], 'rotation_xyz': [0.0, 0.0, 0.0],
             'squad_links': [{'team': 0, 'platoon_id': 0, 'squad': 'Alpha', 'slot': 0}]}
    points = [basic,
              drop_zone(1, [0.0, 90.0, 0.0], {'team': 0, 'platoon_id': 0, 'squad': 'Alpha', 'slot': 2}),
              drop_zone(2, [0.0, -1.5, 0.0], {'team': 0, 'platoon_id': 0, 'squad': 'Bravo', 'slot': 2})]
    source = {'map': {'map': MAP}, 'points': points}
    provenance = {'map': MAP, 'unsupported_spawn_links': [
        {'index': i, 'spawn_type': 5, 'category': 2, 'user_class': 0, 'active': False,
         'squad_links': [[0, 0, i - 1, 2]]} for i in (1, 2)]}
    locations = [{'index': i, 'token': int(p['name_token'], 16),
                  'kind': 'insertion' if i == 0 else 'unsupported', 'active': i == 0,
                  'position': dict(zip('xyz', p['position_xyz']))}
                 for i, p in enumerate(points)]
    catalog = {'format': 'mag_spawn_catalog', 'schema_version': 1,
               'source': json.dumps(provenance),
               'match_profile': {'game_type': 1},
               'active': {'map_name': MAP, 'squad_index': 0, 'default_spawn': 0, 'available': [0]},
               'maps': [{'name': MAP, 'spawn_count': 3, 'runtime_indices_verified': True,
                         'locations': locations,
                         'squads': [{'index': 0, 'available': [0], 'default_spawn': 0}]}]}
    return catalog, source


class AcquisitionLaloTests(unittest.TestCase):
    def test_promotes_only_drop_zones_with_the_authored_pose(self):
        catalog, source = invented()
        result = API.produce(catalog, source, b'invented')
        locations = result['maps'][0]['locations']
        self.assertEqual([row['kind'] for row in locations], ['insertion', 'parachute', 'parachute'])
        first, second = locations[1]['parachute'], locations[2]['parachute']
        self.assertEqual(first['spacing_m'], 10.0)
        self.assertEqual(first['yaw'], 256)
        for a, b in zip(first['forward'], [1.0, 0.0, 0.0]):
            self.assertAlmostEqual(a, b)
        # A small negative turn truncates toward zero from just under a full turn.
        self.assertEqual(second['yaw'], int((1 - 1.5 / 360) * 1024))
        self.assertAlmostEqual(math.hypot(*second['forward']), 1.0)
        # Positions stay the authored airborne points; nothing else changes.
        for row in locations:
            row.pop('parachute', None)
            row['kind'] = 'insertion' if row['index'] == 0 else 'unsupported'
        provenance = json.loads(result.pop('source'))
        self.assertEqual(provenance['acquisition_lalo']['points'], [1, 2])
        expected = copy.deepcopy(catalog)
        expected.pop('source')
        self.assertEqual(result, expected)

    def test_refuses_mismatched_or_already_projected_zones(self):
        cases = []
        catalog, source = invented()
        catalog['match_profile']['game_type'] = 2
        cases.append((catalog, source))
        catalog, source = invented()
        source['points'][1]['radius'] = 4.0
        cases.append((catalog, source))
        catalog, source = invented()
        source['points'][2]['active'] = True
        cases.append((catalog, source))
        catalog, source = invented()
        catalog['maps'][0]['locations'][1]['position']['y'] = 0.0
        cases.append((catalog, source))
        catalog, source = invented()
        catalog['maps'][0]['squads'][0]['available'].append(1)
        cases.append((catalog, source))
        catalog, source = invented()
        catalog['source'] = json.dumps({'unsupported_spawn_links': []})
        cases.append((catalog, source))
        catalog, source = invented()
        cases.append((API.produce(catalog, source), source))
        for catalog, source in cases:
            with self.subTest(case=len(cases)), self.assertRaises(SystemExit):
                API.produce(catalog, source)

    def test_cli_writes_and_checks(self):
        catalog, source = invented()
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            (work / 'catalog.json').write_text(json.dumps(catalog))
            (work / 'source.json').write_text(json.dumps({'maps': [source]}))
            command = [sys.executable, str(TOOL), '--catalog', str(work / 'catalog.json'),
                       '--source', str(work / 'source.json'), '--output', str(work / 'out.json')]
            subprocess.run(command, check=True, capture_output=True)
            subprocess.run(command + ['--check'], check=True, capture_output=True)
            self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)


if __name__ == '__main__':
    unittest.main()
