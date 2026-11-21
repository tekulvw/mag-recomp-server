import json
from pathlib import Path
import runpy
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "tools/project-bot-resupply-approach.py"
WORLD, DETOUR = "a" * 64, "b" * 64


def source():
    provider = {"source_offset": 42, "name_token": 57, "center": [1, 2, 3],
                "scales_xyz": [2, 4, 6], "tolerance_m": 0.25,
                "axes": [[1, 0, 0], [0, 1, 0], [0, 0, 1]]}
    return json.dumps({"schema_version": 1, "application_id": 20231,
                       "mode": 0, "map": "mag_cosmodrome_final",
                       "collision_sha256": WORLD, "providers": [provider]}).encode()


def parameters():
    return {"sample_offsets": [[0, 0, 0], [0, 0, 1]], "projection_radius_m": 1.25,
            "query_budget_per_tick": 2, "retry_ms": 75, "goal_radius_m": 0.5,
            "max_route_length_m": 39, "seek_max_duration_ms": 1900}


def reply(rows, collision=WORLD):
    return {"map": "mag_cosmodrome_final", "world_sha256": collision, "detour_sha256": DETOUR,
            "candidates": [{"component": component, "point": point} for component, point in rows]}


class ResupplyApproachProjectorTests(unittest.TestCase):
    def test_filters_against_provider_and_retains_distinct_same_component_samples(self):
        project = runpy.run_path(str(SCRIPT))["project"]
        rows = iter([reply([(4, [1, 2, 3]), (5, [2.5, 2, 3])]),
                     reply([(4, [1, 2, 4]), (5, [1, 2, 3])])])
        policy = project(source(), WORLD, parameters(), lambda *_: next(rows))
        self.assertEqual(policy["providers"], [{"source_offset": 42, "name_token": 57,
                                               "anchors": [[1, 2, 3], [1, 2, 4]]}])
        self.assertEqual(policy["detour_sha256"], DETOUR)
        self.assertEqual(policy["seek_max_duration_ms"], 1900)

    def test_rejects_absent_candidates_and_mismatched_query_provenance(self):
        project = runpy.run_path(str(SCRIPT))["project"]
        for response in (reply([]), reply([(2, [1, 2, 3])], "c" * 64)):
            with self.assertRaises(ValueError):
                project(source(), WORLD, parameters(), lambda *_: response)
        for key, value in (("query_budget_per_tick", True), ("max_route_length_m", 193),
                           ("seek_max_duration_ms", 999), ("seek_max_duration_ms", 120001)):
            bad = {**parameters(), key: value}
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                project(source(), WORLD, bad, lambda *_: reply([(2, [1, 2, 3])]))
