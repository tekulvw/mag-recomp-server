import json
from pathlib import Path
import runpy
import unittest


tool = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-abilities.py'))


def project_with(modifier):
    resources = {}
    global_table = {'CommandGlobalCoolDown_ms': 200, 'StrategicGlobalCoolDown_ms': 300}
    members = (
        ('strategic_blockade_properties', 'StrategicBlockadePropsTable', modifier),
        ('strategic_rapid_redeploy_properties', 'StrategicRapidRedeployPropsTable', '-.25f'),
        ('strategic_sensor_sweep_properties', 'StrategicSensorSweepPropsTable', None),
    )
    for side in ('Attacker', 'Defender'):
        for count in (64, 128, 256):
            global_table[f'{side}SquadCommandTypes{count}'] = {}
            global_table[f'{side}PlatoonCommandTypes{count}'] = {}
            global_table[f'{side}StrategicTypes{count}'] = (
                {'Element0': {'element': 'kStrategicTypeBlockade' if side == 'Attacker'
                              else 'kStrategicTypeRapidRedeploy'}} if count == 256 else {}
            )
    def add(member, key, value):
        resources[f'scripts/abilities/{member}.zpr'] = {
            'parameters': {key: value}, 'sha256': 'a1' * 32,
        }
    add('global_ability_properties', 'GlobalAbilityProps', global_table)
    for stem in tool['COMMANDS'].values():
        add(stem, 'TestCommandTable', {'CoolDown_ms': 200, 'Delay_ms': 10})
    for stem, key, modifier_value in members:
        values = {'CoolDown_ms': 500, 'Duration_ms': 400}
        if modifier_value is not None:
            values['Modifier'] = modifier_value
        add(stem, key, values)
    return tool['project'](json.dumps({
        'schema_version': 1, 'application_id': 20231, 'source_archive_sha256': 'b2' * 32,
        'resources': resources,
    }).encode())


class StrategicProjectionTests(unittest.TestCase):
    def test_preserves_signed_wave_scalars_and_source_members(self):
        projected = project_with('.375f')
        commands = {p['ability']: p for p in projected['strategic']}
        self.assertEqual(commands['Blockade']['spawn_interval_modifier'], 0.375)
        self.assertEqual(commands['RapidRedeploy']['spawn_interval_modifier'], -0.25)
        self.assertEqual(commands['RapidRedeploy']['source_sha256'], 'a1' * 32)
        self.assertNotIn('spawn_interval_modifier', commands['SensorSweep'])

    def test_missing_invalid_or_wrong_sign_scalar_fails_closed(self):
        for value in (None, '1/2', '-.25f', True, 'nanf'):
            with self.subTest(value=value):
                with self.assertRaisesRegex(ValueError, 'strategic_spawn_modifier'):
                    project_with(value)
