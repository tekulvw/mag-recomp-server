import runpy
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1] / 'tools' / 'extraction'
projector = runpy.run_path(str(TOOLS / 'project-retail-active-ability-effects.py'))


def catalog(properties, rows, groups=2):
    factions = ('FactionTypeBlack', 'FactionTypeRed', 'FactionTypeGreen')
    return {
        'Delay_ms': 123,
        'PropsList': {
            f'{properties}{index}': {
                properties: {'Faction': faction, **rows(index)},
            }
            for index, faction in enumerate(factions * groups)
        },
    }


def cluster_properties(index):
    return {
        'NumExplosions': 3 + index,
        'TimeBetweenExplosions_ms': 24,
        'DamageDelay_ms': 5,
        'Damage': 43,
        'Radius': 17,
        'FullDamageRadius': 6,
        'FullDamageScale': 0,
        'HalfDamageRadius': 3,
        'HalfDamageScale': 0,
    }


def sensor_properties(index):
    return {
        'Duration_ms': 81 + index,
        'AttackerRadius': 12,
        'DefenderRadius': 14,
        'DetectionSpeedThreshold': 1.25,
    }


def guided_properties(index):
    return {'DamageDelay_ms': 3 + index, 'EffectiveRadius': 21 + index}


def bomblet_properties(index):
    return {
        'BombletDamage': 13 + index,
        'BombletFullDamageRadius': 6,
        'BombletFullDamageScale': 0.15,
        'BombletHalfDamageRadius': 2,
        'BombletHalfDamageScale': 0.6,
        'TimeBetweenBomblets_ms': 11 + index,
    }


def gas_properties(index):
    return {
        **bomblet_properties(index),
        'Duration_ms': 31 + index,
        'NumBomblets': 5,
        'GasDamagePerSecond': 23,
        'GasFullDamageRadius': 20,
        'GasHalfDamageRadius': 20,
        'GasFullDamageScale': 1,
        'GasHalfDamageScale': 0.75,
    }


def strafe_properties(index):
    return {
        **bomblet_properties(index),
        'DamageDelay_ms': 2 + index,
        'StrafeDuration_ms': 25,
        'StrafeLength': 100,
        'DeviationRadius': 7,
    }


def tables():
    return {
        'cluster': catalog('ClusterBombProps', cluster_properties),
        'sensor': catalog('AbilitySensorProps', sensor_properties),
        'precision_strike': catalog('PrecisionStrikeProps', lambda _: {}, groups=1),
        'guided_artillery': catalog('GuidedArtilleryProps', guided_properties),
        'gas_bombardment': catalog('GasBombardmentProps', gas_properties),
        'strafing_run': catalog('StrafingRunProps', strafe_properties),
    }


class EffectProjectionTests(unittest.TestCase):
    def test_preserves_authored_side_groups_and_named_faction_not_list_index(self):
        rows = projector['project_tables'](tables())
        self.assertEqual(len(rows), 33)
        keys = {(row['side'], row['faction'], next(iter(row['rules']))) for row in rows}
        self.assertEqual(len(keys), 33)
        self.assertEqual(len({name for _, name, _, _, _ in projector['TABLES']}), 6)
        self.assertEqual({(side, faction) for side, faction, ability in keys
                          if ability == 'precision_strike'},
                         {('attacker', faction) for faction in ('black', 'green', 'red')})
        selected = next(row['rules']['cluster'] for row in rows if
                        row['side'] == 'defender' and row['faction'] == 'red'
                        and 'cluster' in row['rules'])
        self.assertEqual(selected['count'], 7)
        self.assertEqual(selected['delay_ms'], 123)
        self.assertEqual(selected['scatter_radius'], 17)
        self.assertEqual(selected['inner_radius'], 3)
        self.assertEqual(selected['outer_radius'], 6)

    def test_rejects_missing_duplicate_or_invalid_authored_rows(self):
        supplied = tables()
        cluster = supplied['cluster']
        row = cluster['PropsList']['ClusterBombProps1']['ClusterBombProps']
        row['Faction'] = 'FactionTypeBlack'
        with self.assertRaisesRegex(ValueError, 'repeated'):
            projector['project_tables'](supplied)
        row['Faction'] = 'FactionTypeRed'
        row['Damage'] = True
        with self.assertRaisesRegex(ValueError, 'Damage'):
            projector['project_tables'](supplied)
        del cluster['PropsList']['ClusterBombProps1']
        with self.assertRaisesRegex(ValueError, 'rows'):
            projector['project_tables'](supplied)

    def test_projects_platoon_values_without_conflating_sides_or_damage_channels(self):
        rows = projector['project_tables'](tables())
        chosen = {(row['side'], row['faction'], ability): rules
                  for row in rows for ability, rules in row['rules'].items()}
        gas = chosen['defender', 'red', 'gas_bombardment']
        self.assertEqual(gas['damage_delay_ms'], 0)
        self.assertEqual(gas['duration_ms'], 35)
        self.assertEqual(gas['bomblet_damage'], 17)
        self.assertEqual(gas['bomblet']['inner_radius'], 2)
        self.assertEqual(gas['gas']['outer_radius'], gas['gas']['inner_radius'])
        self.assertNotEqual(chosen['attacker', 'red', 'gas_bombardment']['interval_ms'],
                            gas['interval_ms'])
        self.assertEqual(chosen['defender', 'red', 'guided_artillery']['effective_radius'], 25)
        self.assertEqual(chosen['attacker', 'red', 'strafing_run']['damage_delay_ms'], 3)
        self.assertEqual(chosen['attacker', 'red', 'precision_strike'], {'delay_ms': 123})

    def test_rejects_extra_precision_rows_and_invalid_platoon_falloff(self):
        supplied = tables()
        supplied['precision_strike'] = catalog('PrecisionStrikeProps', lambda _: {})
        with self.assertRaisesRegex(ValueError, 'precision_strike properties rows'):
            projector['project_tables'](supplied)
        supplied = tables()
        row = supplied['gas_bombardment']['PropsList']['GasBombardmentProps2']['GasBombardmentProps']
        row['BombletFullDamageScale'] = float('nan')
        with self.assertRaisesRegex(ValueError, 'BombletFullDamageScale'):
            projector['project_tables'](supplied)
