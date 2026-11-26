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


def acquisition_inputs():
    """SYNTHETIC Acquisition map: two defending squads, one field hospital each."""
    points = [dict(index=0, token=100, kind='insertion', active=True),
              dict(index=1, token=101, kind='alternate', active=True),
              dict(index=2, token=102, kind='alternate', active=True)]
    squads = [dict(index=16, available=[0, 1], default_spawn=0),
              dict(index=17, available=[0, 2], default_spawn=0)]
    links = [dict(index=i, spawn_type=8, category=2, user_class=0, squad_links=[[1, 0, i - 1, 1]])
             for i in (1, 2)]
    source = dict(native_elf_sha256='e' * 64, level_archive_sha256='a' * 64, level_clv_sha256='c' * 64,
                  unsupported_spawn_links=links)
    spawn = dict(format='mag_spawn_catalog', source=json.dumps(source),
                 match_profile=dict(game_type=1, auto_assign_teams=True, has_bunkers=False),
                 active=dict(map_name='test', squad_index=16, default_spawn=0, available=[0, 1]),
                 maps=[dict(name='test', locations=points, squads=squads)])
    policy = dict(transform=dict(position=dict(x=1., y=2., z=3.), rotation_xyzw=[0., 0., 0., 1.]),
                  candidates=[dict(ranking_offset=[9., 0., 0.], position_offset=[9., -1., 0.])])
    rows = [dict(spawn_index=i, spawn_token=hex(100 + i), spawn_type=8, category=2, team=1, active=True,
                 vehicle_type_token='0x7', vehicle_mountable=True, properties_file='green.zpr',
                 emplacement_origin_xyz=[1., 2., 3.], bunker_exit_policy=policy) for i in (2, 1)]
    exits = dict(format=PROJECTOR.ACQUISITION_EXITS, map='test', field_hospitals=rows)
    model = dict(format=PROJECTOR.ALLOCATION, allocation_scope=PROJECTOR.SCOPE, fresh_family13_pool=True,
                 all_initial_class4_creations_succeed=True,
                 production_without_early_other_class4_creations=True, retail_elf_sha256='e' * 64,
                 maps=[dict(map='other', objects=[dict(object_index=0, spawn_index=5, spawn_token='0x1'),
                                                 dict(object_index=1, spawn_index=9, spawn_token='0x2')])])
    structures = dict(application_id=20231, source_sha256={'vehicle/green.zpr': 'f' * 64})
    structure_raw = PROJECTOR.encode(structures)
    seats = dict(format=PROJECTOR.PLACEMENTS, allocation_scope=PROJECTOR.SCOPE,
                 passenger_pose_scope=PROJECTOR.PASSENGER_SCOPE, passenger_pose_sha256='d' * 64,
                 structure_catalog_sha256=PROJECTOR.sha(structure_raw), vehicle_type_token=7,
                 placements=[dict(properties_source='vehicle/green.zpr', max_occupancy=12,
                                  passenger_seats=12, mounts=[dict(local_offset=[6., 0., 0.], radius=2.)])])
    return spawn, exits, model, structure_raw, seats


class AcquisitionHospitalProjectionTests(unittest.TestCase):
    def project(self, spawn, exits, model, structure_raw, seats):
        spawn_raw, exits_raw = PROJECTOR.encode(spawn), PROJECTOR.encode(exits)
        allocation = PROJECTOR.acquisition_allocation(spawn_raw, exits_raw, PROJECTOR.encode(model))
        promoted = PROJECTOR.promote_acquisition_spawns(spawn_raw, exits_raw, allocation)
        world = PROJECTOR.project_acquisition(promoted, structure_raw, exits_raw, allocation,
                                              [PROJECTOR.encode(seats)])
        return json.loads(allocation), json.loads(promoted), world

    def test_field_hospitals_become_indexed_bunker_passenger_points(self):
        allocation, promoted, world = self.project(*acquisition_inputs())
        self.assertEqual([(row['object_index'], row['spawn_index']) for row in allocation['maps'][0]['objects']],
                         [(0, 1), (1, 2)])
        self.assertTrue(promoted['match_profile']['has_bunkers'])
        kinds = {point['index']: point['kind'] for point in promoted['maps'][0]['locations']}
        self.assertEqual(kinds, {0: 'insertion', 1: 'bunker_passenger', 2: 'bunker_passenger'})
        self.assertEqual(json.loads(promoted['source'])['unsupported_spawn_links'], [])
        self.assertEqual(world['spawn_catalog_sha256'], PROJECTOR.sha(PROJECTOR.encode(promoted)))
        self.assertEqual([row['object_index'] for row in world['placements']], [0, 1])
        self.assertEqual(world['placements'][0]['passenger_seats'], 12)
        self.assertEqual(world['placements'][0]['properties_source'], 'vehicle/green.zpr')

    def test_unselectable_hospitals_join_their_squad_at_the_authored_slot(self):
        spawn, *rest = acquisition_inputs()
        spawn['maps'][0]['locations'][1]['kind'] = 'unsupported'
        spawn['maps'][0]['squads'][0]['available'] = [0]
        spawn['active']['available'] = [0]
        _, promoted, _ = self.project(spawn, *rest)
        self.assertEqual(promoted['active']['available'], [0, 1])

    def test_mismatched_sources_are_refused(self):
        spawn, exits, model, structure_raw, seats = acquisition_inputs()
        model['maps'][0]['objects'][0]['object_index'] = 1
        model['maps'][0]['objects'][1]['object_index'] = 0
        with self.assertRaisesRegex(ValueError, 'spawn order'):
            self.project(spawn, exits, model, structure_raw, seats)
        spawn, exits, model, structure_raw, seats = acquisition_inputs()
        exits['field_hospitals'].pop()
        with self.assertRaisesRegex(ValueError, 'type-8 inventory'):
            self.project(spawn, exits, model, structure_raw, seats)
        spawn, exits, model, structure_raw, seats = acquisition_inputs()
        seats['structure_catalog_sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'structure catalog'):
            self.project(spawn, exits, model, structure_raw, seats)


if __name__ == '__main__':
    unittest.main()
