"""Synthetic smoke projection checks; no original files are required."""
import copy
from pathlib import Path
import runpy
import unittest


TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-smoke-grenades.py'


def resources():
    return ({'id': 'synthetic_smoke', 'ammo_name': 'synthetic_volume',
             'category': 'kWepCategoryGrenade', 'magazine_capacity': 1,
             'initial_magazine_count': 3, 'infinite_ammo': False},
            {'id': 'synthetic_volume', 'propulsion_strategy': 'kWepPropulsionStrategyThrown',
             'detonation_strategy': 'kWepDetonationStrategyTimer',
             'explosion_strategy': 'kWepExplosionStrategyVolume',
             'm_eAmmoPersistance': 'kWepPersistanceGrenadeSmoke',
             'detonation_timer': 4.25, 'm_fLifetimeMax': 16., 'm_fDuration': 24.,
             'destroy_objective': False, 'doHearingLoss': False,
             'damage': {key: 0 for key in ('character', 'characterAPierce', 'characterARedux',
                                         'vehicle', 'vehicleAPierce', 'vehicleARedux',
                                         'objective', 'objectiveAPierce', 'objectiveARedux', 'mine')}})


def stock():
    bootstrap, items, ids, original = {'factions': []}, {}, {}, {}
    for faction in range(3):
        group = f'FactionItemList{faction + 1}'
        ref, name, wire = f'{group}:7', f'synthetic_smoke_{faction}', 100 * (faction + 1) + 7
        kit = [None] * 22
        kit[4] = wire
        bootstrap['factions'].append({'faction': faction, 'loadouts': [{}, {'items': kit}]})
        items[ref] = {'authored_ref': ref, 'item_type': 'kWeapon', 'name': name,
                      'definition': {'group': 'weapons', 'name': name}}
        ids[ref] = wire
        original[group] = {'items': {'ItemListData7': {'id': name, 'item_type': 'kWeapon'}}}
    return bootstrap, items, ids, original


class SmokeGrenadeProjectorTests(unittest.TestCase):
    def setUp(self):
        self.tool = runpy.run_path(str(TOOL))

    def test_authored_fuse_projectile_and_volume_lifetimes_remain_distinct(self):
        weapon, ammo = resources()
        self.assertEqual(self.tool['project_parameters'](weapon, ammo), {
            'initial_count': 3, 'fuse_ms': 4250, 'projectile_lifetime_ms': 16000,
            'volume_duration_ms': 24000})

    def test_smoke_never_admits_damage_or_other_explosion_strategies(self):
        for field in resources()[1]['damage']:
            weapon, ammo = resources()
            ammo['damage'][field] = .01
            with self.subTest(damage=field), self.assertRaises(ValueError):
                self.tool['project_parameters'](weapon, ammo)
        for field, value in [('destroy_objective', True), ('doHearingLoss', True),
                             ('propulsion_strategy', 'kWepPropulsionStrategyRocketDumb'),
                             ('detonation_strategy', 'kWepDetonationStrategyImpact'),
                             ('explosion_strategy', 'kWepExplosionStrategyDebris'),
                             ('m_eAmmoPersistance', 'kWepPersistanceGrenadeFrag')]:
            weapon, ammo = resources()
            ammo[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.tool['project_parameters'](weapon, ammo)

    def test_invalid_timing_inventory_and_ammo_binding_are_rejected(self):
        for field, value in [('detonation_timer', float('nan')), ('m_fDuration', 0),
                             ('m_fLifetimeMax', 4), ('m_fDuration', .0005)]:
            weapon, ammo = resources()
            ammo[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.tool['project_parameters'](weapon, ammo)
        for field, value in [('infinite_ammo', True), ('magazine_capacity', True),
                             ('initial_magazine_count', 0), ('ammo_name', 'other')]:
            weapon, ammo = resources()
            weapon[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.tool['project_parameters'](weapon, ammo)

    def test_stock_selection_requires_all_factions_exact_marksman_gear_and_original_rows(self):
        args = stock()
        self.assertEqual([(r['faction'], r['item_id']) for r in self.tool['select_stock'](*args)],
                         [(0, 107), (1, 207), (2, 307)])
        for change in ('duplicate_faction', 'foreign_item', 'wrong_gear', 'changed_original'):
            bootstrap, items, ids, original = copy.deepcopy(args)
            kit = bootstrap['factions'][0]['loadouts'][1]['items']
            if change == 'duplicate_faction':
                bootstrap['factions'][2]['faction'] = 1
            elif change == 'foreign_item':
                kit[4] = 207
            elif change == 'wrong_gear':
                kit[3], kit[4] = kit[4], None
            else:
                original['FactionItemList1']['items']['ItemListData7']['id'] = 'other'
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.tool['select_stock'](bootstrap, items, ids, original)


if __name__ == '__main__':
    unittest.main()
