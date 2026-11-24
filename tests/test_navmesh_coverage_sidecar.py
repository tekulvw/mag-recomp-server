"""Synthetic checks for collision coverage sidecars and whole-map preparation."""
import copy
import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
PREPARE = ROOT / "tools/navmesh/prepare.py"
MAKE = ROOT / "tools/navmesh/make-coverage-sidecar.py"
SHA = "a" * 64


def world(side=18):
    return {"format": "mag_collision_world", "schema_version": 1, "coordinate_space": "world",
            "metadata": {"map_name": "synthetic", "build": "synthetic", "coverage": "provisional",
                         "provenance": ["synthetic"], "unknowns": [], "physics_regions": []},
            "instances": [{"id": "floor", "vertices_xyz": [[0, 2, 0], [0, 2, side], [side, 2, 0]],
                           "triangles": [[0, 1, 2]]}]}


def region(minimum, maximum):
    return {"min": minimum, "max": maximum, "ground_verified": True, "obstacles_verified": True,
            "provenance": ["synthetic assertion"]}


def sidecar(world_sha256=SHA, regions=None):
    return {"format": "mag_collision_coverage_v1", "map_name": "synthetic", "world_sha256": world_sha256,
            "basis": "operator_assertion", "provenance": ["synthetic"],
            "physics_regions": regions or [region([-1, -1, -1], [19, 6, 19])]}


class CoverageSidecarTests(unittest.TestCase):
    def setUp(self):
        self.tools = runpy.run_path(str(PREPARE))

    def test_sidecar_validation_binds_world_hash_and_map(self):
        validate = self.tools["validate_coverage_sidecar"]
        self.assertEqual(validate(sidecar(), SHA, "synthetic"), sidecar()["physics_regions"])
        with self.assertRaisesRegex(ValueError, "world mismatch"):
            validate(sidecar(), "b" * 64, "synthetic")
        with self.assertRaisesRegex(ValueError, "world mismatch"):
            validate(sidecar(), SHA, "elsewhere")
        bad_region = region([5, 0, 0], [1, 1, 1])
        far = region([0, 0, 0], [1, 1, 200_000])
        for key, value in [("format", "mag_collision_coverage_v2"), ("basis", "guess"),
                           ("provenance", []), ("physics_regions", []),
                           ("physics_regions", [bad_region]), ("physics_regions", [far]),
                           ("physics_regions", [dict(region([0, 0, 0], [1, 1, 1]), extra=1)]),
                           ("world_sha256", "A" * 64), ("unknown", True)]:
            invalid = sidecar()
            invalid[key] = value
            with self.assertRaisesRegex(ValueError, "invalid collision coverage sidecar", msg=key):
                validate(invalid, SHA, "synthetic")

    def test_prepare_unions_sidecar_regions_with_the_world_regions(self):
        prepare = self.tools["prepare"]
        source = world()
        with self.assertRaisesRegex(ValueError, "no verified static coverage"):
            prepare(source, SHA)
        unchanged = copy.deepcopy(source)
        value, report = prepare(source, SHA, coverage=sidecar())
        self.assertEqual(source, unchanged)
        self.assertEqual(value["physics_regions"], sidecar()["physics_regions"])
        self.assertEqual(value["bounds"], {"min": [-1, -1, -1], "max": [19, 6, 19]})
        self.assertEqual(report["coverage_sidecar_regions"], 1)
        self.assertEqual(report["runtime_layout"], "tiled_256")
        own = region([100, 0, 100], [110, 5, 110])
        source["metadata"]["physics_regions"] = [own]
        value, _ = prepare(source, SHA, coverage=sidecar())
        self.assertEqual(value["physics_regions"], [own] + sidecar()["physics_regions"])
        self.assertEqual(value["bounds"], {"min": [100, 0, 100], "max": [110, 5, 110]})
        with self.assertRaisesRegex(ValueError, "world mismatch"):
            prepare(world(), SHA, coverage=sidecar("b" * 64))
        with self.assertRaisesRegex(ValueError, "never claims coverage"):
            prepare(world(), SHA, reference=True, coverage=sidecar())

    def test_prepare_reports_the_runtime_grid_layout_and_refuses_over_512_tiles(self):
        prepare, grid_tiles = self.tools["prepare"], self.tools["grid_tiles"]
        self.assertEqual(grid_tiles({"min": [0, 0, 0], "max": [819.2, 1, 819.2]}), 256)
        self.assertEqual(grid_tiles({"min": [0, 0, 0], "max": [870, 1, 870]}), 289)
        wide = sidecar(regions=[region([0, -3, 0], [870, 3, 870])])
        _, report = prepare(world(870), SHA, coverage=wide)
        self.assertEqual((report["runtime_grid_tiles"], report["runtime_layout"]), (289, "wide_512"))
        too_wide = sidecar(regions=[region([0, -3, 0], [1200, 3, 1200])])
        with self.assertRaisesRegex(ValueError, "512-tile"):
            prepare(world(870), SHA, coverage=too_wide)


class MakeCoverageSidecarTests(unittest.TestCase):
    def run_tool(self, *args):
        return subprocess.run([sys.executable, "-I", str(MAKE), *map(str, args)],
                              capture_output=True, text=True, check=False)

    def test_whole_world_sidecar_requires_an_assertion_and_the_exact_world(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "world.json"
            source.write_text(json.dumps(world()))
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            output = directory / "coverage.json"
            missing = self.run_tool("--world", source, "--expected-world-sha256", digest,
                                    "--output", output, "--whole-world")
            self.assertNotEqual(missing.returncode, 0)
            self.assertIn("--operator-assertion", missing.stderr)
            wrong = self.run_tool("--world", source, "--expected-world-sha256", "b" * 64, "--output", output,
                                  "--whole-world", "--operator-assertion", "synthetic")
            self.assertNotEqual(wrong.returncode, 0)
            self.assertIn("world source hash mismatch", wrong.stderr)
            self.assertFalse(output.exists())
            made = self.run_tool("--world", source, "--expected-world-sha256", digest, "--output", output,
                                 "--whole-world", "--operator-assertion", "synthetic review",
                                 "--region", -5, -1, -5, 5, 4, 5)
            self.assertEqual(made.returncode, 0, made.stderr)
            value = json.loads(output.read_text())
            self.assertEqual(value["world_sha256"], digest)
            self.assertEqual(value["basis"], "operator_assertion")
            self.assertEqual([r["min"] for r in value["physics_regions"]], [[0, -1, 0], [-5, -1, -5]])
            self.assertEqual(value["physics_regions"][0]["max"], [18, 5, 18])
            self.assertIn("operator assertion: synthetic review", value["provenance"])
            self.assertEqual(json.loads(made.stdout)["sidecar_sha256"],
                             hashlib.sha256(output.read_bytes()).hexdigest())
            again = self.run_tool("--world", source, "--expected-world-sha256", digest, "--output", output,
                                  "--whole-world", "--operator-assertion", "synthetic")
            self.assertNotEqual(again.returncode, 0)
            prepare = runpy.run_path(str(PREPARE))["prepare"]
            prepared, _ = prepare(world(), digest, coverage=value)
            self.assertEqual(prepared["bounds"], {"min": [0, -1, 0], "max": [18, 5, 18]})

    def test_prepare_cli_records_the_sidecar_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "world.json"
            source.write_text(json.dumps(world()))
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            coverage = directory / "coverage.json"
            coverage.write_text(json.dumps(sidecar(digest)))
            output = directory / "prep"
            result = subprocess.run([sys.executable, "-I", str(PREPARE), "--world", str(source),
                                     "--expected-world-sha256", digest, "--collision-coverage", str(coverage),
                                     "--output", str(output)], capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            manifest = json.loads((output / "manifest.json").read_text())
            self.assertEqual(manifest["coverage_sidecar_sha256"], hashlib.sha256(coverage.read_bytes()).hexdigest())
            prepared = json.loads((output / "input.json").read_text())
            self.assertEqual(prepared["physics_regions"], sidecar(digest)["physics_regions"])
            both = subprocess.run([sys.executable, "-I", str(PREPARE), "--world", str(source),
                                   "--expected-world-sha256", digest, "--collision-coverage", str(coverage),
                                   "--reference", "--output", str(directory / "other")],
                                  capture_output=True, text=True, check=False)
            self.assertNotEqual(both.returncode, 0)


if __name__ == "__main__":
    unittest.main()
