"""Synthetic infantry rocket projection checks, without retail files."""
import copy
from pathlib import Path
import runpy
import unittest


TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-infantry-rockets.py'


def resources():
    weapon = {
        'id': 'synthetic_launcher',
        'category': 'kWepCategoryRocketLauncher',
        'fire_param_id': 'kFireParamSecondary',
        'initial_fire_mode': 'kSemiAuto',
        'ammo_name': 'synthetic_round',
        'use_turret_mesh': False,
        'alternate_firepoint': '',
        'required_sight_attachment': 'synthetic_sight',
        'attachment_list': {'WeaponAttachment0': {
            'WeaponAttachment': {'name': 'synthetic_sight'},
        }},
        'infinite_ammo': False,
        'bSupportLightAmmoPack': False,
        'magazine_capacity': 1,
        'initial_magazine_count': 4,
        'fire_mode': {
            'refire_time': 0.75,
            'reload_time': 2.5,
            'fire_modes': {'FireModeTypeClass0': {
                'FireModeTypeClass': {'fire_mode_value': 'kSemiAuto'},
            }},
        },
    }
    ammo = {
        'id': 'synthetic_round',
        'collisionType': 'kWepProjCollisionShapeCapsule',
        'propulsion_strategy': 'kWepPropulsionStrategyRocketDumb',
        'detonation_strategy': 'kWepDetonationStrategyImpact',
        'explosion_strategy': 'kWepExplosionStrategyDebris',
        'detonation_timer': 0,
        'rocketDumbDirectionChangeRange': [0, 0],
        'rocketDumbCruiseStraightRange': 2.,
        'rocketAutoExplodeDistance': 70.,
        'm_fArmedTime': .25,
        'm_fLifetimeMax': 9.,
        'radius': .08,
        'capsuleHeight': [0, 0, .4],
        'launch_velocity': 50.,
        'half_damage_radius': 1.,
        'full_damage_radius': 4.,
        'MaterialPenetration': .8,
        'm_fImpactDamage': 91.,
        'damage': {
            'character': 25., 'vehicle': 725., 'objective': 950.,
            'vehicleAPierce': 0, 'vehicleARedux': 0,
        },
    }
    return weapon, ammo


def stock():
    bootstrap = {'factions': []}
    items, ids, original = {}, {}, {}
    for faction in range(3):
        group = f'FactionItemList{faction + 1}'
        names = ('launcher', 'sight')
        kinds = ('kWeapon', 'kAttachmentItem')
        categories = ('weapons', 'attachments')
        kit = [None] * 22
        original[group] = {'items': {}}
        for index, (name, kind, category) in enumerate(zip(names, kinds, categories)):
            ordinal = index + 50
            ref = f'{group}:{ordinal}'
            item_name = f'synthetic_{name}_{faction}'
            item_id = 100 * (faction + 1) + ordinal
            items[ref] = {'authored_ref': ref, 'name': item_name, 'item_type': kind,
                          'definition': {'group': category, 'name': item_name}}
            ids[ref] = item_id
            original[group]['items'][f'ItemListData{ordinal}'] = {'id': item_name, 'item_type': kind}
            kit[(3, 21)[index]] = item_id
        bootstrap['factions'].append({'faction': faction, 'loadouts': [{}, {}, {'items': kit}]})
    return bootstrap, items, ids, original


class InfantryRocketProjectorTests(unittest.TestCase):
    def setUp(self):
        self.tool = runpy.run_path(str(TOOL))

    def test_ballistics_preserves_capsule_arming_range_and_distinct_damage_channels(self):
        weapon, ammo = resources()
        weapon_bits, flight, blast = self.tool['project_ballistics'](weapon, ammo, 3.)
        self.assertEqual(weapon_bits, {'magazine_size': 1, 'initial_magazine_count': 4,
                                       'refire_seconds': 0.75, 'reload_seconds': 2.5})
        self.assertEqual(flight, {'radius_m': .08, 'capsule_spine_local_xyz': [0., 0., .4],
                                  'speed_mps': 50., 'armed_time_seconds': .25,
                                  'auto_explode_distance_m': 70., 'lifetime_seconds': 9.,
                                  'age_grace_seconds': 3.})
        self.assertEqual(blast, {'half_radius_m': 1., 'outer_radius_m': 4.,
                                 'material_penetration': .8, 'character_hp': 25., 'vehicle_hp': 725.})

    def test_hostile_input_cannot_switch_shapes_strategies_or_damage_channels(self):
        for field, value in [
            ('collisionType', 'kWepProjCollisionShapeSphere'), ('capsuleHeight', [0, 0, 0]),
            ('capsuleHeight', [0, False, .5]), ('rocketDumbDirectionChangeRange', [0, .1]),
            ('propulsion_strategy', 'kWepPropulsionStrategyRocketStraight'),
            ('detonation_strategy', 'kWepDetonationStrategyTimer'),
            ('explosion_strategy', 'kWepExplosionStrategyVolume'), ('detonation_timer', 1),
            ('m_fArmedTime', -1), ('m_fArmedTime', 10), ('launch_velocity', float('inf')),
            ('rocketAutoExplodeDistance', 1), ('half_damage_radius', 4.),
        ]:
            with self.subTest(field=field, value=value):
                weapon, ammo = resources()
                ammo[field] = value
                with self.assertRaises(ValueError):
                    self.tool['project_ballistics'](weapon, ammo, 3.)
        weapon, ammo = resources()
        ammo['damage']['vehicleARedux'] = .5
        with self.assertRaisesRegex(ValueError, 'vehicleARedux'):
            self.tool['project_ballistics'](weapon, ammo, 3.)

    def test_stock_magazines_remain_finite_and_ordinary(self):
        for field, value in [('infinite_ammo', True), ('bSupportLightAmmoPack', True),
                             ('magazine_capacity', True), ('magazine_capacity', 2),
                             ('initial_magazine_count', 0), ('category', 'kWepCategoryTurretRocket')]:
            with self.subTest(field=field):
                weapon, ammo = resources()
                weapon[field] = value
                with self.assertRaises(ValueError):
                    self.tool['project_ballistics'](weapon, ammo, 3.)

    def test_stock_binding_uses_exact_gear_sight_faction_and_original_item_row(self):
        args = stock()
        rows = self.tool['select_stock'](*args)
        self.assertEqual([(r['faction'], r['item_id'], r['stock_sight_item_id']) for r in rows],
                         [(0, 150, 151), (1, 250, 251), (2, 350, 351)])
        for change in ('duplicate_faction', 'other_faction_item', 'misplaced_sight', 'changed_original'):
            with self.subTest(change=change):
                bootstrap, items, ids, original = copy.deepcopy(args)
                kit = bootstrap['factions'][0]['loadouts'][2]['items']
                if change == 'duplicate_faction':
                    bootstrap['factions'][1]['faction'] = 0
                elif change == 'other_faction_item':
                    kit[3] = 250
                elif change == 'misplaced_sight':
                    kit[21] = kit[3]
                else:
                    original['FactionItemList1']['items']['ItemListData50']['id'] = 'foreign'
                with self.assertRaises(ValueError):
                    self.tool['select_stock'](bootstrap, items, ids, original)

    def test_binding_and_bootstrap_provenance_is_joint(self):
        boot = {'schema_version': 1, 'application_id': 20231, 'provenance': {
            'original_retail_elf_sha256': self.tool['SOURCE']['RETAIL_ELF_SHA256'],
            'catalog_sha256': 'a' * 64, 'snapshot_sha256': 'b' * 64,
        }}
        self.tool['validate_bootstrap'](boot, 'a' * 64, 'b' * 64)
        for field in ('catalog_sha256', 'snapshot_sha256', 'original_retail_elf_sha256'):
            with self.subTest(field=field):
                changed = copy.deepcopy(boot)
                changed['provenance'][field] = 'c' * 64
                with self.assertRaisesRegex(ValueError, 'bootstrap'):
                    self.tool['validate_bootstrap'](changed, 'a' * 64, 'b' * 64)


if __name__ == '__main__':
    unittest.main()
