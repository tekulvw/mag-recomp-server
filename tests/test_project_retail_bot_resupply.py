import copy
from pathlib import Path
import runpy
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "tools/project-retail-bot-resupply.py"


class StaticSupplyProjectorTests(unittest.TestCase):
    def test_selection_and_authored_nonuniform_scale(self):
        project = runpy.run_path(str(SCRIPT))["provider"]
        values = {"Name": "0x12345678", "IsVisible_M": True, "IsFrozen_M": False,
                  "SnapToTerrain_M": False, "Scale": 9.0}
        for prefix, vector in (("Translate", [1.0, 2.0, 3.0]), ("Rotate", [0.0, 90.0, 0.0]),
                               ("Scale", [6.0, 4.0, 2.0])):
            values.update({prefix + axis: value for axis, value in zip("XYZ", vector)})
        row = {"offset": "0xa0", "source_sha256": "b"*64, "preceding_token": 21,
               "fields": [{"name": k, "value": v} for k, v in values.items()]}
        definitions = {21: {"offset": 32, "source_sha256": "d"*64, "tick_rate_seconds": 1.25}}
        result = project(row, definitions)
        self.assertEqual(result["center"], [1.0, 2.0, 3.0])
        self.assertEqual(result["scales_xyz"], [6.0, 4.0, 2.0])
        self.assertAlmostEqual(result["axes"][0][2], -1.0)
        self.assertEqual(result["tick_rate_seconds"], 1.25)
        self.assertEqual(result["tolerance_m"], 1.0)
        for name, value in (("IsVisible_M", False), ("IsFrozen_M", True), ("SnapToTerrain_M", True)):
            candidate = copy.deepcopy(row)
            next(field for field in candidate["fields"] if field["name"] == name)["value"] = value
            self.assertIsNone(project(candidate, definitions))
        invalid = copy.deepcopy(row)
        next(field for field in invalid["fields"] if field["name"] == "ScaleY")["value"] = -4
        with self.assertRaises(ValueError):
            project(invalid, definitions)

    def test_additional_inputs_must_select_the_same_sabotage_world(self):
        import json

        validate = runpy.run_path(str(SCRIPT))["validate_bindings"]
        spawns = {"active": {"map_name": "mag_cosmodrome_final"},
                  "match_profile": {"game_type": 0}}
        collision = {"format": "mag_collision_world", "coordinate_space": "world",
                     "metadata": {"map_name": "mag_cosmodrome_final"}}
        validate(json.dumps(spawns), json.dumps(collision))
        collision["metadata"]["map_name"] = "somewhere_else"
        with self.assertRaises(ValueError):
            validate(json.dumps(spawns), json.dumps(collision))
        collision["metadata"]["map_name"] = "mag_cosmodrome_final"
        spawns["match_profile"]["game_type"] = False
        with self.assertRaises(ValueError):
            validate(json.dumps(spawns), json.dumps(collision))
