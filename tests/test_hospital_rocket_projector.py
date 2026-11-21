"""Synthetic validation for source-bound hospital rocket projections."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
import zlib


TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-hospital-rockets.py'


def token(name):
    return zlib.crc32(name.lower().replace('\\', '/').encode('ascii'))


def source_model():
    nodes = [
        SimpleNamespace(node_id=i, parent_node_id=parent, transform_index=i, name_token=token(name))
        for i, (parent, name) in enumerate([
            (0xffff, 'gunner_seat'), (0, 'turret_yaw'), (1, 'turret_pitch'), (2, 'firepoint_02'),
        ])
    ]
    transforms = [
        SimpleNamespace(translation_xyz=position, quaternion_xyzw=(0., 0., 0., 1.), uniform_scale=1.)
        for position in [(0., 0., 0.), (0., 1., 0.), (0., 0.4, -0.1), (0.2, 0., 0.5)]
    ]
    return {'nodes': nodes, 'reference_transforms': transforms}


def source_ballistics():
    weapon = {
        'id': 'invented_rocket', 'ammo_name': 'invented_rocket', 'use_turret_mesh': True,
        'model_name': '', 'alternate_firepoint': 'firepoint_02', 'category': 'kWepCategoryTurretRocket',
        'fire_param_id': 'kFireParamSecondary', 'initial_fire_mode': 'kSemiAuto',
        'fire_mode': {'fire_modes': {'FireModeTypeClass0': {
            'FireModeTypeClass': {'fire_mode_value': 'kSemiAuto'},
        }}},
    }
    ammo = {
        'id': 'invented_rocket', 'propulsion_strategy': 'kWepPropulsionStrategyRocketStraight',
        'detonation_strategy': 'kWepDetonationStrategyImpact',
        'explosion_strategy': 'kWepExplosionStrategyDebris',
        'collisionType': 'kWepProjCollisionShapeSphere',
        'radius': 0.2, 'launch_velocity': 50., 'm_fLifetimeMax': 8., 'm_fArmedTime': -1.,
        'detonation_timer': 0, 'half_damage_radius': 1.5, 'full_damage_radius': 4.,
        'MaterialPenetration': 1.2, 'm_fImpactDamage': 72.,
        'damage': {'character': 134., 'vehicle': 750., 'objective': 820., 'mine': 23.,
                   'vehicleAPierce': 0., 'vehicleARedux': 0., 'objectiveAPierce': 0.,
                   'objectiveARedux': 0., 'characterAPierce': 0., 'characterARedux': 0.4},
    }
    return weapon, ammo


class HospitalRocketProjectorTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('synthetic_hospital_rockets', TOOL)
        self.tool = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.tool)

    def test_typed_channels_and_half_radius_derive_from_authored_ammo(self):
        weapon, ammo = source_ballistics()
        flight, blast = self.tool.project_ballistics(weapon, ammo, 2.)
        self.assertEqual(flight['radius_m'], 0.2)
        self.assertEqual(flight['speed_mps'], 50.)
        self.assertEqual(flight['lifetime_seconds'], 8.)
        self.assertEqual(flight['age_grace_seconds'], 2.)
        self.assertEqual(flight['armed_time_seconds'], -1.)
        self.assertEqual(blast, {
            'half_radius_m': 1.5, 'outer_radius_m': 4., 'material_penetration': 1.2,
            'character_hp': 134., 'vehicle_hp': 750., 'objective_hp': 820.,
        })

    def test_unqualified_rocket_inputs_fail_closed(self):
        for key, bad in [
            ('radius', True), ('launch_velocity', float('inf')), ('half_damage_radius', 4.),
            ('detonation_strategy', 'Timer'), ('detonation_timer', 1), ('m_fArmedTime', 2.),
        ]:
            with self.subTest(key=key):
                weapon, ammo = source_ballistics()
                ammo[key] = bad
                with self.assertRaises(ValueError):
                    self.tool.project_ballistics(weapon, ammo, 2.)
        weapon, ammo = source_ballistics()
        weapon['use_turret_mesh'] = False
        with self.assertRaises(ValueError):
            self.tool.project_ballistics(weapon, ammo, 2.)

    def test_weapon_policy_retains_authored_times_and_separate_magazine_values(self):
        source = {
            'fire_mode': {'refire_time': 0.7, 'reload_time': 3.5},
            'infinite_ammo': False, 'magazine_capacity': 7, 'initial_magazine_count': 4,
        }
        self.assertEqual(self.tool.project_weapon(source), {
            'refire_seconds': 0.7, 'reload_seconds': 3.5, 'infinite_ammo': False,
            'magazine_size': 7, 'initial_magazine_count': 4,
        })
        for field, bad in [('infinite_ammo', 1), ('magazine_capacity', 256),
                           ('initial_magazine_count', 1.5)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.tool.project_weapon(source | {field: bad})

    def test_exact_named_muzzle_chain_keeps_nonzero_local_poses(self):
        model = source_model()
        chain, refs = self.tool.project_muzzle(model, 'firepoint_02')
        self.assertEqual(chain['pitch']['position_xyz'], (0., 0.4, -0.1))
        self.assertEqual(chain['firepoint']['position_xyz'], (0.2, 0., 0.5))
        self.assertEqual(refs['firepoint']['parent_node_id'], 2)
        model['nodes'][3].parent_node_id = 1
        with self.assertRaises(ValueError):
            self.tool.project_muzzle(model, 'firepoint_02')

    def test_muzzle_rejects_multiple_matching_nodes_and_wrong_transform(self):
        model = source_model()
        model['nodes'].append(model['nodes'][-1])
        with self.assertRaises(ValueError):
            self.tool.project_muzzle(model, 'firepoint_02')
        model = source_model()
        model['reference_transforms'][3].uniform_scale = 2.
        with self.assertRaises(ValueError):
            self.tool.project_muzzle(model, 'firepoint_02')

    def test_override_checks_canonical_keys_and_does_not_remove_other_entries(self):
        original = b'// default\n"SOME\\THING", "first",\n"some/thing", "second" // duplicate\n'
        checked, proof = self.tool.project_tokens(original, ['invented_rocket', 'firepoint_02'])
        self.assertEqual(checked['invented_rocket'], token('invented_rocket'))
        self.assertEqual(proof['authored_pairs'], 2)
        self.assertEqual(proof['canonical_keys'], 1)
        with self.assertRaises(ValueError):
            self.tool.project_tokens(original, ['SOME/THING'])
        with self.assertRaises(ValueError):
            self.tool.project_tokens(b'"invented_rocket", "renamed"\nunknown', ['firepoint_02'])

    def test_token_selection_refuses_duplicate_requested_names(self):
        with self.assertRaises(ValueError):
            self.tool.project_tokens(b'', ['same', 'SAME'])
