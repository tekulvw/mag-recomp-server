import copy
import json
from pathlib import Path
import runpy
import unittest


MODULE = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tools/extraction'
                            / 'project-retail-domination-initial-objectives.py'))


class DominationInitialProjectionTest(unittest.TestCase):
    def setUp(self):
        self.map_name = 'mag_baku_final'
        self.source = {'archive_sha256': 'a' * 64, 'clv_sha256': 'b' * 64}
        cls = 'ObjectiveSecureAndHoldProperties'
        self.instances = [
            dict(name_token='0x2', definition_token='0xa', source={'offset': '0x102'}),
            dict(name_token='0x3', definition_token='0xa', source={'offset': '0x103'}),
            dict(name_token='0x4', definition_token='0xb', source={'offset': '0x104'}),
        ]
        squads = [dict(team=i // 16, platoon=(i % 16) // 4, squad=i % 4) for i in range(32)]
        self.metadata = dict(format='mag_retail_domination_map_metadata', schema_version=1,
                             mode_enum='GameType256', maps=[])
        elf = {'retail': MODULE['RETAIL_SHA']}
        self.native = dict(original_elf_sha256=elf, status=MODULE['STATUS'])
        self.proof = dict(schema='mag_domination_objective_initial_client_bindings_v1',
                          status=MODULE['STATUS'], original_elf_sha256=elf, checker_sha256='c' * 64,
                          source_model_binding_count=6, maps=[])
        for name in sorted(MODULE['MAPS']):
            map_fields = dict(map=name, mode_enum='GameType256')
            self.metadata['maps'].append(dict(
                map=map_fields, source=self.source, squads=squads,
                objectives=dict(instances=self.instances, definitions=[
                    dict(definition_token='0xa', class_name=cls),
                    dict(definition_token='0xb', class_name='ObjectiveTriggerProperties'),
                ]),
            ))
            objects = []
            for index, row in enumerate(reversed(self.instances[:2])):
                objects.append(dict(
                    name_token=row['name_token'], definition_token=row['definition_token'],
                    class_name=cls, instance_source=row['source'],
                    native_initial_client_binding=dict(
                        entity_family=12, network_type=12, client_class='ClientObjectiveSecureAndHold',
                        client_pool_slot=index, network_index=index,
                        packed_client_entity_id=hex(0x0c800000 | index), initial_network_state=0,
                        source_model_only=True, runtime_observed=False,
                    ),
                ))
            squad_bindings = [dict(
                team=s['team'], platoon_id=s['platoon'], squad=s['squad'], squad_index=index,
                ordered_source_entries=[dict(objective_token='0x2', authored_order=10)],
            ) for index, s in enumerate(squads)]
            self.proof['maps'].append(dict(
                map=map_fields, source=copy.deepcopy(self.source), excluded_trigger_count=1,
                objects=objects, squads=squad_bindings,
                native_initial_client_pool=dict(array=12, capacity=128, stride=1872,
                    initial_load_success_scope=True, runtime_observed=False, expected_occupied_slots=[0, 1]),
            ))
        self.selected = self.proof['maps'][0]

    def project(self):
        metadata = json.dumps(self.metadata).encode()
        native = json.dumps(self.native).encode()
        self.proof.update(catalog_sha256=MODULE['sha'](metadata), native_proof_replay_sha256=MODULE['sha'](native))
        return MODULE['project'](metadata, json.dumps(self.proof).encode(), native, self.map_name)

    def test_uses_qualified_slots_and_inactive_constructor_state(self):
        out = self.project()
        self.assertEqual(out['game_type'], 2)
        self.assertEqual(out['objects'], [
            dict(token='0x00000003', object_index=0, initial_state=0),
            dict(token='0x00000002', object_index=1, initial_state=0),
        ])
        self.assertEqual(len(out['squads']), 32)
        self.assertEqual(out['squads'][-1], dict(team=1, squad_index=31, initial_objective_index=65535,
                         authored_objectives=[dict(object_index=1, order=10)]))

    def test_rejects_incorrect_network_index(self):
        self.selected['objects'][0]['native_initial_client_binding']['network_index'] = 19
        with self.assertRaisesRegex(ValueError, 'Inconsistent native objective binding'):
            self.project()

    def test_rejects_partial_inventory(self):
        self.selected['objects'].pop()
        self.proof['source_model_binding_count'] -= 1
        with self.assertRaisesRegex(ValueError, 'Incomplete family 12 inventory'):
            self.project()

    def test_rejects_duplicate_squad_address(self):
        self.selected['squads'][-1] = self.selected['squads'][-2]
        with self.assertRaisesRegex(ValueError, 'Invalid squad address'):
            self.project()

    def test_rejects_source_mismatch(self):
        self.selected['source']['clv_sha256'] = 'd' * 64
        with self.assertRaisesRegex(ValueError, 'Level source mismatch'):
            self.project()


if __name__ == '__main__':
    unittest.main()
