"""Synthetic checks for the optional guided launcher projection."""
from copy import deepcopy
from pathlib import Path
import runpy
import unittest


PROJECTOR = runpy.run_path(
    str(Path(__file__).resolve().parents[1] / "tools/extraction/project-retail-guided-launchers.py")
)


class GuidedProjection(unittest.TestCase):
    def test_steering_keeps_guided_and_no_lock_strategy_parameters_distinct(self):
        ammo = {
            "rocketGuidedDecisionTimeRange": [.012, .08],
            "rocketGuidedMaxDirectionChange": .06,
            "rocketDumbDecisionTimeRange": [.014, .014],
            "rocketDumbDirectionChangeRange": [.004, .005],
            "rocketDumbCruiseStraightRange": 2.5,
            "rocketAutoExplodeDistance": 140,
        }
        result = PROJECTOR["project_steering"](ammo)
        self.assertEqual(result["guided_decision_min_seconds"], .012)
        self.assertEqual(result["dumb_decision_min_seconds"], .014)
        self.assertEqual(result["dumb_direction_change_max"], .005)
        self.assertEqual(result["dumb_cruise_range_m"], 2.5)
        self.assertEqual(result["dumb_max_range_m"], 140)
        for key, value in {
            "rocketGuidedDecisionTimeRange": [.08, .012],
            "rocketGuidedMaxDirectionChange": float("nan"),
            "rocketDumbDecisionTimeRange": [0, .014],
            "rocketDumbDirectionChangeRange": [-.001, .005],
            "rocketDumbCruiseStraightRange": 140,
        }.items():
            rejected = deepcopy(ammo)
            rejected[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                PROJECTOR["project_steering"](rejected)

    def test_skill_grant_requires_launcher_and_exact_sight(self):
        skill = {
            "Tier": 2, "Cost": 5, "Category": "kSkillCategoryHeavy",
            "Green": {"Available": True, "Equipment": {
                "Element0": {"element": "launcher"}, "Element1": {"element": "launcher_sight"},
            }},
        }
        self.assertEqual(PROJECTOR["select_skill_pair"](skill, "Green"), ("launcher", "launcher_sight"))
        skill["Green"]["Equipment"]["Element2"] = {"element": "other_sight"}
        with self.assertRaises(ValueError):
            PROJECTOR["select_skill_pair"](skill, "Green")

    def test_native_proof_substitution_fails_before_address_reads(self):
        with self.assertRaisesRegex(ValueError, "unreviewed"):
            PROJECTOR["qualified_native"](b"{}", b"{}", b"{}", object())

    def test_all_faction_bindings_preserve_purchase_scope_and_required_sight(self):
        items, ids, original, qualifies = {}, {}, {}, []
        armory = {"items": [], "factions": []}
        availability = {"equipment_token_item_ids": {}}
        skill = {"Tier": 2, "Cost": 5, "Category": "kSkillCategoryHeavy"}
        for faction, color in enumerate(("Green", "Black", "Red")):
            group = f"FactionItemList{faction + 1}"
            weapon, sight, ammo = f"launcher_{faction}", f"sight_{faction}", f"ammo_{faction}"
            first_wire = 180 + 200 * faction
            original[group] = {"items": {}}
            armory["factions"].append({"faction": faction, "available_item_bits": []})
            skill[color] = {"Available": True, "Equipment": {
                "Element0": {"element": weapon}, "Element1": {"element": sight},
            }}
            qualifies.append({"faction": faction, "wire_item": first_wire, "authored_item": f"{group}:0",
                               "item": weapon, "ammo": ammo, "ammo_token": 20 + faction})
            for ordinal, (name, item_type, kind) in enumerate((
                (weapon, "kWeapon", "weapons"), (sight, "kAttachmentItem", "attachments"),
            )):
                ref = f"{group}:{ordinal}"
                items[ref] = {"name": name, "authored_ref": ref, "definition": {"group": kind, "name": name}}
                ids[ref] = first_wire + ordinal
                original[group]["items"][f"ItemListData{ordinal}"] = {"id": name, "item_type": item_type}
                armory["items"].append({"wire_id": first_wire + ordinal, "name": name, "authored_ref": ref,
                                       "factions": [faction], "available_bit": ordinal + 1,
                                       "primary_ammo": 1 if ordinal == 0 else None})
                availability["equipment_token_item_ids"][name] = [ordinal + 1]

        select = PROJECTOR["select_variants"]
        selected = select(items, ids, original, skill, availability, armory, qualifies)
        self.assertEqual([(row["item_id"], row["stock_sight_item_id"]) for row in selected],
                         [(180, 181), (380, 381), (580, 581)])
        wrong_role = deepcopy(original)
        wrong_role["FactionItemList3"]["items"]["ItemListData1"]["item_type"] = "kWeapon"
        with self.assertRaisesRegex(ValueError, "ItemList"):
            select(items, ids, wrong_role, skill, availability, armory, qualifies)
        wrong_default = deepcopy(armory)
        wrong_default["factions"][1]["available_item_bits"].append(2)
        with self.assertRaisesRegex(ValueError, "purchased Armory"):
            select(items, ids, original, skill, availability, wrong_default, qualifies)
