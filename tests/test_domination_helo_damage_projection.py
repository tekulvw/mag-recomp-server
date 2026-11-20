import math
from pathlib import Path
import runpy
import unittest

TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-domination-helo-damage.py'


class HeloDamageProjectionTests(unittest.TestCase):
    def test_grenade_penetration_requires_complete_unanimous_authored_rows(self):
        project = runpy.run_path(str(TOOL))['project_grenade_penetration']
        rows = {'ProjectileAmmoData0': {'ProjectileAmmoData': {'MaterialPenetration': '1.25'}},
                'ProjectileAmmoData1': {'ProjectileAmmoData': {'MaterialPenetration': '1.25'}}}
        resource = {'ProjectileAmmoDataTable': {'ProjectileAmmoDataList': rows}}
        self.assertEqual(project(resource), 1.25)
        rows['ProjectileAmmoData1']['ProjectileAmmoData']['MaterialPenetration'] = '1.5'
        with self.assertRaises(ValueError):
            project(resource)
        rows.clear()
        with self.assertRaises(ValueError):
            project(resource)

    def fixture(self):
        return {name: {'HeloVehicleProperties': {
            'DamageProperties': {'ArmorType': '2', 'DamageCriticalHealthLossRate': '4.5',
                'DestroyedHulk': '', 'DestroyDamageValue': '123',
                'DestroyDamageHalfRadius': '3', 'DestroyDamageFullRadius': '6', 'DestroyFXTime': '1.75'},
            'InBoundMinDamageFraction': '0.1', 'OutBoundMinDamageFraction': '0.2',
            'RetreatDamageFraction': '0.3'}} for name in ['Valor', 'Raven', 'SVER']}

    def test_preserves_authored_armor_and_native_defaults_without_radius_inversion(self):
        project = runpy.run_path(str(TOOL))['project_profiles']
        profiles = project(self.fixture(), 91.5, 1.125)
        self.assertEqual(set(profiles), {0, 1, 2})
        self.assertEqual(profiles[1]['max_health'], 91.5)
        self.assertEqual(profiles[1]['armor_type'], 2)
        self.assertEqual(profiles[1]['inbound_min_fraction'], 0.1)
        self.assertEqual(profiles[1]['destruction'], {
            'damage': 123., 'inner_radius': 3., 'outer_radius': 6., 'material_penetration': 1.125,
            'effect_seconds': 1.75})

    def test_rejects_unqualified_overrides_hulks_and_nonfinite_properties(self):
        project = runpy.run_path(str(TOOL))['project_profiles']
        for key, value in [('MaxHitpoints', '100'), ('DestroyMaterialPenetration', '2'),
                           ('DestroyedHulk', 'invented-hulk'), ('DestroyDamageValue', 'nan')]:
            sources = self.fixture()
            sources['SVER']['HeloVehicleProperties']['DamageProperties'][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                project(sources, 91.5, 1.125)
        for value in [True, 0., math.inf]:
            with self.subTest(default=value), self.assertRaises(ValueError):
                project(self.fixture(), value, 1.125)


if __name__ == '__main__':
    unittest.main()
