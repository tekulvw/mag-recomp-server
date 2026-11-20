import hashlib
from pathlib import Path
import runpy
import unittest


TOOLS = Path(__file__).resolve().parents[1] / 'tools' / 'extraction'
PROJECT = runpy.run_path(str(TOOLS / 'project-retail-domination-control.py'))


class DominationControlProjectorTests(unittest.TestCase):
    def test_else_and_callback_parameter_binding(self):
        script = b'''
function Synthetic(input, output)
  if (ZScript.Mission_GetObjectiveState(input) == ZScript.ObjectiveState.kObjectiveStateCompleted) then
    ZScript.Mission_SetObjectiveState(output, ZScript.Caller, ZScript.ObjectiveState.kObjectiveStateLocked)
  else
    ZScript.Mission_SetObjectiveState(output, nil, ZScript.ObjectiveState.kObjectiveStateDeactivated)
  end
end
'''
        scripts = {'synthetic.lua': PROJECT['parse_script'](script)}
        entry = PROJECT['project_entry']({
            'member': 'synthetic.lua',
            'function': 'Synthetic',
            'delay_ms': 287,
            'parameters': [
                {'Object': '0x00000007', 'Number': 0},
                {'Object': '0x00000009', 'Number': 0},
            ],
        }, scripts, {7, 9})
        self.assertEqual(entry['delay_ms'], 287)
        self.assertEqual([rule['conditions'][0] for rule in entry['rules']], [
            {'objective_token': 7, 'state': 'completed', 'equal': True},
            {'objective_token': 7, 'state': 'completed', 'equal': False},
        ])
        self.assertEqual([rule['commands'][0] for rule in entry['rules']], [
            {'kind': 'set_state', 'objective_token': 9, 'state': 'locked', 'caller': 'original'},
            {'kind': 'set_state', 'objective_token': 9, 'state': 'deactivated', 'caller': 'none'},
        ])

    def test_rejects_unrecognized_statement_and_out_of_bounds_source(self):
        with self.assertRaisesRegex(ValueError, 'Unsupported callback statement'):
            PROJECT['parse_script'](b'function Synthetic(a)\nUnexpected(a)\nend')
        with self.assertRaisesRegex(ValueError, 'exceeds parser bound'):
            PROJECT['parse_script'](b' ' * 65_537)

    def test_rejects_forged_callback_fingerprint(self):
        class Archive:
            names = ['synthetic.lua']

            def extract(self, member):
                self.last_member = member
                return b'changed contents'

        archive = Archive()
        digest = hashlib.sha256(b'original contents').hexdigest()
        with self.assertRaisesRegex(ValueError, 'fingerprint mismatch'):
            PROJECT['parse_members'](archive, {'synthetic.lua': digest})
        self.assertEqual(archive.last_member, 1)
