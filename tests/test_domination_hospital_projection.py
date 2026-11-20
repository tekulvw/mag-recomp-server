"""Synthetic tests for hospital source projection and qualified spawn promotion."""

import importlib.util
import json
from pathlib import Path
import unittest

SOURCE = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-domination-hospitals.py'
SPEC = importlib.util.spec_from_file_location('hospital_projector', SOURCE)
PROJECTOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROJECTOR)


class DominationHospitalProjectionTests(unittest.TestCase):
    def test_promotes_only_original_hospital_squad_members(self):
        points, squads, unsupported, hospitals = [], [], [], []
        for i in range(16):
            token = 100 + i
            points.append(dict(index=i, kind='unsupported', active=True, token=token))
            squads.append(dict(index=16+i, available=[20, 21]))
            unsupported.append(dict(index=i, spawn_type=8, user_class=0,
                                    squad_links=[[1, i//4, i%4, 2]]))
            hospitals.append(dict(spawn_index=i, spawn_token=hex(token), team=1,
                                  squad_link=dict(team=1, platoon_id=i//4, slot=2,
                                                  squad=('Alpha','Bravo','Charlie','Delta')[i%4])))
        spawn = dict(match_profile=dict(has_bunkers=False),
                     active=dict(map_name='test', squad_index=16, default_spawn=20),
                     source=json.dumps(dict(native_spawn_bindings_sha256='source',
                                            unsupported_spawn_links=unsupported)),
                     maps=[dict(name='test', locations=points, squads=squads)])
        authored = dict(spawn_binding_sha256='source', maps=[dict(map='test', hospitals=hospitals)])
        def promote():
            return json.loads(PROJECTOR.promote_spawns(
                PROJECTOR.encode(spawn), PROJECTOR.encode(authored), b'proof'))
        promoted = promote()
        self.assertEqual(promoted['active']['available'], [20, 21, 0])
        self.assertEqual(promoted['active']['default_spawn'], 20)
        self.assertTrue(promoted['match_profile']['has_bunkers'])
        self.assertTrue(all(point['kind'] == 'bunker_passenger'
                            for point in promoted['maps'][0]['locations']))
        hospitals[0]['squad_link']['slot'] = 1
        with self.assertRaisesRegex(ValueError, 'original spawn membership'):
            promote()

    def test_inverse_rotation_preserves_world_exit_direction(self):
        root = 2 ** -0.5
        x, y, z = PROJECTOR.inverse_rotate((0., root, 0., root), (0., 0., -2.))
        self.assertAlmostEqual(x, 2.)
        self.assertAlmostEqual(y, 0.)
        self.assertAlmostEqual(z, 0.)


if __name__ == '__main__':
    unittest.main()
