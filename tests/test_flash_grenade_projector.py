"""Synthetic authored inputs for the retail Flash projector."""
from copy import deepcopy
from pathlib import Path
import runpy
import unittest


TOOLS = Path(__file__).resolve().parents[1] / 'tools/extraction'


def recruit_inputs():
    items, ids, original = {}, {}, {}
    armory = {'items': [], 'factions': []}
    recruit = {'Tier': 0, 'Cost': 0}
    availability = {'network_item_id_to_available_item_bit': {}, 'equipment_token_item_ids': {}}
    qualified = []
    for faction, color in enumerate(('Green', 'Black', 'Red')):
        group = f'FactionItemList{faction + 1}'
        name = f'synthetic_equipment_{faction}'
        ref = f'{group}:8'
        wire = (faction + 1) * 1024 + 24
        items[ref] = {'authored_ref': ref, 'name': name, 'item_type': 'kWeapon',
                      'definition': {'group': 'weapons', 'name': name}}
        original[group] = {'items': {'ItemListData8': {'id': name, 'item_type': 'kWeapon'}}}
        ids[ref] = wire
        recruit[color] = {'Available': True, 'Equipment': {
            'Element0': {'element': 'unrelated_grant'}, 'Element1': {'element': name},
        }}
        availability['network_item_id_to_available_item_bit'][str(wire)] = 24
        availability['equipment_token_item_ids'][name] = [24]
        armory['items'].append({'wire_id': wire, 'name': name, 'available_bit': 24,
                                'authored_ref': ref, 'factions': [faction]})
        armory['factions'].append({'faction': faction, 'available_item_bits': [24]})
        qualified.append({'item': wire, 'weapon': name, 'ammo': f'synthetic_ammo_{faction}'})
    return items, ids, original, recruit, availability, armory, qualified


def authored_inputs():
    weapon = {
        'id': 'synthetic_equipment', 'ammo_name': 'synthetic_flash',
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
        'id': 'synthetic_flash', 'propulsion_strategy': 'kWepPropulsionStrategyThrown',
        'detonation_strategy': 'kWepDetonationStrategyTimer',
        'explosion_strategy': 'kWepExplosionStrategyDebris',
        'detonation_timer': 4.25, 'm_fLifetimeMax': 16,
        'm_fDuration': 2.5, 'm_fInitialDelay': .3,
        'half_damage_radius': 3.5, 'full_damage_radius': 6.25, 'MaterialPenetration': 1.125,
        'destroy_objective': False, 'doHearingLoss': True,
        'damage': {key: 0 for key in (
            'character', 'characterAPierce', 'characterARedux', 'vehicle',
            'vehicleAPierce', 'vehicleARedux', 'objective', 'objectiveAPierce',
            'objectiveARedux', 'mine',
        )},
    }
    return weapon, ammo, 'Green'


class FlashGrenadeProjectorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tool = runpy.run_path(str(TOOLS / 'project-retail-flash-grenades.py'))

    def test_faction_recruit_grant_requires_default_armory_availability(self):
        source = recruit_inputs()
        variants = self.tool['select_recruit'](*source)
        self.assertEqual([row['faction'] for row in variants], [0, 1, 2])
        self.assertEqual([row['available_bit'] for row in variants], [24, 24, 24])
        for bad_index, mutation, error in [
            (3, lambda x: x['Black']['Equipment'].pop('Element1'), 'Recruit'),
            (5, lambda x: x['factions'][1]['available_item_bits'].clear(), 'available'),
            (6, lambda x: x[1].update(item=x[0]['item']), 'faction'),
        ]:
            with self.subTest(error=error):
                altered = deepcopy(source)
                mutation(altered[bad_index])
                with self.assertRaisesRegex(ValueError, error):
                    self.tool['select_recruit'](*altered)

    def test_projects_authored_variant_and_the_two_immediate_durations(self):
        variant, durations = self.tool['project_parameters'](*authored_inputs())
        self.assertEqual(variant, {'initial_count': 3, 'fuse_ms': 4250,
                                   'projectile_lifetime_ms': 16000, 'inner_radius_m': 3.5,
                                   'outer_radius_m': 6.25, 'material_penetration': 1.125})
        self.assertEqual(durations, {'strong_duration_s': 2.5, 'weak_duration_s': .3})

    def test_rejects_persistent_damage_role_and_invalid_authored_bounds(self):
        weapon, ammo, color = authored_inputs()
        for patch, diagnostic in [
            ({'explosion_strategy': 'kWepExplosionStrategyVolume'}, 'explosion_strategy'),
            ({'doHearingLoss': False}, 'hearing'),
            ({'damage': {**ammo['damage'], 'mine': 1}}, 'non-damaging'),
            ({'m_fInitialDelay': 2.5}, 'durations'),
            ({'m_fDuration': 32}, 'strong duration'),
            ({'MaterialPenetration': float('nan')}, 'penetration'),
            ({'half_damage_radius': 6.25}, 'radii'),
        ]:
            with self.subTest(patch=patch):
                with self.assertRaisesRegex(ValueError, diagnostic):
                    self.tool['project_parameters'](weapon, {**ammo, **patch}, color)

    def test_rejects_unreviewed_original_proof_before_reading_native_values(self):
        with self.assertRaisesRegex(ValueError, 'unreviewed original flash'):
            self.tool['qualified_native'](b'changed', b'{}', b'{}', object())
