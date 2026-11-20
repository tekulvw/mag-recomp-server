"""Synthetic checks for the leadership election policy projector."""
import importlib.util
from pathlib import Path
import unittest


TOOL = Path(__file__).resolve().parents[1] / "tools/extraction/project-retail-leadership-role-policy.py"
SPEC = importlib.util.spec_from_file_location("role_policy", TOOL)
PROJECTOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROJECTOR)


class LeadershipRolePolicyTest(unittest.TestCase):
    def rules(self, **platoon_weights):
        return {
            "schema_version": 1,
            "application_id": 20231,
            "resources": {
                PROJECTOR.MEMBER: {
                    "sha256": "a" * 64,
                    "parameters": {"MagSquadMatchmakingConfigSPropertyTable": {
                        "SquadLeader_LevelWeight": 4,
                        "SquadLeader_MSLCWeight": 2.5,
                        **platoon_weights,
                    }},
                },
            },
        }

    def test_schema_three_projects_both_authored_platoon_weights(self):
        policy = PROJECTOR.project(self.rules(
            PlatoonLeader_LevelWeight=3.25, PlatoonLeader_MSLCWeight=7,
        ), {"leadership_level": 1, "nominations": 1, "matches_since_last_command": 0}, platoon_only=True)
        self.assertEqual(policy["schema_version"], 3)
        self.assertEqual(policy["platoon_election"], {"level_weight": 3.25, "mslc_weight": 7.0})
        self.assertEqual(policy["native_election"]["source_sha256"], "a" * 64)

    def test_missing_platoon_weight_fails_explicitly(self):
        with self.assertRaisesRegex(ValueError, "invalid authored PlatoonLeader_MSLCWeight"):
            PROJECTOR.project(self.rules(PlatoonLeader_LevelWeight=3.25), {}, platoon_only=True)

    def test_schema_four_uses_original_company_weight_keys(self):
        policy = PROJECTOR.project(self.rules(
            PlatoonLeader_LevelWeight=3.25, PlatoonLeader_MSLCWeight=7,
            TeamLeader_LevelWeight=9, TeamLeader_MSLCWeight=0.5,
        ), {})
        self.assertEqual(policy["schema_version"], 4)
        self.assertEqual(policy["company_election"], {"level_weight": 9.0, "mslc_weight": 0.5})
