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

    def test_optional_single_drop_is_explicit_and_preserves_walking_policy(self):
        project = runpy.run_path(str(SCRIPT))["project"]
        query = lambda *_: reply([(2, [1, 2, 3])])
        walking = project(source(), WORLD, parameters(), query)
        self.assertNotIn("allowed_drop_id", walking)
        with self.assertRaises(ValueError):
            project(source(), WORLD, {**parameters(), "allowed_drop_id": 57}, query)
        enabled = {**parameters(), "allowed_drop_id": 57, "planning_window_ms": 1500}
        drop = project(source(), WORLD, enabled, query)
        self.assertEqual(drop.pop("allowed_drop_id"), 57)
        self.assertEqual(drop.pop("planning_window_ms"), 1500)
        self.assertEqual(drop, walking)
        for value in (0, -1, True, "57", 2**32, [57, 58]):
            with self.subTest(value=value), self.assertRaises(ValueError):
                project(source(), WORLD, {**enabled, "allowed_drop_id": value}, query)
        for value in (999, 10001, 1901, True, "1500"):
            with self.subTest(window=value), self.assertRaises(ValueError):
                project(source(), WORLD, {**enabled, "planning_window_ms": value}, query)
        for retry in (1500, 1501):
            with self.subTest(retry=retry), self.assertRaises(ValueError):
                project(source(), WORLD, {**enabled, "retry_ms": retry}, query)
        with self.assertRaises(ValueError):
            project(source(), WORLD, {**parameters(), "planning_window_ms": 1500}, query)

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

    def test_coverage_sidecar_is_passed_to_goal_queries_only_when_supplied(self):
        command = runpy.run_path(str(SCRIPT))["goals_command"]
        plain = command("/cli", "/mesh", "/world", None, [1, 2, 3], 0.75)
        self.assertEqual(plain, ["/cli", "goals", "--mesh", "/mesh", "--collision-world", "/world",
                                 "--goal", "1,2,3", "--goal-radius", "0.75"])
        covered = command("/cli", "/mesh", "/world", "/coverage", [1, 2, 3], 0.75)
        self.assertEqual(covered[:8], ["/cli", "goals", "--mesh", "/mesh", "--collision-world", "/world",
                                       "--collision-coverage", "/coverage"])
        self.assertEqual(covered[8:], plain[6:])

