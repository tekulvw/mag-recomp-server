"""Synthetic coverage for validation of the external launcher projection."""
from pathlib import Path
import runpy
import unittest

TOOL = Path(__file__).resolve().parents[1] / "tools/extraction/project-retail-underbarrel-launchers.py"
PROJECTOR = runpy.run_path(str(TOOL))


class UnderbarrelProjectorTests(unittest.TestCase):
    def test_upgraded_host_is_the_unique_faction_rifle_in_the_skill_grant(self):
        find = PROJECTOR["upgraded_host"]
        rows = {"ItemListData3": {"id": "stock", "item_type": "kWeapon"},
                "ItemListData4": {"id": "upgraded", "item_type": "kWeapon"},
                "ItemListData5": {"id": "grip", "item_type": "kAttachmentItem"},
                "ItemListData6": {}}
        weapons = {name: {"category": "kWepCategoryAssaultRifle"}
                   for name in ("stock", "upgraded")}
        skill = {"Available": True, "Equipment": {
            "tier_rifle": {"element": "upgraded"}, "tier_grip": {"element": "grip"}}}
        self.assertEqual(find("FactionItemList2", rows, skill, weapons),
                         ("FactionItemList2:4", "upgraded"))
        for bad in ({**skill, "Available": False}, {**skill, "Equipment": {}},
                    {**skill, "Equipment": {**skill["Equipment"],
                                            "extra": {"element": "stock"}}}):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                find("FactionItemList2", rows, bad, weapons)

    def test_unlocked_host_armory_row_matches_the_original_item_identity(self):
        match = PROJECTOR["inventory_bit"]
        row = {"name": "upgraded", "authored_ref": "FactionItemList1:70",
               "factions": [0], "available_bit": 120}
        purchases = {"network_item_id_to_available_item_bit": {"1144": 120},
                     "equipment_token_item_ids": {"upgraded": [120]}}
        self.assertEqual(match({1144: row}, purchases, 0, 1144,
                               "FactionItemList1:70", "upgraded"), 120)
        for altered in ({**row, "factions": [1]}, {**row, "available_bit": 121},
                        {**row, "authored_ref": "FactionItemList1:71"}):
            with self.subTest(altered=altered), self.assertRaises(ValueError):
                match({1144: altered}, purchases, 0, 1144,
                      "FactionItemList1:70", "upgraded")

    def test_authored_physics_and_damage_channels_remain_independent(self):
        ammo = {**PROJECTOR["AMMO_STRATEGIES"],
                **dict.fromkeys(PROJECTOR["AUTHORED_SCALARS"], 1),
                "id": "synthetic_impact", "capsuleHeight": [0, 0, .12],
                "m_fLifetimeMax": 9, "m_fArmedTime": .3,
                "full_damage_radius": 7, "half_damage_radius": 3,
                "m_fImpactDamage": 43, "destroy_objective": False,
                "damage": dict.fromkeys(PROJECTOR["DAMAGE_CHANNELS"], 0)}
        ammo["damage"].update(character=19, characterARedux=.1,
                              vehicle=700, vehicleARedux=.25)
        projected = PROJECTOR["project_ammo"](ammo, 31337)
        self.assertEqual(projected["token"], 31337)
        authored = projected["authored"]
        self.assertEqual(authored["capsuleHeight"], [0., 0., .12])
        self.assertEqual(authored["m_fImpactDamage"], 43)
        self.assertEqual(authored["damage"], ammo["damage"])
        self.assertEqual(authored["m_fArmedTime"], .3)
        for altered in ({**ammo, "collisionType": "sphere"},
                        {**ammo, "m_fArmedTime": 10},
                        {**ammo, "linearDamping": float("nan")}):
            with self.subTest(altered=altered), self.assertRaises(ValueError):
                PROJECTOR["project_ammo"](altered, 31337)

    def test_launcher_weapon_is_finite_and_distinct_from_host(self):
        host = {"id": "rifle", "category": "kWepCategoryAssaultRifle",
                "ammo_name": "rifle_round",
                "attachment_list": {"row": {"WeaponAttachment": {"name": "launcher"}}}}
        attachment = {"id": "launcher", "weapon_name": "mounted_weapon",
                      "attachment_type": "kAttachTypeLauncher",
                      "character_attachment_type": "kCharAttachTypePrimary"}
        weapon = {"id": "mounted_weapon", "category": "kWepCategoryGrenadeLauncher",
                  "ammo_name": "projectile", "magazine_capacity": 2,
                  "initial_magazine_count": 4, "infinite_ammo": False,
                  "fire_mode": {"refire_time": 0.125, "reload_time": 2}}
        check = PROJECTOR["project_weapon"]
        self.assertEqual(check(host, attachment, weapon)["initial_magazine_count"], 4)
        for altered in (
            {**weapon, "infinite_ammo": True},
            {**weapon, "id": "rifle"},
            {**weapon, "ammo_name": "rifle_round"},
            {**weapon, "fire_mode": {"refire_time": float("nan"), "reload_time": 2}},
        ):
            with self.subTest(altered=altered), self.assertRaises(ValueError):
                check(host, attachment, altered)
        with self.assertRaises(ValueError):
            check(host, {**attachment, "attachment_type": "kAttachTypeNone"}, weapon)

    def test_damage_schema_is_complete_and_numeric(self):
        channels = PROJECTOR["DAMAGE_CHANNELS"]
        damage = dict.fromkeys(channels, 0)
        damage["character"] = 11
        self.assertEqual(PROJECTOR["project_damage"](damage), damage)
        for altered in ({**damage, "mine": True}, {**damage, "future": 0},
                        {key: val for key, val in damage.items() if key != "mine"}):
            with self.subTest(altered=altered), self.assertRaises(ValueError):
                PROJECTOR["project_damage"](altered)
