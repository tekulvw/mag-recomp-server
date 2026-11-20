import copy
import json
from pathlib import Path
import runpy
import unittest


API = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tools/extraction'
                         / 'project-retail-domination-spawns.py'))


class DominationSpawnProjection(unittest.TestCase):
    def setUp(self):
        name = 'mag_baku_final'
        builder = API['BUILDER']
        game_sha = builder['GAME_ARCHIVE_SHA256']
        settings_sha = builder['SETTINGS_SHA256']
        elf_sha = API['RETAIL_SHA']
        self.factory = json.dumps({
            'schema': 'mag_acquisition_spawn_native_initial_bindings_v1',
            'status': API['FACTORY_STATUS'],
            'original_elf_sha256': {'retail': elf_sha},
            'native_factory': dict(object_type=35, capacity=128, spawn_stride=224,
                                   property_class='SpawnPointProperties', automatic_creation_id='2380ffff'),
        }).encode()

        def links(index, addresses, slot):
            return [dict(team=q//16, platoon_id=q%16//4, squad=tuple(builder['SQUADS'])[q%4], slot=slot,
                         point_token=hex(index + 100)) for q in addresses]

        locations, points = [], []
        for index in range(112):
            slot = 111 - index
            kind = {1: 'alternate', 2: 'unsupported'}.get(index, 'insertion')
            locations.append(dict(index=index, token=index+100, kind=kind, active=True,
                                  position=dict(x=index+1, y=2, z=3)))
            membership = {
                0: links(0, range(1, 16), 0), 1: links(1, [0], 1),
                2: links(2, [0], 0), 3: links(3, range(16, 32), 0),
            }
            points.append(dict(name_token=hex(index+100), authored_catalog_id=f'{name}:{index:03d}',
                authored_object_id=0xffffffff, initial_client_pool_slot=slot,
                initial_client_object_id=hex(0x23800000 | slot), spawn_type=3 if index == 2 else 0,
                user_class=0, category=1 if index == 1 else 0, active=True, team=int(index >= 3),
                squad_links=membership.get(index, [])))
        squads = []
        for index in range(32):
            default = 1 if index == 0 else 0 if index < 16 else 3
            squads.append(dict(index=index, team=index//16, platoon=index%16//4, squad=index%4,
                               faction=2 if index >= 16 else None, default_spawn=default, available=[default]))
        self.source = dict(map=name, game_archive_sha256=game_sha,
                        level_archive_sha256='a'*64, level_clv_sha256='b'*64,
                        game_rules_source={'sha256': settings_sha},
                        unsupported_spawn_links=[dict(index=2, spawn_type=3, category=0, user_class=0,
                                                      active=True, squad_links=[[0, 0, 0, 0]])],
                        default_spawn_fallbacks=[dict(squad_index=0, default_spawn=1)])
        self.catalog = dict(format='mag_spawn_catalog', schema_version=1, match_profile={'game_type': 2},
                            active=dict(map_name=name, squad_index=0, available=[1], default_spawn=1),
                            maps=[dict(name=name, spawn_count=len(locations), locations=locations,
                                       runtime_indices_verified=False, owner_faction=2, squads=squads)])
        self.selected = dict(map=dict(map=name, mode_enum='GameType256', owner_faction='SVER'),
                            source=dict(archive_sha256='a'*64, clv_sha256='b'*64),
                            points=points, spawn_types={'0': 111, '3': 1}, user_classes={'0': 112})
        self.proof = dict(schema='mag_domination_spawn_native_initial_bindings_v1', status=API['STATUS'],
            native_elf_sha256=elf_sha, native_factory_proof_sha256=API['sha'](self.factory),
            game_archive_sha256=game_sha, settings_sha256=settings_sha, checker_sha256='c'*64,
            topology_checker_sha256='d'*64, spawn_extractor_sha256='e'*64,
            pool=dict(family=35, capacity=128, stride=224, runtime_observed=False),
            maps=[self.selected] + [dict(map=dict(map=other)) for other in sorted(API['MAPS'] - {name})])

    def project(self):
        self.catalog['source'] = json.dumps(self.source)
        return API['project'](json.dumps(self.catalog).encode(), json.dumps(self.proof).encode(), self.factory)

    def test_uses_explicit_native_slots_and_preserves_support_policy(self):
        output = self.project()
        by_slot = output['maps'][0]['locations']
        self.assertEqual(output['active']['default_spawn'], 110)
        self.assertEqual(output['active']['available'], [110])
        self.assertEqual(output['maps'][0]['squads'][31]['default_spawn'], 108)
        self.assertEqual(output['maps'][0]['squads'][31]['available'], [108])
        self.assertEqual(by_slot[109]['token'], 102)
        self.assertEqual(by_slot[109]['kind'], 'unsupported')
        self.assertEqual(by_slot[109]['position'], {'x': 3, 'y': 2, 'z': 3})
        self.assertTrue(output['maps'][0]['runtime_indices_verified'])
        source = json.loads(output['source'])
        self.assertEqual(source['unsupported_spawn_links'][0]['index'], 109)
        self.assertEqual(source['default_spawn_fallbacks'][0]['default_spawn'], 110)
        self.assertEqual(output['match_profile'], self.catalog['match_profile'])

    def test_rejects_unqualified_or_incomplete_family_inventory(self):
        self.selected['points'][0]['initial_client_object_id'] = '0x2280006f'
        with self.assertRaisesRegex(ValueError, 'Spawn identity'):
            self.project()
        self.selected['points'].pop(0)
        with self.assertRaisesRegex(ValueError, 'Incomplete spawn inventory'):
            self.project()

    def test_rejects_duplicate_native_slot_even_if_packed_value_matches(self):
        for name in ('initial_client_pool_slot', 'initial_client_object_id'):
            self.selected['points'][0][name] = self.selected['points'][1][name]
        with self.assertRaisesRegex(ValueError, 'Duplicate or missing native pool slots'):
            self.project()

    def test_refuses_mismatched_source_factory_digest_and_user_class(self):
        original = copy.deepcopy(self.selected)
        self.source['level_clv_sha256'] = 'f' * 64
        with self.assertRaisesRegex(ValueError, 'Original game or level source mismatch'):
            self.project()
        self.source['level_clv_sha256'] = self.selected['source']['clv_sha256']
        self.selected['points'][0]['user_class'] = 1
        with self.assertRaisesRegex(ValueError, 'Spawn identity'):
            self.project()
        self.selected.update(original)
        self.factory += b' '
        with self.assertRaisesRegex(ValueError, 'Unqualified initial spawn proof'):
            self.project()

    def test_refuses_membership_policy_mismatch(self):
        self.selected['points'][0]['squad_links'].pop()
        with self.assertRaisesRegex(ValueError, 'Squad available list differs'):
            self.project()
