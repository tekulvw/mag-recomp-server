import runpy
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1] / 'tools' / 'extraction'
projector = runpy.run_path(str(TOOLS / 'project-retail-active-ability-effects.py'))


def catalog(properties, rows):
    factions = ('FactionTypeBlack', 'FactionTypeRed', 'FactionTypeGreen')
    return {
        'Delay_ms': 123,
        'PropsList': {
            f'{properties}{index}': {
                properties: {'Faction': faction, **rows(index)},
            }
            for index, faction in enumerate(factions * 2)
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


class EffectProjectionTests(unittest.TestCase):
    def test_preserves_authored_side_groups_and_named_faction_not_list_index(self):
        cluster = catalog('ClusterBombProps', cluster_properties)
        sensor = catalog('AbilitySensorProps', sensor_properties)
        rows = projector['project_tables'](cluster, sensor)
        self.assertEqual(len(rows), 12)
        selected = next(row['rules']['cluster'] for row in rows if
                        row['side'] == 'defender' and row['faction'] == 'red'
                        and 'cluster' in row['rules'])
        self.assertEqual(selected['count'], 7)
        self.assertEqual(selected['delay_ms'], 123)
        self.assertEqual(selected['scatter_radius'], 17)
        self.assertEqual(selected['inner_radius'], 3)
        self.assertEqual(selected['outer_radius'], 6)

    def test_rejects_missing_duplicate_or_invalid_authored_rows(self):
        sensor = catalog('AbilitySensorProps', sensor_properties)
        cluster = catalog('ClusterBombProps', cluster_properties)
        row = cluster['PropsList']['ClusterBombProps1']['ClusterBombProps']
        row['Faction'] = 'FactionTypeBlack'
        with self.assertRaisesRegex(ValueError, 'repeated'):
            projector['project_tables'](cluster, sensor)
        row['Faction'] = 'FactionTypeRed'
        row['Damage'] = True
        with self.assertRaisesRegex(ValueError, 'Damage'):
            projector['project_tables'](cluster, sensor)
        del cluster['PropsList']['ClusterBombProps1']
        with self.assertRaisesRegex(ValueError, 'rows'):
            projector['project_tables'](cluster, sensor)
