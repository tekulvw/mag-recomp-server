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

    def test_authored_selection_keeps_every_placement_with_valid_flags(self):
        module = runpy.run_path(str(SCRIPT))
        project, authored = module["provider"], module["AUTHORED"]
        values = {"Name": "0x12345678", "IsVisible_M": False, "IsFrozen_M": True,
                  "SnapToTerrain_M": True, "Scale": 1.0}
        for prefix, vector in (("Translate", [1.0, 2.0, 3.0]), ("Rotate", [0.0, 0.0, 0.0]),
                               ("Scale", [4.0, 2.0, 4.0])):
            values.update({prefix + axis: value for axis, value in zip("XYZ", vector)})
        row = {"offset": "0xa0", "source_sha256": "b"*64, "preceding_token": 21,
               "fields": [{"name": k, "value": v} for k, v in values.items()]}
        definitions = {21: {"offset": 32, "source_sha256": "d"*64, "tick_rate_seconds": 1.0}}
        self.assertIsNone(project(row, definitions))
        self.assertEqual(project(row, definitions, authored)["center"], [1.0, 2.0, 3.0])
        bad = copy.deepcopy(row)
        next(field for field in bad["fields"] if field["name"] == "IsVisible_M")["value"] = 0
        with self.assertRaises(ValueError):
            project(bad, definitions, authored)
        with self.assertRaises(ValueError):
            project(row, definitions, "synthetic_selection")

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

    def test_each_map_has_its_own_pins_and_bindings(self):
        import json
        import re

        module = runpy.run_path(str(SCRIPT))
        self.assertEqual(module["DEFAULT_MAP"], "mag_cosmodrome_final")
        counts = {name: (module["map_pins"](name)["mode"], module["map_pins"](name)["providers"])
                  for name in module["QUALIFIED"]}
        self.assertEqual(counts, {
            "mag_cosmodrome_final": (0, 3), "mag_kennicott_final": (0, 2),
            "mag_remote_comms_final": (0, 3),
            "mag_alaska_subpen_final": (1, 10), "mag_panama_military_base_final": (1, 9),
            "mag_voz_final": (1, 11),
            "mag_baku_final": (2, 20), "mag_canal_final": (2, 20), "mag_unalaska_final": (2, 21)})
        digests = set()
        for name in counts:
            pins = module["map_pins"](name)
            if pins["mode"] == 0:
                level = module["SABOTAGE_MAPS"][name]
                self.assertEqual(pins["selection"], "visible_unfrozen_unsnapped")
                self.assertEqual(pins["level_sha256"], level["level_sha256"])
                self.assertEqual(pins["clv_sha256"], level["clv_sha256"])
            else:
                level = module["ACQDOM_MAPS"][name]
                self.assertEqual(pins["selection"], "authored_identity_parent")
                self.assertEqual(pins["mode"], level.mode)
                self.assertEqual(pins["providers"], level.provider_count)
                self.assertEqual(pins["level_sha256"], level.archive_sha)
                self.assertEqual(pins["clv_sha256"], level.clv_sha)
            self.assertEqual(pins["member"], f"/run/mag/ps3/levels/{name}/{name}.clv")
            for key in ("level_sha256", "clv_sha256", "qualification_sha256"):
                self.assertRegex(pins[key], re.compile("^[0-9a-f]{64}$"))
                digests.add(pins[key])
        self.assertEqual(len(digests), 3 * len(counts))
        self.assertEqual(module["map_pins"]("mag_cosmodrome_final")["qualification_sha256"],
                         "f24127b06399d1353e4bb1c1a73943278227cd31a1488eda409096099b089b0d")

        validate = module["validate_bindings"]
        spawns = {"active": {"map_name": "mag_kennicott_final"},
                  "match_profile": {"game_type": 0}}
        collision = {"format": "mag_collision_world", "coordinate_space": "world",
                     "metadata": {"map_name": "mag_kennicott_final"}}
        validate(json.dumps(spawns), json.dumps(collision), "mag_kennicott_final")
        with self.assertRaises(ValueError):
            validate(json.dumps(spawns), json.dumps(collision))
        with self.assertRaises(ValueError):
            validate(json.dumps(spawns), json.dumps(collision), "mag_remote_comms_final")
        collision["metadata"]["map_name"] = "mag_remote_comms_final"
        with self.assertRaises(ValueError):
            validate(json.dumps(spawns), json.dumps(collision), "mag_kennicott_final")

        # Acquisition and Domination maps bind only their own mode.
        for name, mode in (("mag_voz_final", 1), ("mag_canal_final", 2)):
            spawns = {"active": {"map_name": name}, "match_profile": {"game_type": mode}}
            collision = {"format": "mag_collision_world", "coordinate_space": "world",
                         "metadata": {"map_name": name}}
            validate(json.dumps(spawns), json.dumps(collision), name)
            for other in (0, 3 - mode, True):
                spawns["match_profile"]["game_type"] = other
                with self.subTest(map=name, mode=other), self.assertRaises(ValueError):
                    validate(json.dumps(spawns), json.dumps(collision), name)

    def test_unqualified_maps_are_rejected(self):
        import json

        module = runpy.run_path(str(SCRIPT))
        for name in ("mag_sabotage_unknown_final", "synthetic_map"):
            with self.assertRaises(ValueError):
                module["map_pins"](name)
            spawns = {"active": {"map_name": name}, "match_profile": {"game_type": 0}}
            collision = {"format": "mag_collision_world", "coordinate_space": "world",
                         "metadata": {"map_name": name}}
            with self.assertRaises(ValueError):
                module["validate_bindings"](json.dumps(spawns), json.dumps(collision), name)

    def test_runtime_catalog_pins_match_the_projector(self):
        import re

        module = runpy.run_path(str(SCRIPT))
        source = (SCRIPT.parents[1] / "gameplay/src/bot_resupply_catalog.rs").read_text()
        rows = re.findall(
            r'MapPins \{\s*map: "([^"]+)",\s*mode: (\d+),\s*level_sha256: "([0-9a-f]{64})",\s*'
            r'clv_sha256: "([0-9a-f]{64})",\s*qualification_sha256: "([0-9a-f]{64})",\s*'
            r'providers: (\d+),\s*\}', source)
        expected = []
        for name in module["QUALIFIED"]:
            pins = module["map_pins"](name)
            expected.append((name, str(pins["mode"]), pins["level_sha256"], pins["clv_sha256"],
                             pins["qualification_sha256"], str(pins["providers"])))
        self.assertEqual(rows, expected)
