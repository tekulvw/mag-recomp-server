import copy
from pathlib import Path
import runpy
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "tools/navmesh/prepare.py"
SHA = "a" * 64


def world():
    return {"format": "mag_collision_world", "schema_version": 1, "coordinate_space": "world",
            "metadata": {"map_name": "synthetic", "physics_regions": []},
            "instances": [{"id": "terrain:synthetic",
                           "vertices_xyz": [[0, 2, 0], [0, 2, 11], [18, 2, 0]],
                           "triangles": [[0, 1, 2], [0, 2, 1]]}]}


class NavmeshReferencePreparationTests(unittest.TestCase):
    def test_explicit_reference_retains_full_faces_without_claiming_coverage(self):
        prepare = runpy.run_path(str(SCRIPT))["prepare"]
        source = world()
        with self.assertRaisesRegex(ValueError, "no verified static coverage"):
            prepare(source, SHA)
        unchanged = copy.deepcopy(source)
        value, report = prepare(source, SHA, reference=True)
        self.assertEqual(source, unchanged)
        self.assertEqual(value["format"], "mag_navmesh_reference_bake_input_v1")
        self.assertEqual(value["world_sha256"], SHA)
        self.assertEqual(value["physics_regions"], [])
        self.assertEqual(value["triangles"], source["instances"][0]["triangles"])
        self.assertEqual(value["bounds"], {"min": [0, -1, 0], "max": [18, 5, 11]})
        self.assertEqual(report["source_geometry_bounds"], {"min": [0, 2, 0], "max": [18, 2, 11]})

    def test_reference_cannot_narrow_bounds_and_default_held_regions_are_retained(self):
        prepare = runpy.run_path(str(SCRIPT))["prepare"]
        source = world()
        small = {"min": [1, 0, 1], "max": [2, 3, 2]}
        with self.assertRaisesRegex(ValueError, "reference bounds"):
            prepare(source, SHA, small, reference=True)
        region = {"min": [-1, -2, -1], "max": [19, 7, 12], "ground_verified": True,
                  "obstacles_verified": True, "provenance": ["synthetic"]}
        source["metadata"]["physics_regions"] = [region]
        value, _ = prepare(source, SHA)
        self.assertEqual(value["format"], "mag_navmesh_bake_input_v1")
        self.assertEqual(value["physics_regions"], [region])
        self.assertEqual(value["bounds"], {key: region[key] for key in ("min", "max")})
