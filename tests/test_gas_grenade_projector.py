"""Tests for gas projection using synthetic authored rows and proof inputs."""

from pathlib import Path
import runpy
import unittest


TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-gas-grenades.py'


def purchase_inputs():
    items, ids, original, skill = {}, {}, {}, {}
    purchases = {'network_item_id_to_available_item_bit': {}, 'equipment_token_item_ids': {}}
    armory = {'items': [], 'factions': []}
    for faction, color in enumerate(('Green', 'Black', 'Red')):
        namespace = f'FactionItemList{faction + 1}'
        ref = f'{namespace}:7'
        wire = (faction + 1) * 1024 + 17
        name = f'synthetic_grenade_{faction}'
        ids[ref] = wire
        items[ref] = {'authored_ref': ref, 'name': name, 'item_type': 'kWeapon',
                      'definition': {'group': 'weapons', 'name': name}}
        original[namespace] = {'items': {'ItemListData7': {'id': name, 'item_type': 'kWeapon'}}}
        skill[color] = {'Available': True, 'Equipment': {'Element0': {'element': name}}}
        purchases['network_item_id_to_available_item_bit'][str(wire)] = 17
        purchases['equipment_token_item_ids'][name] = [17]
        armory['items'].append({'wire_id': wire, 'authored_ref': ref, 'name': name,
                                'available_bit': 17, 'factions': [faction]})
        armory['factions'].append({'faction': faction, 'available_item_bits': [3, 16]})
    return items, ids, original, skill, purchases, armory


def ammo_inputs():
    weapon = {
        'id': 'synthetic_grenade', 'ammo_name': 'synthetic_cloud',
        'category': 'kWepCategoryGrenade', 'faction': 'FactionTypeGreen',
        'menu_selection_slot': 'kSelectionMenuSlotEquipment',
        'character_attachment_type': 'kCharAttachTypeDefault',
        'fire_param_id': 'kFireParamSecondary',
        'fire_mode': {'fire_modes': {'FireModeTypeClass0': {'FireModeTypeClass': {
            'fire_mode_value': 'kGrenade',
        }}}},
        'magazine_capacity': 1, 'initial_magazine_count': 3, 'infinite_ammo': False,
        'bSupportLightAmmoPack': False,
    }
    ammo = {
        'id': 'synthetic_cloud', 'propulsion_strategy': 'kWepPropulsionStrategyThrown',
        'detonation_strategy': 'kWepDetonationStrategyTimer',
        'explosion_strategy': 'kWepExplosionStrategyVolume',
        'm_eAmmoPersistance': 'kWepPersistanceGrenadeGasPoison',
        'detonation_timer': 4.25, 'm_fLifetimeMax': 16, 'm_fDuration': 24, 'm_fInitialDelay': .75,
        'half_damage_radius': 3.5, 'full_damage_radius': 6.25,
        'destroy_objective': False, 'doHearingLoss': False,
        'damage': {key: 0 for key in ('character', 'characterAPierce', 'characterARedux',
                                     'vehicle', 'vehicleAPierce', 'vehicleARedux',
                                     'objective', 'objectiveAPierce', 'objectiveARedux', 'mine')},
    }
    ammo['damage']['character'] = 32
    native = {'capacity': 3, 'delay_s': .75, 'interval_s': .25, 'limit_s': 24,
              'inner_radius_m': 3.5, 'inner_scale': .375, 'outer_radius_m': 6.25,
              'outer_scale': .125, 'character_amount_per_pulse': 32}
    return weapon, ammo, 'Green', native


class GasGrenadeProjectorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tool = runpy.run_path(str(TOOL))

    def test_projects_separate_authored_lifetimes_and_native_pulse(self):
        self.assertEqual(self.tool['project_parameters'](*ammo_inputs()), {
            'initial_count': 3, 'fuse_ms': 4250, 'projectile_lifetime_ms': 16000,
            'volume_duration_ms': 24000, 'warmup_ms': 750, 'interval_ms': 250,
            'inner_radius_m': 3.5, 'inner_scale': .375, 'outer_radius_m': 6.25,
            'outer_scale': .125, 'character_hp_per_pulse': 32,
        })

    def test_rejects_different_ammo_roles_faction_and_nondamage_channels(self):
        for scope, field, changed in (
            (0, 'faction', 'FactionTypeRed'), (0, 'ammo_name', 'other'),
            (0, 'menu_selection_slot', 'kSelectionMenuSlotPrimary'),
            (0, 'bSupportLightAmmoPack', True), (0, 'magazine_capacity', True),
            (1, 'explosion_strategy', 'kWepExplosionStrategyDebris'),
            (1, 'm_eAmmoPersistance', 'kWepPersistanceGrenadeSmoke'),
            (1, 'destroy_objective', True), (1, 'doHearingLoss', True),
            (1, 'half_damage_radius_scale', .25),
        ):
            args = ammo_inputs()
            args[scope][field] = changed
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.tool['project_parameters'](*args)
        for channel in ('vehicle', 'characterARedux', 'mine'):
            args = ammo_inputs()
            args[1]['damage'][channel] = 1
            with self.subTest(channel=channel), self.assertRaises(ValueError):
                self.tool['project_parameters'](*args)

    def test_rejects_timing_volume_shape_and_source_disagreement(self):
        for index, field, changed in (
            (1, 'm_fLifetimeMax', 3), (1, 'm_fInitialDelay', float('nan')),
            (1, 'm_fDuration', 31), (1, 'half_damage_radius', 5),
            (3, 'interval_s', .0005), (3, 'inner_radius_m', 7),
            (3, 'outer_scale', 2), (3, 'character_amount_per_pulse', 31),
        ):
            args = ammo_inputs()
            args[index][field] = changed
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.tool['project_parameters'](*args)

    def test_selects_only_exact_purchased_faction_item_and_availability_bit(self):
        self.assertEqual(
            [(row['faction'], row['item_id'], row['available_bit'])
             for row in self.tool['select_purchased'](*purchase_inputs())],
            [(0, 1041, 17), (1, 2065, 17), (2, 3089, 17)],
        )
        for change in ('disabled_grant', 'cross_faction', 'nonweapon', 'stock_bit',
                       'wrong_purchase_bit', 'duplicate_original', 'duplicate_armory_faction'):
            _, _, original, skill, purchases, armory = args = purchase_inputs()
            if change == 'disabled_grant':
                skill['Green']['Available'] = False
            elif change == 'cross_faction':
                skill['Green']['Equipment'] = skill['Black']['Equipment']
            elif change == 'nonweapon':
                original['FactionItemList1']['items']['ItemListData7']['item_type'] = 'kAttachmentItem'
            elif change == 'stock_bit':
                armory['factions'][0]['available_item_bits'].append(17)
            elif change == 'wrong_purchase_bit':
                purchases['network_item_id_to_available_item_bit']['1041'] = 18
            elif change == 'duplicate_original':
                rows = original['FactionItemList1']['items']
                rows['ItemListData8'] = rows['ItemListData7'].copy()
            else:
                armory['factions'][2]['faction'] = 0
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.tool['select_purchased'](*args)

    def test_rejects_unreviewed_native_evidence(self):
        with self.assertRaises(ValueError):
            self.tool['qualified_native'](b'{}', b'{}', '0' * 64)


if __name__ == '__main__':
    unittest.main()
