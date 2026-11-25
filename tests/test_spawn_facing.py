"""Synthetic spawn-facing projection checks. Invented catalogs only; no original records."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / 'tools/extraction/add-spawn-facing.py'
SPEC = importlib.util.spec_from_file_location('spawn_facing', TOOL)
API = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(API)

# Invented authored rotations (degrees) and the packed yaw each must produce.
ROTATIONS = [
    ([0.0, 0.0, 0.0], 0),                  # faces +z
    ([0.0, 90.0, 0.0], 256),               # faces +x
    ([0.0, -90.0, 0.0], 768),              # faces -x
    ([-180.0, 0.9, -180.0], 509),          # flipped X/Z encode a turn of 179.1 degrees: faces -z
    ([180.0, 88.0, 180.0], 262),           # flipped X/Z with a near quarter turn
    ([0.0, 45.0, 30.0], 116),              # the Z turn is applied last and shifts the heading
    ([10.0, -40.0, 0.0], 910),             # pitch is applied first, so the heading is the Y turn
    ([-20.0, 30.0, 15.0], 69),             # pitch, turn and roll together
]


def invented():
    source = {'maps': [{'map': {'map': 'invented_map'}, 'points': [
        {'name_token': f'0x{0x100 + i:08x}', 'rotation_xyz': rotation}
        for i, (rotation, _) in enumerate(ROTATIONS)]}]}
    catalog = {'format': 'mag_spawn_catalog', 'schema_version': 1, 'source': 'SYNTHETIC',
               'active': {'map_name': 'invented_map', 'squad_index': 0, 'default_spawn': 0, 'available': [0]},
               'maps': [{'name': 'invented_map', 'spawn_count': len(ROTATIONS), 'locations': [
                   {'index': i, 'token': 0x100 + i, 'kind': 'insertion', 'active': True,
                    'position': {'x': 10.0 + i, 'y': 2.0, 'z': 20.0}}
                   for i in range(len(ROTATIONS))]}]}
    return catalog, source


class SpawnFacingTests(unittest.TestCase):
    def test_packed_yaw_matches_the_wire_convention(self):
        for rotation, expected in ROTATIONS:
            with self.subTest(rotation=rotation):
                self.assertEqual(API.packed_yaw(rotation), expected)

    def test_adds_yaw_by_token_and_keeps_everything_else(self):
        catalog, source = invented()
        result = API.produce(catalog, source)
        locations = result['maps'][0]['locations']
        self.assertEqual([row['yaw'] for row in locations], [yaw for _, yaw in ROTATIONS])
        for row in locations:
            del row['yaw']
        self.assertEqual(result, catalog)

    def test_refuses_missing_points_and_conflicting_yaw(self):
        catalog, source = invented()
        source['maps'][0]['points'].pop()
        with self.assertRaises(SystemExit):
            API.produce(catalog, source)
        catalog, source = invented()
        catalog['maps'][0]['locations'][1]['yaw'] = 3
        with self.assertRaises(SystemExit):
            API.produce(catalog, source)
        with self.assertRaises(SystemExit):
            API.packed_yaw([90.0, 0.0, 0.0])  # facing straight down has no heading

    def test_cli_writes_and_checks(self):
        catalog, source = invented()
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            (work / 'catalog.json').write_text(json.dumps(catalog))
            (work / 'source.json').write_text(json.dumps(source))
            command = [sys.executable, str(TOOL), '--catalog', str(work / 'catalog.json'),
                       '--source', str(work / 'source.json'), '--output', str(work / 'out.json')]
            subprocess.run(command, check=True, capture_output=True)
            subprocess.run(command + ['--check'], check=True, capture_output=True)
            refused = subprocess.run(command, capture_output=True)
            self.assertNotEqual(refused.returncode, 0)
            written = json.loads((work / 'out.json').read_text())
            self.assertEqual(written['maps'][0]['locations'][3]['yaw'], 509)


if __name__ == '__main__':
    unittest.main()
