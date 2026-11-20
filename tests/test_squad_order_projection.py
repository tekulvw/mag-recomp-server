import runpy
import unittest
from pathlib import Path

PROJECTOR = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-squad-orders.py'


class SquadOrderProjection(unittest.TestCase):
    def test_uses_authored_setting_and_provenance(self):
        tool = runpy.run_path(str(PROJECTOR))
        member = b'character_gameplay = { SquadLeaderOrderCooldown_ms = "1800" }'
        data = tool['project'](member, 'a' * 64)
        self.assertEqual(data['settings']['squad_order_cooldown_ms'], 1800)
        source = data['source']
        self.assertEqual(source['archive_sha256'], 'a' * 64)
        self.assertEqual(source['member_sha256'], tool['sha'](member))

    def test_missing_and_malformed_settings_are_rejected(self):
        project = runpy.run_path(str(PROJECTOR))['project']
        for value in ('', 'SquadLeaderOrderCooldown_ms = -1',
                      'SquadLeaderOrderCooldown_ms = 12.5',
                      'SquadLeaderOrderCooldown_ms = "12x"',
                      'SquadLeaderOrderCooldown_ms = 4294967296',
                      'SquadLeaderOrderCooldown_ms = 12 SquadLeaderOrderCooldown_ms = 13'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                project(f'character_gameplay = {{ {value} }}'.encode(), 'a' * 64)


if __name__ == '__main__':
    unittest.main()
