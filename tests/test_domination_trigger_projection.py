import json
from pathlib import Path
import runpy
import unittest


API = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tools/extraction'
                         / 'project-retail-domination-triggers.py'))


class DominationTriggerProjectionTest(unittest.TestCase):
    def setUp(self):
        self.name = 'mag_baku_final'
        sha = API['RETAIL_SHA']
        self.objectives = dict(format='mag_retail_objective_catalog', schema_version=1,
            game_type=2, entity_family=12, network_type=12, map_name=self.name,
            source=dict(source_image_sha256=sha, binding_capture_sha256='a'*64,
                        capture_qualification='native_initial_load',
                        initial_load_scope='pre_mission_start_before_dynamic_creation'),
            objects=[dict(token=hex(100+i), object_index=i) for i in range(94)])
        self.proof = dict(schema='mag_domination_initial_trigger_bindings_v1', status=API['STATUS'],
            original_elf_sha256={'retail':sha}, objective_binding_sha256='a'*64, maps=[])
        self.metadata = dict(format='mag_retail_domination_map_metadata', schema_version=1,
                             mode_enum='GameType256', maps=[])
        for name in sorted(API['MAPS']):
            self.metadata['maps'].append(dict(map={'map':name}, source={'clv_sha256':'b'*64},
                                              objective_trigger_volumes=[]))
            self.proof['maps'].append(dict(map={'map':name}, source={'clv_sha256':'b'*64}, triggers=[]))
        selected = self.metadata['maps'][0]
        self.qualified = self.proof['maps'][0]['triggers']
        for i in range(95):
            # First two authored rows deliberately have the opposite native order.
            objective = 0 if i < 2 else i-1
            volume = dict(trigger_token=hex(200+i), objective_token=hex(100+objective))
            native_index = int(i == 0)
            self.qualified.append(dict(trigger_token=volume['trigger_token'],
                objective_token=volume['objective_token'], authored_volume=volume,
                objective_initial_client_network_index=objective, client_trigger_index=native_index,
                sim_trigger_index=native_index, family56_level_object_slot=None))
            if i == 1:
                selected['objective_trigger_volumes'][0]['triggers'].append(volume)
            else:
                selected['objective_trigger_volumes'].append(dict(triggers=[volume]))

    def project(self):
        meta = json.dumps(self.metadata).encode()
        self.proof['authored_metadata_sha256'] = self.objectives['source']['authored_catalog_sha256'] = API['sha'](meta)
        return API['project'](meta, json.dumps(self.proof).encode(), json.dumps(self.objectives).encode())

    def test_native_slots_reorder_authored_volume_rows(self):
        result = self.project()
        self.assertEqual([row['authored_volume']['trigger_token'] for row in result['triggers'][:2]],
                         ['0xc9', '0xc8'])

    def test_rejects_truncated_vectors_and_client_sim_disagreement(self):
        self.qualified[0]['sim_trigger_index'] = 0
        with self.assertRaisesRegex(ValueError, 'Conflicting native trigger vector index'):
            self.project()
        self.qualified[0]['client_trigger_index'] = self.qualified[0]['sim_trigger_index'] = 2
        with self.assertRaisesRegex(ValueError, 'Incomplete per-objective trigger vector'):
            self.project()

    def test_rejects_objective_binding_mismatch(self):
        self.qualified[2]['objective_initial_client_network_index'] = 2
        with self.assertRaisesRegex(ValueError, 'Trigger volume/objective source mismatch'):
            self.project()
