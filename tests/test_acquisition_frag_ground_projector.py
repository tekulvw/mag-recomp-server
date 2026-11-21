"""Synthetic regressions for Acquisition frag Ground chassis source projection."""
import runpy
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1] / 'tools/extraction'
TOOL = ROOT / 'project-retail-acquisition-frag-ground-areas.py'


class AcquisitionFragGroundProjectionTests(unittest.TestCase):
    def test_operational_chassis_picks_original_tag_and_pinned_pack_not_ordinal_zero(self):
        tool = runpy.run_path(str(TOOL))
        select = tool['GROUND']['ground_chassis_entry']
        turret = {'index': 0, 'collision_tag': 31, 'sha256': 'a' * 64}
        chassis = {'index': 1, 'collision_tag': 9, 'sha256': 'b' * 64}
        self.assertEqual(select([turret, chassis], chassis['sha256']), chassis)
        with self.assertRaisesRegex(ValueError, 'chassis'):
            select([turret, chassis], turret['sha256'])
        with self.assertRaisesRegex(ValueError, 'chassis'):
            select([chassis, {**turret, 'collision_tag': 0}], chassis['sha256'])

    def test_scope_covers_all_three_factions_of_all_three_ground_roles(self):
        tool = runpy.run_path(str(TOOL))
        token = tool['GROUND']['source_token']
        sources = [{'vehicle_type': role, 'PropertiesFileName': f'{role}-{faction}.zpr'}
                   for role in ('apc', 'truck', 'escort') for faction in range(3)]
        bindings = [{'vehicle_type_token': token(role), 'faction': faction,
                     'template': f'{role}-{faction}'}
                    for role in ('apc', 'truck', 'escort') for faction in range(3)]
        rows = tool['select_variants'](sources, bindings)
        self.assertEqual([(kind, binding['faction']) for kind, binding, _ in rows],
                         [(kind, faction) for kind in (3, 7, 9) for faction in range(3)])
        with self.assertRaises(ValueError):
            tool['select_variants'](sources, bindings[:-1])
        with self.assertRaises(ValueError):
            tool['select_variants'](sources, [*bindings, bindings[0]])
